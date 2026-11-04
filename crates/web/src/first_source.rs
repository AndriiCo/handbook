//! First source: staff asking the office itself (voice calls, email)
//! when the published pages leave a question open.
//!
//! ADMIN LISTENER ONLY (merged in `mission_admin_routes`, behind the
//! host guard, IP allowlist and CSRF double-submit; handlers take
//! [`AdminAuth`]). Three parts:
//!
//! - **Open questions** per authority, ranked from the fact store:
//!   accepted facts the Wonder cross-check contradicted or could not
//!   find, topics with only unconfirmed proposals, core topics with no
//!   fact at all, stale topics, and open staff tasks. Deterministic (no
//!   model): the wording comes from `docs/specs/TOPICS.md`.
//! - **Contacts**: the authority's email/phone/webform channels (the
//!   baseline `channel` table, purpose `our_validation`).
//! - **Email outreach**: Muse Spark drafts the greeting and sign-off,
//!   the server lays the questions out verbatim, staff edit and
//!   approve, the configured mailer sends with our own `Message-ID`
//!   so the office's reply threads back to the draft, and with
//!   `Reply-To` set to the polled mailbox (`mail_poll.email`) so the
//!   reply lands where the mail poll reads it.

use std::collections::HashMap;

use axum::{
    Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::{get, post, put},
};
use handbook_store::{db, store::Store};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{AppState, admin::AdminAuth, store_unavailable, unix_now};

/// A topic's accepted fact is stale after a year without a newer capture.
const STALE_SECS: i64 = 365 * 86_400;

/// Topics an applicant needs before sending anything, highest first.
const CORE_TOPICS: &[&str] = &[
    "fees",
    "mailing",
    "processing_time",
    "payment",
    "forms",
    "legalization_chain",
];

/// Every topic worth asking an office about, in asking order. Internal
/// topics (`rejections`, `website`, `phone_script`, `route_photos`,
/// `parking`) are left out: staff never phone an office about them.
const ASKABLE_TOPICS: &[&str] = &[
    "fees",
    "mailing",
    "po_box",
    "processing_time",
    "payment",
    "forms",
    "legalization_chain",
    "hours",
    "address",
    "walkin",
    "appointment",
    "id_requirements",
    "notarization",
    "authentication",
    "jurisdiction",
    "copies",
    "translation",
    "holiday_hours",
    "dropbox",
    "contacts",
];

/// The question to ask when a topic has nothing confirmed
/// (`docs/specs/TOPICS.md` example questions, phrased for a call).
fn topic_question(topic: &str) -> &'static str {
    match topic {
        "fees" => {
            "What is the fee per document for an apostille or authentication, and is there a separate counter or expedite fee?"
        }
        "mailing" => {
            "What is the full mailing address for mail-in requests, and is a different street address used for FedEx or UPS?"
        }
        "po_box" => {
            "Is there a PO box for mail-in requests, and do couriers need a street address instead?"
        }
        "processing_time" => {
            "How many business days does a mailed request take, and is expedited service offered?"
        }
        "payment" => {
            "Which payment methods do you accept, and exactly whom should a check or money order be payable to?"
        }
        "forms" => {
            "Which request form should applicants use, and where is the current version published?"
        }
        "legalization_chain" => {
            "What steps must a document complete before it reaches your office, for example a county clerk certification?"
        }
        "hours" => "What are the counter hours for apostille requests?",
        "address" => "What is the street address of the office, including floor and suite?",
        "walkin" => "Do you accept walk-in requests, and is there a daily limit or cutoff time?",
        "appointment" => "Is an appointment required for in-person service, and how is it booked?",
        "id_requirements" => "What ID must an applicant show at the counter?",
        "notarization" => {
            "Which documents must be notarized before you will apostille them, and is remote online notarization accepted?"
        }
        "authentication" => {
            "Which documents receive an apostille and which receive an authentication certificate instead?"
        }
        "jurisdiction" => {
            "Which documents does your office handle, and which must go to a county or another office first?"
        }
        "copies" => {
            "Do you accept certified copies, or only originals, and how many copies per request?"
        }
        "translation" => {
            "Must documents in another language be translated, and does the translation need certifying?"
        }
        "holiday_hours" => {
            "Which holidays is the office closed, and are there special year-end hours?"
        }
        "dropbox" => "Is there an after-hours drop box, and when is it emptied?",
        "contacts" => "Which team or direct line handles apostille questions?",
        _ => "What should applicants know about this?",
    }
}

/// Why a question is on the list (also its sort order, strongest first).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// The Wonder cross-check found the source saying otherwise.
    Contradicted,
    /// A staff task is open for this authority.
    OpenTask,
    /// A core topic has no fact at all.
    MissingCore,
    /// Drafted from a page but not yet confirmed by staff.
    Unconfirmed,
    /// The cross-check could not find the accepted fact in its source.
    NotFound,
    /// The newest accepted fact's capture is over a year old.
    Stale,
    /// A non-core topic with no fact.
    Missing,
}

impl Reason {
    fn why(self) -> &'static str {
        match self {
            Reason::Contradicted => {
                "The source page now says something different from what we publish."
            }
            Reason::OpenTask => "A staff task is open for this office.",
            Reason::MissingCore => {
                "Applicants need this before they send anything, and no page we captured states it."
            }
            Reason::Unconfirmed => {
                "Muse Spark drafted this from a page; nobody has confirmed it yet."
            }
            Reason::NotFound => "The cross-check could not find this fact in the page it cites.",
            Reason::Stale => "Our newest source for this is over a year old.",
            Reason::Missing => "No page we captured states this.",
        }
    }

    fn priority(self) -> &'static str {
        match self {
            Reason::Contradicted | Reason::OpenTask | Reason::MissingCore => "high",
            Reason::Unconfirmed | Reason::NotFound => "medium",
            Reason::Stale | Reason::Missing => "low",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Question {
    pub topic: String,
    pub question: String,
    pub reason: Reason,
    pub why: &'static str,
    pub priority: &'static str,
    pub fact_ids: Vec<i64>,
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}

/// "We list: '<statement>'. Is that still correct?" — kept within the
/// voice pack's per-question bound so the list can be dialled as is.
fn confirm_question(statement: &str) -> String {
    let frame = "We list: \"\". Is that still correct?".chars().count();
    let room = voice::scripts::MAX_QUESTION_CHARS.saturating_sub(frame);
    format!(
        "We list: \"{}\". Is that still correct?",
        clip(spoken(statement), room)
    )
}

/// A statement as an office reads it: the drafter's `field_key: value`
/// prefix (`per_document: $15 …`) is internal and dropped.
fn spoken(statement: &str) -> &str {
    let trimmed = statement.trim();
    if let Some((key, rest)) = trimmed.split_once(": ")
        && key.starts_with(|c: char| c.is_ascii_lowercase())
        && key
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !rest.trim().is_empty()
    {
        return rest.trim();
    }
    trimmed
}

fn has_table(conn: &Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
        [name],
        |_| Ok(()),
    )
    .optional()
    .ok()
    .flatten()
    .is_some()
}

/// Resolve an authority slug to `(id, name)`.
fn authority(conn: &Connection, slug: &str) -> rusqlite::Result<Option<(i64, String)>> {
    conn.query_row(
        "SELECT id, name FROM authority WHERE slug = ?1",
        [slug],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
}

/// Ranked open questions for one authority (at most the voice pack's
/// question cap, so a call can take the list unchanged).
pub fn open_questions(
    conn: &Connection,
    authority_id: i64,
    now: i64,
) -> rusqlite::Result<Vec<Question>> {
    // topic -> (accepted ids, proposed (id, statement))
    let mut accepted: HashMap<String, Vec<(i64, String)>> = HashMap::new();
    let mut proposed: HashMap<String, Vec<(i64, String)>> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT id, topic, statement, status FROM fact
         WHERE authority_id = ?1 AND status IN ('accepted', 'proposed')
         ORDER BY id",
    )?;
    let rows = stmt.query_map([authority_id], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
        ))
    })?;
    for row in rows {
        let (id, topic, statement, status) = row?;
        let bucket = if status == "accepted" {
            &mut accepted
        } else {
            &mut proposed
        };
        bucket.entry(topic).or_default().push((id, statement));
    }

    let mut out: Vec<Question> = Vec::new();
    let mut asked: Vec<i64> = Vec::new();
    let mut push = |topic: &str, question: String, reason: Reason, fact_ids: Vec<i64>| {
        out.push(Question {
            topic: topic.to_string(),
            question,
            reason,
            why: reason.why(),
            priority: reason.priority(),
            fact_ids,
        });
    };

    // Cross-check verdicts on accepted facts (latest verdict per fact).
    if has_table(conn, "fact_check") {
        let mut stmt = conn.prepare(
            "SELECT f.id, f.topic, f.statement, fc.verdict FROM fact f
             JOIN fact_check fc ON fc.fact_id = f.id
             WHERE f.authority_id = ?1 AND f.status = 'accepted'
               AND fc.id = (SELECT MAX(id) FROM fact_check WHERE fact_id = f.id)
               AND fc.verdict IN ('contradicted', 'not_found')
             ORDER BY f.id",
        )?;
        let rows = stmt.query_map([authority_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        for row in rows {
            let (id, topic, statement, verdict) = row?;
            let reason = if verdict == "contradicted" {
                Reason::Contradicted
            } else {
                Reason::NotFound
            };
            asked.push(id);
            push(&topic, confirm_question(&statement), reason, vec![id]);
        }
    }

    // Open staff tasks from the system (pack observations, drafting
    // follow-ups, call/email/visit tasks): title only. `correction`
    // tasks carry a customer's own note, so they never become a
    // question that is emailed, dialled or sent to a model.
    let mut stmt = conn.prepare(
        "SELECT title FROM task
         WHERE authority_id = ?1 AND status IN ('open', 'batched', 'in_progress')
           AND kind IN ('validate_fact', 'call', 'email_check', 'visit')
         ORDER BY priority DESC, id",
    )?;
    let rows = stmt.query_map([authority_id], |r| r.get::<_, String>(0))?;
    for row in rows {
        let title = row?;
        let title = title.trim();
        if title.is_empty() || looks_personal(title) {
            continue;
        }
        push(
            "contacts",
            clip(title, voice::scripts::MAX_QUESTION_CHARS),
            Reason::OpenTask,
            Vec::new(),
        );
    }

    // Newest accepted capture per topic, for staleness.
    let mut newest: HashMap<String, i64> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT f.topic, MAX(c.captured_at) FROM fact f
         JOIN capture c ON c.id = f.capture_id
         WHERE f.authority_id = ?1 AND f.status = 'accepted'
         GROUP BY f.topic",
    )?;
    let rows = stmt.query_map([authority_id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })?;
    for row in rows {
        let (topic, at) = row?;
        newest.insert(topic, at);
    }

    for topic in ASKABLE_TOPICS {
        let has_accepted = accepted.get(*topic).is_some_and(|v| !v.is_empty());
        if has_accepted {
            let ids: Vec<i64> = accepted[*topic].iter().map(|(id, _)| *id).collect();
            let already = ids.iter().any(|id| asked.contains(id));
            if !already
                && newest
                    .get(*topic)
                    .is_some_and(|at| now.saturating_sub(*at) > STALE_SECS)
            {
                push(topic, topic_question(topic).to_string(), Reason::Stale, ids);
            }
            continue;
        }
        match proposed.get(*topic) {
            Some(drafts) if !drafts.is_empty() => {
                for (id, statement) in drafts.iter().filter(|(id, _)| !asked.contains(id)) {
                    push(
                        topic,
                        confirm_question(statement),
                        Reason::Unconfirmed,
                        vec![*id],
                    );
                }
            }
            _ => {
                let reason = if CORE_TOPICS.contains(topic) {
                    Reason::MissingCore
                } else {
                    Reason::Missing
                };
                push(topic, topic_question(topic).to_string(), reason, Vec::new());
            }
        }
    }

    // Stable: reason first, then the asking order above.
    out.sort_by_key(|q| q.reason);
    out.truncate(voice::scripts::MAX_QUESTIONS);
    Ok(out)
}

/// One authority contact channel (baseline `channel` table).
#[derive(Debug, Clone, Serialize)]
pub struct Contact {
    pub id: i64,
    pub kind: String,
    pub value: String,
    pub details: String,
    pub visibility: String,
}

pub fn contacts(conn: &Connection, authority_id: i64) -> rusqlite::Result<Vec<Contact>> {
    let mut stmt = conn.prepare(
        "SELECT id, kind, value, details, visibility FROM channel
         WHERE authority_id = ?1 AND kind IN ('email', 'phone', 'webform')
         ORDER BY kind, id",
    )?;
    let rows = stmt.query_map([authority_id], |r| {
        Ok(Contact {
            id: r.get(0)?,
            kind: r.get(1)?,
            value: r.get(2)?,
            details: r.get(3)?,
            visibility: r.get(4)?,
        })
    })?;
    rows.collect()
}

// ------------------------------------------------------- email outreach

/// The drafting model (public text only: the office's name and our
/// questions — never customer data).
const DRAFT_MODEL: &str = "muse-spark-1.3";
/// Outbound outreach emails per Pacific-agnostic rolling day.
const DAILY_SEND_CAP: i64 = 20;
/// Most questions one email asks (an office answers a short list).
const MAX_EMAIL_QUESTIONS: usize = 12;
const MAX_SUBJECT: usize = 140;
const MAX_BODY: usize = 8_000;
const SIGNATURE: &str = "Apostille Handbook research team\nhttps://andrii.ai/apostille";

const DRAFT_PROMPT: &str = "You write the opening and closing of a short email from the research team of Apostille Handbook, a public guide to apostille procedures, to a government office that issues apostilles. The server inserts the numbered questions between your opening and closing, word for word; never restate, answer or add questions.\n\nRules:\n- Opening: greet the office by its name, say in one or two sentences that we keep a public guide to its apostille procedure accurate and would be grateful for a short reply to the questions below.\n- Closing: thank them, say a reply to this email is enough, no attachments needed.\n- Subject: under 90 characters, names the office and says it is a question about apostille procedures.\n- Plain text. Polite, brief, neutral. No digits, no URLs, no email addresses, no promises, no claims about the office, no customer or personal details, no signature (the server adds it).";

pub fn migrate_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS outreach_email (
            id INTEGER PRIMARY KEY,
            authority_slug TEXT NOT NULL,
            to_addr TEXT NOT NULL,
            subject TEXT NOT NULL,
            body TEXT NOT NULL,
            questions_json TEXT NOT NULL,
            status TEXT NOT NULL,
            model TEXT NOT NULL,
            note TEXT NOT NULL DEFAULT '',
            run_id INTEGER,
            message_id TEXT NOT NULL DEFAULT '',
            error TEXT NOT NULL DEFAULT '',
            drafted_by TEXT NOT NULL,
            drafted_at INTEGER NOT NULL,
            sent_by TEXT NOT NULL DEFAULT '',
            sent_at INTEGER,
            reply_capture_id INTEGER
        );
        CREATE INDEX IF NOT EXISTS outreach_email_slug ON outreach_email(authority_slug, drafted_at);",
    )
}

/// One mailbox, `local@domain.tld`, no display name, no spaces.
pub fn valid_email(addr: &str) -> bool {
    let addr = addr.trim();
    let Some((local, domain)) = addr.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && local.len() <= 64
        && domain.len() <= 253
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && addr
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "@._%+-".contains(c))
        && addr.matches('@').count() == 1
}

/// Model-written opening/closing gate: bounded, and nothing a reader
/// could act on (digits, links, addresses) the server did not write.
fn prose_ok(text: &str, max: usize) -> bool {
    let t = text.trim();
    !t.is_empty()
        && t.chars().count() <= max
        && !t.chars().any(|c| c.is_ascii_digit())
        && !t.contains("http")
        && !t.contains("www.")
        && !t.contains('@')
        && !t.contains('<')
}

fn template_parts(name: &str) -> (String, String, String) {
    (
        format!("Question about apostille procedures at {name}"),
        format!(
            "Hello {name} team,\n\nWe keep a public guide to your office's apostille procedure and want it to stay accurate. Could you help with a short reply to the questions below?"
        ),
        "Thank you for your time. A reply to this email is all we need; no attachments are necessary.".to_string(),
    )
}

/// Opening + numbered questions (verbatim) + closing + signature.
fn compose_body(greeting: &str, questions: &[String], closing: &str) -> String {
    let list: Vec<String> = questions
        .iter()
        .enumerate()
        .map(|(i, q)| format!("{}. {}", i + 1, q.trim()))
        .collect();
    format!(
        "{}\n\n{}\n\n{}\n\n{SIGNATURE}",
        greeting.trim(),
        list.join("\n"),
        closing.trim()
    )
}

struct Drafted {
    subject: String,
    body: String,
    model: String,
    note: String,
}

/// Muse Spark writes subject, opening and closing; any failure (no
/// key, cap, refusal, a gate miss) falls back to the fixed template,
/// and the note says why.
fn draft_blocking(
    root: &std::path::Path,
    kek: Option<&config::Kek>,
    name: &str,
    questions: &[String],
    run_id: u64,
) -> Drafted {
    let fallback = |note: String| {
        let (subject, greeting, closing) = template_parts(name);
        Drafted {
            subject,
            body: compose_body(&greeting, questions, &closing),
            model: "template".to_string(),
            note,
        }
    };
    let Ok(conn) = db::open(&Store::new(root)) else {
        return fallback("store unavailable; used the fixed template".to_string());
    };
    let client = match crate::draft::live_client(&conn, kek) {
        Ok(client) => client.with_observer(
            crate::telemetry::DbObserver::new(root.to_path_buf()),
            "outreach",
            Some(run_id),
        ),
        Err(e) => return fallback(format!("{e}; used the fixed template")),
    };
    drop(conn);
    let user = format!(
        "Office: {name}\nQuestions the server will insert ({}):\n{}",
        questions.len(),
        questions
            .iter()
            .map(|q| format!("- {q}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let resp = client.chat_json(&handbook_agent::model::ChatJsonRequest {
        model: DRAFT_MODEL,
        developer: DRAFT_PROMPT,
        user,
        schema_name: "OutreachEmailV1",
        schema: json!({"type": "object", "additionalProperties": false,
            "required": ["subject", "greeting", "closing"],
            "properties": {"subject": {"type": "string"}, "greeting": {"type": "string"},
                           "closing": {"type": "string"}}}),
        reasoning_effort: "low",
        prompt_cache_key: "outreach-email-v1".to_string(),
        max_completion_tokens: 2_048,
        pipeline: "outreach",
        run_id: Some(run_id),
    });
    let value = match resp {
        Ok(r) => r.value,
        Err(e) => {
            return fallback(format!(
                "Muse Spark draft failed ({e}); used the fixed template"
            ));
        }
    };
    let (subject, greeting, closing) = (
        value["subject"].as_str().unwrap_or("").trim().to_string(),
        value["greeting"].as_str().unwrap_or("").trim().to_string(),
        value["closing"].as_str().unwrap_or("").trim().to_string(),
    );
    if !prose_ok(&subject, 90) || !prose_ok(&greeting, 600) || !prose_ok(&closing, 400) {
        return fallback(
            "Muse Spark's draft broke a rule (digits, links, addresses or length); used the fixed template"
                .to_string(),
        );
    }
    Drafted {
        subject,
        body: compose_body(&greeting, questions, &closing),
        model: DRAFT_MODEL.to_string(),
        note: String::new(),
    }
}

#[derive(Debug, Serialize)]
pub struct OutreachEmail {
    pub id: i64,
    pub slug: String,
    pub to: String,
    pub subject: String,
    pub body: String,
    pub questions: Vec<String>,
    pub status: String,
    pub model: String,
    pub note: String,
    pub error: String,
    pub drafted_at: i64,
    pub sent_at: Option<i64>,
    pub reply_capture_id: Option<i64>,
}

const EMAIL_COLUMNS: &str = "id, authority_slug, to_addr, subject, body, questions_json, status, model, note, error, drafted_at, sent_at, reply_capture_id";

fn email_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<OutreachEmail> {
    let questions: String = r.get(5)?;
    Ok(OutreachEmail {
        id: r.get(0)?,
        slug: r.get(1)?,
        to: r.get(2)?,
        subject: r.get(3)?,
        body: r.get(4)?,
        questions: serde_json::from_str(&questions).unwrap_or_default(),
        status: r.get(6)?,
        model: r.get(7)?,
        note: r.get(8)?,
        error: r.get(9)?,
        drafted_at: r.get(10)?,
        sent_at: r.get(11)?,
        reply_capture_id: r.get(12)?,
    })
}

fn load_email(conn: &Connection, id: i64) -> rusqlite::Result<Option<OutreachEmail>> {
    conn.query_row(
        &format!("SELECT {EMAIL_COLUMNS} FROM outreach_email WHERE id = ?1"),
        [id],
        email_row,
    )
    .optional()
}

fn bad_request(msg: &str) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({"error": msg})),
    )
        .into_response()
}

fn conflict(msg: &str) -> Response {
    (StatusCode::CONFLICT, Json(json!({"error": msg}))).into_response()
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DraftBody {
    pub slug: String,
    pub to: String,
    pub questions: Vec<String>,
}

fn clean_questions(raw: &[String]) -> Result<Vec<String>, &'static str> {
    let qs: Vec<String> = raw
        .iter()
        .map(|q| q.trim().to_string())
        .filter(|q| !q.is_empty())
        .collect();
    if qs.is_empty() {
        return Err("pick at least one question");
    }
    if qs.len() > MAX_EMAIL_QUESTIONS {
        return Err("an email asks at most 12 questions");
    }
    if qs
        .iter()
        .any(|q| q.chars().count() > voice::scripts::MAX_QUESTION_CHARS)
    {
        return Err("a question is over 280 characters");
    }
    // Outreach asks about public procedure only: nothing that looks
    // like a customer's email, order id, or phone/case number leaves.
    if qs.iter().any(|q| looks_personal(q)) {
        return Err("questions must not carry customer details (emails, order ids, long numbers)");
    }
    Ok(qs)
}

/// Customer-shaped data: an email address, an order id, or a long
/// unbroken digit run (account, card, tracking or case numbers).
/// Separated groups (a ZIP+4, an office phone, a date) are public
/// procedure text and pass.
fn looks_personal(text: &str) -> bool {
    let mut run = 0usize;
    for c in text.chars() {
        if c.is_ascii_digit() {
            run += 1;
            if run >= 9 {
                return true;
            }
        } else {
            run = 0;
        }
    }
    text.contains('@') || text.contains("ord_")
}

async fn draft_email(
    auth: AdminAuth,
    State(state): State<AppState>,
    Json(body): Json<DraftBody>,
) -> Response {
    let to = body.to.trim().to_ascii_lowercase();
    if !valid_email(&to) {
        return bad_request("recipient must be one email address");
    }
    let questions = match clean_questions(&body.questions) {
        Ok(q) => q,
        Err(e) => return bad_request(e),
    };
    let Ok(conn) = db::open(&Store::new(&state.store_root)) else {
        return store_unavailable();
    };
    let Ok(found) = authority(&conn, &body.slug) else {
        return store_unavailable();
    };
    let Some((_, name)) = found else {
        return not_found(&body.slug);
    };
    let capped = crate::telemetry::over_daily_cap(&conn, &state, unix_now());
    drop(conn);
    let run_id = state.draft_runs.reserve_id();
    let (root, kek) = (state.store_root.clone(), state.kek.clone());
    let qs = questions.clone();
    let drafted = if capped {
        let (subject, greeting, closing) = template_parts(&name);
        Drafted {
            subject,
            body: compose_body(&greeting, &qs, &closing),
            model: "template".to_string(),
            note: "daily model-call cap reached (meta.daily_call_cap); used the fixed template"
                .to_string(),
        }
    } else {
        let name = name.clone();
        match tokio::task::spawn_blocking(move || {
            draft_blocking(&root, kek.as_deref(), &name, &qs, run_id)
        })
        .await
        {
            Ok(d) => d,
            Err(_) => return store_unavailable(),
        }
    };
    let Ok(conn) = db::open(&Store::new(&state.store_root)) else {
        return store_unavailable();
    };
    let now = unix_now();
    let inserted = conn.execute(
        "INSERT INTO outreach_email(authority_slug, to_addr, subject, body, questions_json, status,
                                    model, note, run_id, drafted_by, drafted_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'draft', ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            body.slug,
            to,
            drafted.subject,
            drafted.body,
            serde_json::to_string(&questions).unwrap_or_else(|_| "[]".to_string()),
            drafted.model,
            drafted.note,
            run_id as i64,
            auth.user.user_id,
            now,
        ],
    );
    if inserted.is_err() {
        return store_unavailable();
    }
    let id = conn.last_insert_rowid();
    match load_email(&conn, id) {
        Ok(Some(email)) => (StatusCode::CREATED, Json(json!({"email": email}))).into_response(),
        _ => store_unavailable(),
    }
}

#[derive(Debug, Deserialize)]
pub struct EmailListQuery {
    pub slug: Option<String>,
}

async fn list_emails(
    _auth: AdminAuth,
    State(state): State<AppState>,
    Query(q): Query<EmailListQuery>,
) -> Response {
    let Ok(conn) = db::open(&Store::new(&state.store_root)) else {
        return store_unavailable();
    };
    let sql = format!(
        "SELECT {EMAIL_COLUMNS} FROM outreach_email
         WHERE (?1 IS NULL OR authority_slug = ?1)
         ORDER BY id DESC LIMIT 50"
    );
    let rows = conn.prepare(&sql).and_then(|mut stmt| {
        stmt.query_map([q.slug.as_deref()], email_row)?
            .collect::<rusqlite::Result<Vec<_>>>()
    });
    match rows {
        Ok(emails) => Json(json!({"emails": emails, "daily_cap": DAILY_SEND_CAP})).into_response(),
        Err(_) => store_unavailable(),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditBody {
    pub to: String,
    pub subject: String,
    pub body: String,
}

async fn edit_email(
    _auth: AdminAuth,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(edit): Json<EditBody>,
) -> Response {
    let to = edit.to.trim().to_ascii_lowercase();
    if !valid_email(&to) {
        return bad_request("recipient must be one email address");
    }
    let subject = edit.subject.trim();
    if subject.is_empty() || subject.chars().count() > MAX_SUBJECT || subject.contains(['\r', '\n'])
    {
        return bad_request("subject must be one line of at most 140 characters");
    }
    if edit.body.trim().is_empty() || edit.body.chars().count() > MAX_BODY {
        return bad_request("body must be 1 to 8000 characters");
    }
    let Ok(conn) = db::open(&Store::new(&state.store_root)) else {
        return store_unavailable();
    };
    match conn.execute(
        "UPDATE outreach_email SET to_addr = ?1, subject = ?2, body = ?3
         WHERE id = ?4 AND status = 'draft'",
        rusqlite::params![to, subject, edit.body.trim(), id],
    ) {
        Ok(1) => match load_email(&conn, id) {
            Ok(Some(email)) => Json(json!({"email": email})).into_response(),
            _ => store_unavailable(),
        },
        Ok(_) => conflict("only a draft can be edited"),
        Err(_) => store_unavailable(),
    }
}

async fn discard_email(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Response {
    let Ok(conn) = db::open(&Store::new(&state.store_root)) else {
        return store_unavailable();
    };
    match conn.execute(
        "UPDATE outreach_email SET status = 'discarded' WHERE id = ?1 AND status IN ('draft', 'failed')",
        [id],
    ) {
        Ok(1) => {
            let _ = conn.execute(
                "INSERT INTO audit(at, actor, action, entity, entity_id, reason)
                 VALUES (?1, ?2, 'outreach_email_discard', 'outreach_email', ?3, '')",
                rusqlite::params![unix_now(), auth.user.user_id, id],
            );
            match load_email(&conn, id) {
                Ok(Some(email)) => Json(json!({"email": email})).into_response(),
                _ => store_unavailable(),
            }
        }
        Ok(_) => conflict("only a draft or a failed send can be discarded"),
        Err(_) => store_unavailable(),
    }
}

/// Headers for one outreach send: our `Message-ID`, plus `Reply-To`
/// naming the mailbox the Gmail poll reads (`mail_poll.email`, seeded
/// from the Gmail profile) when one is set up. Without it the office
/// replies to `mailer.from`, which nothing may read.
fn outreach_headers(conn: &Connection, message_id: &str) -> Vec<(&'static str, String)> {
    let mut headers = vec![("Message-ID", message_id.to_string())];
    let polled: Option<String> = conn
        .query_row("SELECT email FROM mail_poll WHERE id = 1", [], |r| r.get(0))
        .optional()
        .ok()
        .flatten();
    if let Some(mailbox) = polled.map(|m| m.trim().to_ascii_lowercase())
        && valid_email(&mailbox)
    {
        headers.push(("Reply-To", mailbox));
    }
    headers
}

/// Human approval IS the send: staff read the final text and press
/// Send. Capped per day; the office's reply threads back on our
/// `Message-ID`.
async fn send_email(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Response {
    let Some(mailer) = state.mailer.clone() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "no mailer configured (mailer.provider in /config)"})),
        )
            .into_response();
    };
    let Ok(conn) = db::open(&Store::new(&state.store_root)) else {
        return store_unavailable();
    };
    let now = unix_now();
    let sent_today: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM outreach_email
             WHERE (status IN ('sent', 'replied') AND sent_at >= ?1) OR status = 'sending'",
            [now - 86_400],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if sent_today >= DAILY_SEND_CAP {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "daily outreach cap reached (20 emails in 24 hours)"})),
        )
            .into_response();
    }
    let message_id = format!(
        "<outreach-{id}-{}@andrii.ai>",
        &auth::csrf::mint_token()[..16]
    );
    // Claim the draft before the network call, so a double click
    // cannot send twice.
    match conn.execute(
        "UPDATE outreach_email SET status = 'sending', message_id = ?1, sent_by = ?2
         WHERE id = ?3 AND status = 'draft'",
        rusqlite::params![message_id, auth.user.user_id, id],
    ) {
        Ok(1) => {}
        Ok(_) => return conflict("only a draft can be sent"),
        Err(_) => return store_unavailable(),
    }
    let Ok(Some(email)) = load_email(&conn, id) else {
        return store_unavailable();
    };
    let headers = outreach_headers(&conn, &message_id);
    drop(conn);
    // The send and its bookkeeping run in a detached task: if the
    // request is cut off (the admin request budget), the email still
    // ends sent or failed with its audit row, never stuck in 'sending'.
    let root = state.store_root.clone();
    let actor = auth.user.user_id.clone();
    let finish = tokio::spawn(async move {
        let (to, subject, body) = (email.to.clone(), email.subject.clone(), email.body.clone());
        let sent = tokio::task::spawn_blocking(move || {
            let headers: Vec<(&str, &str)> =
                headers.iter().map(|(n, v)| (*n, v.as_str())).collect();
            mailer.send_with_headers(&to, &subject, &body, &headers)
        })
        .await;
        let (status, error) = match sent {
            Ok(Ok(())) => ("sent", String::new()),
            Ok(Err(e)) => ("failed", format!("{e:?}")),
            Err(e) => ("failed", format!("send task failed: {e}")),
        };
        let Ok(conn) = db::open(&Store::new(&root)) else {
            return status;
        };
        let _ = conn.execute(
            "UPDATE outreach_email SET status = ?1, error = ?2, sent_at = ?3 WHERE id = ?4",
            rusqlite::params![status, error, (status == "sent").then_some(now), id],
        );
        let _ = conn.execute(
            "INSERT INTO audit(at, actor, action, entity, entity_id, reason)
             VALUES (?1, ?2, ?3, 'outreach_email', ?4, ?5)",
            rusqlite::params![
                now,
                actor,
                if status == "sent" {
                    "outreach_email_sent"
                } else {
                    "outreach_email_failed"
                },
                id,
                email.slug,
            ],
        );
        if status == "sent" {
            remember_contact(&conn, &email.slug, "email", &email.to, "");
        }
        status
    });
    let status = finish.await.unwrap_or("failed");
    let Ok(conn) = db::open(&Store::new(&state.store_root)) else {
        return store_unavailable();
    };
    match load_email(&conn, id) {
        Ok(Some(email)) if status == "sent" => Json(json!({"email": email})).into_response(),
        Ok(Some(email)) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": email.error, "email": email})),
        )
            .into_response(),
        _ => store_unavailable(),
    }
}

// ------------------------------------------------------------ contacts

/// E.164 from what staff typed: a leading `+` (or `00`) keeps the
/// country code; ten bare digits (or eleven starting with 1) are NANP.
pub fn normalize_phone(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let digits: String = trimmed.chars().filter(|c| c.is_ascii_digit()).collect();
    let international = trimmed.starts_with('+') || trimmed.starts_with("00");
    let digits = if trimmed.starts_with("00") {
        digits.get(2..)?.to_string()
    } else {
        digits
    };
    let e164 = if international {
        digits
    } else if digits.len() == 10 {
        format!("1{digits}")
    } else if digits.len() == 11 && digits.starts_with('1') {
        digits
    } else {
        return None;
    };
    (8..=15).contains(&e164.len()).then(|| format!("+{e164}"))
}

/// Insert a `channel` row unless the same kind+value is on file.
fn remember_contact(
    conn: &Connection,
    slug: &str,
    kind: &str,
    value: &str,
    details: &str,
) -> Option<(i64, bool)> {
    let (authority_id, _) = authority(conn, slug).ok()??;
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM channel WHERE authority_id = ?1 AND kind = ?2 AND value = ?3",
            rusqlite::params![authority_id, kind, value],
            |r| r.get(0),
        )
        .optional()
        .ok()?;
    if let Some(id) = existing {
        return Some((id, false));
    }
    conn.execute(
        "INSERT INTO channel(authority_id, visibility, kind, value, details, purpose)
         VALUES (?1, 'private', ?2, ?3, ?4, 'our_validation')",
        rusqlite::params![authority_id, kind, value, details],
    )
    .ok()?;
    Some((conn.last_insert_rowid(), true))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContactBody {
    pub kind: String,
    pub value: String,
    #[serde(default)]
    pub details: String,
}

async fn add_contact(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Json(body): Json<ContactBody>,
) -> Response {
    let value = match body.kind.as_str() {
        "email" => {
            let v = body.value.trim().to_ascii_lowercase();
            if !valid_email(&v) {
                return bad_request("not an email address");
            }
            v
        }
        "phone" => match normalize_phone(&body.value) {
            Some(v) => v,
            None => {
                return bad_request(
                    "not a phone number (use +<country code> for numbers outside the US and Canada)",
                );
            }
        },
        _ => return bad_request("kind must be email or phone"),
    };
    let details: String = body.details.trim().chars().take(200).collect();
    let Ok(conn) = db::open(&Store::new(&state.store_root)) else {
        return store_unavailable();
    };
    let Ok(found) = authority(&conn, &slug) else {
        return store_unavailable();
    };
    if found.is_none() {
        return not_found(&slug);
    }
    let Some((id, created)) = remember_contact(&conn, &slug, &body.kind, &value, &details) else {
        return store_unavailable();
    };
    if created {
        let _ = conn.execute(
            "INSERT INTO audit(at, actor, action, entity, entity_id, reason)
             VALUES (?1, ?2, 'channel_add', 'channel', ?3, ?4)",
            rusqlite::params![
                unix_now(),
                auth.user.user_id,
                id,
                format!("{slug} {}", body.kind)
            ],
        );
    }
    let found = found.map(|(aid, _)| aid).unwrap_or_default();
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    match contacts(&conn, found) {
        Ok(list) => (
            status,
            Json(json!({"id": id, "created": created, "contacts": list})),
        )
            .into_response(),
        Err(_) => store_unavailable(),
    }
}

async fn list_contacts(
    _auth: AdminAuth,
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Response {
    let Ok(conn) = db::open(&Store::new(&state.store_root)) else {
        return store_unavailable();
    };
    let Ok(found) = authority(&conn, &slug) else {
        return store_unavailable();
    };
    let Some((id, _)) = found else {
        return not_found(&slug);
    };
    match contacts(&conn, id) {
        Ok(list) => Json(json!({"slug": slug, "contacts": list})).into_response(),
        Err(_) => store_unavailable(),
    }
}

// ------------------------------------------------------------- staff

fn not_found(slug: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({"error": format!("unknown authority slug {slug}")})),
    )
        .into_response()
}

async fn questions(
    _auth: AdminAuth,
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Response {
    let Ok(conn) = db::open(&Store::new(&state.store_root)) else {
        return store_unavailable();
    };
    let Ok(found) = authority(&conn, &slug) else {
        return store_unavailable();
    };
    let Some((id, name)) = found else {
        return not_found(&slug);
    };
    match (open_questions(&conn, id, unix_now()), contacts(&conn, id)) {
        (Ok(questions), Ok(contacts)) => Json(json!({
            "slug": slug,
            "name": name,
            "questions": questions,
            "contacts": contacts,
        }))
        .into_response(),
        _ => store_unavailable(),
    }
}

/// Staff authority index for pickers: the public index's fields plus
/// what staff triage by (proposed facts, contact channels on file).
/// The public `/authorities` route is not on the admin host.
async fn staff_authorities(_auth: AdminAuth, State(state): State<AppState>) -> Response {
    let Ok(conn) = db::open(&Store::new(&state.store_root)) else {
        return store_unavailable();
    };
    let rows = conn
        .prepare(
            "SELECT a.slug, a.name, a.country, a.region, a.kind,
                    (SELECT COUNT(*) FROM fact f WHERE f.authority_id = a.id AND f.status = 'accepted'),
                    (SELECT COUNT(*) FROM fact f WHERE f.authority_id = a.id AND f.status = 'proposed'),
                    (SELECT COUNT(*) FROM channel c WHERE c.authority_id = a.id
                       AND c.kind IN ('email', 'phone', 'webform'))
             FROM authority a WHERE a.id != 0 ORDER BY a.name",
        )
        .and_then(|mut stmt| {
            stmt.query_map([], |r| {
                Ok(json!({
                    "slug": r.get::<_, String>(0)?,
                    "name": r.get::<_, String>(1)?,
                    "country": r.get::<_, String>(2)?,
                    "region": r.get::<_, String>(3)?,
                    "kind": r.get::<_, String>(4)?,
                    "fact_count": r.get::<_, i64>(5)?,
                    "proposed_count": r.get::<_, i64>(6)?,
                    "contact_count": r.get::<_, i64>(7)?,
                }))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
        });
    match rows {
        Ok(authorities) => Json(json!({"authorities": authorities})).into_response(),
        Err(_) => store_unavailable(),
    }
}

/// Staff routes merged onto the ADMIN listener only.
pub fn admin_router(state: AppState) -> Router {
    Router::<AppState>::new()
        .route(
            "/apostille/api/v1/admin/authorities",
            get(staff_authorities),
        )
        .route(
            "/apostille/api/v1/admin/first-source/{slug}/questions",
            get(questions),
        )
        .route(
            "/apostille/api/v1/admin/first-source/{slug}/contacts",
            get(list_contacts).post(add_contact),
        )
        .route(
            "/apostille/api/v1/admin/first-source/emails",
            get(list_emails).post(draft_email),
        )
        .route(
            "/apostille/api/v1/admin/first-source/emails/{id}",
            put(edit_email),
        )
        .route(
            "/apostille/api/v1/admin/first-source/emails/{id}/send",
            post(send_email),
        )
        .route(
            "/apostille/api/v1/admin/first-source/emails/{id}/discard",
            post(discard_email),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Connection, i64, i64) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.init().unwrap();
        let conn = db::open(&store).unwrap();
        db::migrate(&conn).unwrap();
        crate::factcheck::migrate_tables(&conn).unwrap();
        let authority = db::add_authority(
            &conn,
            &db::NewAuthority {
                kind: "state",
                country: "USA",
                region: "Washington",
                name: "Synthetic Secretary of State",
                slug: "synthetic-sos",
                actor: "test",
                at: 1_800_000_000,
            },
        )
        .unwrap();
        conn.execute(
            "INSERT INTO capture(authority_id, captured_at, kind, path, bytes_raw, bytes_stored, sha256, trust)
             VALUES (?1, 1700000000, 'file', 'x', 1, 1, 'aa', 'official')",
            [authority],
        )
        .unwrap();
        let capture = conn.last_insert_rowid();
        (dir, conn, authority, capture)
    }

    fn fact(conn: &Connection, a: i64, c: i64, topic: &str, statement: &str, status: &str) -> i64 {
        conn.execute(
            "INSERT INTO fact(authority_id, capture_id, topic, statement, status, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 1700000000)",
            rusqlite::params![a, c, topic, statement, status],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn questions_rank_contradicted_then_missing_core_then_unconfirmed() {
        let (_dir, conn, a, c) = store();
        let fees = fact(
            &conn,
            a,
            c,
            "fees",
            "Apostilles cost $15 per document.",
            "accepted",
        );
        conn.execute(
            "INSERT INTO fact_check(fact_id, authority_slug, capture_id, verdict, quote, reason, model, at)
             VALUES (?1, 'synthetic-sos', ?2, 'contradicted', 'Apostilles cost $20.', '', 'muse-wonder-1.0', 1)",
            rusqlite::params![fees, c],
        )
        .unwrap();
        let hours = fact(&conn, a, c, "hours", "Open 8:00 to 4:30.", "proposed");
        // Captured in 2023, "now" is 2027: the fees topic is also stale,
        // but the contradicted fact is asked once, as a contradiction.
        let now = 1_800_000_000;
        let qs = open_questions(&conn, a, now).unwrap();

        assert_eq!(qs[0].reason, Reason::Contradicted);
        assert_eq!(qs[0].fact_ids, vec![fees]);
        assert!(
            qs[0].question.contains("$15 per document"),
            "{}",
            qs[0].question
        );
        // Core topics with nothing at all follow, in asking order.
        let core: Vec<&str> = qs
            .iter()
            .filter(|q| q.reason == Reason::MissingCore)
            .map(|q| q.topic.as_str())
            .collect();
        assert_eq!(
            core,
            [
                "mailing",
                "processing_time",
                "payment",
                "forms",
                "legalization_chain"
            ]
        );
        let unconfirmed = qs.iter().find(|q| q.reason == Reason::Unconfirmed).unwrap();
        assert_eq!(unconfirmed.fact_ids, vec![hours]);
        assert!(qs.len() <= voice::scripts::MAX_QUESTIONS);
        assert!(
            qs.iter()
                .all(|q| q.question.chars().count() <= voice::scripts::MAX_QUESTION_CHARS)
        );
        // Ordered strongest reason first.
        assert!(qs.windows(2).all(|w| w[0].reason <= w[1].reason));
        assert!(
            !qs.iter()
                .any(|q| q.topic == "fees" && q.reason == Reason::Stale)
        );
    }

    #[test]
    fn phones_normalize_to_e164_and_emails_validate() {
        assert_eq!(
            normalize_phone("(518) 474-4429").as_deref(),
            Some("+15184744429")
        );
        assert_eq!(
            normalize_phone("1-518-474-4429").as_deref(),
            Some("+15184744429")
        );
        assert_eq!(
            normalize_phone("+84 24 3799 3125").as_deref(),
            Some("+842437993125")
        );
        assert_eq!(
            normalize_phone("0084 24 3799 3125").as_deref(),
            Some("+842437993125")
        );
        assert_eq!(normalize_phone("474-4429"), None);
        assert_eq!(normalize_phone("+1234"), None);
        assert!(valid_email("apostille@sos.example.gov"));
        assert!(!valid_email("Office <apostille@sos.example.gov>"));
        assert!(!valid_email("a@b"));
        assert!(!valid_email("a@@b.gov"));
        assert!(!valid_email("a b@c.gov"));
    }

    #[test]
    fn model_prose_never_carries_numbers_links_or_addresses() {
        assert!(prose_ok("Hello Synthetic Office team,", 600));
        assert!(!prose_ok("Call us at 555", 600));
        assert!(!prose_ok("See https://x.example", 600));
        assert!(!prose_ok("Write to a@b.gov", 600));
        assert!(!prose_ok("", 600));
        assert!(!prose_ok(&"a".repeat(601), 600));
        let body = compose_body(
            "Hello team,",
            &["What is the fee?".to_string(), "Which form?".to_string()],
            "Thanks.",
        );
        assert!(
            body.contains("1. What is the fee?\n2. Which form?"),
            "{body}"
        );
        assert!(body.ends_with(SIGNATURE));
        assert!(clean_questions(&[" ".to_string()]).is_err());
        assert!(clean_questions(&vec!["q".to_string(); 13]).is_err());
        for personal in [
            "Did you receive ord_k2Lp9vW3sQ?",
            "Is jane@example.test on file?",
            "Status of case 5550100123?",
        ] {
            assert!(
                clean_questions(&[personal.to_string()]).is_err(),
                "{personal}"
            );
        }
        for public in [
            "Is the fee $20 per document in 2026?",
            "We list: \"PO Box 40234, Olympia, WA 98504-0234\". Is that still correct?",
            "Is (518) 474-4429 the status line?",
        ] {
            assert!(clean_questions(&[public.to_string()]).is_ok(), "{public}");
        }
    }

    #[test]
    fn outreach_replies_go_to_the_polled_mailbox() {
        let (dir, conn, _, _) = store();
        let mid = "<outreach-7-ab@andrii.ai>";
        // No mail poll set up: our Message-ID only.
        assert_eq!(
            outreach_headers(&conn, mid),
            [("Message-ID", mid.to_string())]
        );
        // Seeded but not yet profiled (empty email): still no Reply-To.
        conn.execute("INSERT INTO mail_poll(id) VALUES (1)", [])
            .unwrap();
        assert_eq!(outreach_headers(&conn, mid).len(), 1);
        conn.execute(
            "UPDATE mail_poll SET email = 'Notices@Example.com' WHERE id = 1",
            [],
        )
        .unwrap();
        let headers = outreach_headers(&conn, mid);
        assert_eq!(
            headers,
            [
                ("Message-ID", mid.to_string()),
                ("Reply-To", "notices@example.com".to_string())
            ]
        );
        // The outbox file records the header as sent.
        let mailer = crate::magic::FileMailer {
            dir: dir.path().to_path_buf(),
        };
        let refs: Vec<(&str, &str)> = headers.iter().map(|(n, v)| (*n, v.as_str())).collect();
        mailer
            .send_with_headers(
                "apostille@sos.example.gov",
                "Two questions",
                "Hello.",
                &refs,
            )
            .unwrap();
        let sent = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .find(|e| e.path().extension().is_some_and(|x| x == "txt"))
            .map(|e| std::fs::read_to_string(e.path()).unwrap())
            .unwrap();
        assert!(sent.contains("Reply-To: notices@example.com\n"), "{sent}");
        assert!(sent.contains(&format!("Message-ID: {mid}\n")), "{sent}");
    }

    #[test]
    fn contacts_dedupe_into_the_channel_table() {
        let (_dir, conn, a, _) = store();
        let first = remember_contact(&conn, "synthetic-sos", "email", "x@sos.example.gov", "");
        let again = remember_contact(&conn, "synthetic-sos", "email", "x@sos.example.gov", "");
        assert!(first.is_some_and(|(_, created)| created));
        assert_eq!(again, first.map(|(id, _)| (id, false)));
        remember_contact(&conn, "synthetic-sos", "phone", "+15550100", "front desk");
        let list = contacts(&conn, a).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].kind, "email");
        assert_eq!(list[1].details, "front desk");
        assert_eq!(
            remember_contact(&conn, "no-such", "email", "x@y.gov", ""),
            None
        );
    }

    #[test]
    fn confirm_questions_drop_the_field_key() {
        assert_eq!(
            confirm_question("per_document: $15 per document."),
            "We list: \"$15 per document.\". Is that still correct?"
        );
        assert_eq!(spoken("Fees: $15"), "Fees: $15");
        assert_eq!(spoken("po_box: "), "po_box:");
    }

    #[test]
    fn confirm_questions_stay_within_the_call_bound() {
        let long = "x".repeat(1_000);
        let q = confirm_question(&long);
        assert!(q.chars().count() <= voice::scripts::MAX_QUESTION_CHARS);
        assert!(q.ends_with("Is that still correct?"));
    }
}
