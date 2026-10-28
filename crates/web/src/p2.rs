//! P2 customer forms flow (U1–U5 of
//! `docs/specs/p2-forms-labels.md`): template GET with prefill from
//! accepted facts, guided-fill submit with server validation, PDF
//! render, replica/stale/conflict states, and the correction loop.
//!
//! Mounted at `/apostille/api/v1/p2`. Templates live in
//! `handbook.sqlite` (`form_template`, non-PII); submissions live in
//! `p2.sqlite`. Wet-ink signatures only: the PDF carries an empty
//! signature + date line (P2-S2).

use axum::{
    Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Json},
    routing::{get, patch, post},
};
use forms::{
    FieldError, FormRenderer, FormTemplate, PdfRenderer, ReplicaAcceptance, SignatureMode,
    Submission as RenderSubmission, TemplateStatus, diff_fields, validate_submission,
};
use handbook_store::{db, store::Store};
use rusqlite::{Connection, OptionalExtension as _};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Digest as _;
use std::collections::BTreeMap;

use crate::{AppState, store_unavailable, unix_now};

/// P2 sub-router, nested at `/apostille/api/v1/p2`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/forms/{authority}/{code}", get(get_form))
        .route("/submissions", post(create_submission))
        .route(
            "/submissions/{id}",
            patch(patch_submission).get(get_submission),
        )
        .route("/submissions/{id}/pdf", get(get_pdf))
        .route("/corrections", post(post_correction))
        .route("/status", get(status))
        .merge(crate::p2_labels::router())
}

async fn status() -> impl IntoResponse {
    Json(json!({"status": "ok", "flow": "p2-forms"}))
}

// ---------------------------------------------------------------------------
// Views
// ---------------------------------------------------------------------------

/// One prefilled value: the statement plus the fact it traces to (P2-G2).
#[derive(Debug, Clone, Serialize)]
struct PrefillView {
    value: Value,
    fact_id: i64,
}

/// One withheld prefill (U4): conflicting accepted facts, or a value the
/// schema cannot take. Either way the customer types it manually.
#[derive(Debug, Clone, Serialize)]
struct WithheldView {
    key: String,
    topic: String,
    fact_ids: Vec<i64>,
    reason: String,
}

/// Template GET response: active template + prefill + replica state.
#[derive(Debug, Clone, Serialize)]
struct FormView {
    id: i64,
    authority: String,
    code: String,
    title: String,
    version: u32,
    authority_revision: String,
    status: String,
    fields: Vec<forms::FieldDef>,
    accepts_replica: String,
    signature_mode: String,
    prefill: BTreeMap<String, PrefillView>,
    withheld: Vec<WithheldView>,
    replica_notice: Option<String>,
    replica_unconfirmed: bool,
}

/// Submission review payload (U1 step 5 + review screen).
#[derive(Debug, Clone, Serialize)]
struct SubmissionView {
    id: i64,
    form_template_id: i64,
    authority: String,
    code: String,
    template_version: u32,
    values: BTreeMap<String, Value>,
    customer_override: bool,
    replica_unconfirmed: bool,
    replica_notice: Option<String>,
    pdf_sha256: String,
    pdf_url: Option<String>,
    status: String,
    created_at: i64,
    submitted_at: Option<i64>,
}

// ---------------------------------------------------------------------------
// Auth: the customer behind the request
// ---------------------------------------------------------------------------

/// Resolve the caller's `customer_ref` from either session cookie: the
/// DEMO `demo_session` cookie or a real auth session with a customer
/// ref. `None` means anonymous — customer endpoints answer 401.
fn caller_customer_ref(state: &AppState, headers: &HeaderMap) -> Option<String> {
    let cookies = headers.get(header::COOKIE)?.to_str().ok()?;
    let conn = db::open(&Store::new(&state.store_root)).ok()?;
    if let Some(token) = cookie_value(cookies, "demo_session")
        && let Ok(Some((customer_ref, expires_at))) = db::get_demo_session(&conn, token)
        && unix_now() < expires_at
    {
        return Some(customer_ref);
    }
    let token = auth::cookie_value(cookies, auth::ADM_COOKIE)
        .or_else(|| auth::cookie_value(cookies, auth::PUB_COOKIE))?;
    let session = auth::get_session(&conn, token).ok()??;
    if !auth::session_usable(&conn, &session, unix_now()) {
        return None;
    }
    session.customer_ref
}

/// WS ownership lookup for `submission:{id}` / `shipment:{id}`
/// topics: the S6 seam in `ws_bus.rs` swaps [`auth::ws::NoOwners`]
/// for this. Unknown ids fail closed (`None`).
pub struct P2Owners {
    store_root: std::path::PathBuf,
}

impl P2Owners {
    pub fn new(store_root: &std::path::Path) -> Self {
        Self {
            store_root: store_root.to_path_buf(),
        }
    }
}

impl auth::ws::OwnerLookup for P2Owners {
    fn submission_owner(&self, id: i64) -> Option<String> {
        let conn = db::open_p2(&Store::new(&self.store_root)).ok()?;
        load_submission(&conn, id).map(|s| s.customer_ref)
    }

    fn shipment_owner(&self, id: i64) -> Option<String> {
        let conn = db::open_p2(&Store::new(&self.store_root)).ok()?;
        conn.query_row(
            "SELECT s.customer_ref FROM form_submission s
             JOIN shipment h ON h.submission_id = s.id
             WHERE h.id = ?1",
            [id],
            |row| row.get(0),
        )
        .optional()
        .ok()
        .flatten()
    }
}

fn cookie_value<'a>(cookies: &'a str, name: &str) -> Option<&'a str> {
    cookies
        .split(';')
        .map(str::trim)
        .find_map(|pair| pair.strip_prefix(&format!("{name}=")))
        .filter(|v| !v.is_empty())
}

fn unauthorized() -> axum::response::Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({"error": "unauthorized"})),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Store access (per-request connections; rusqlite is !Send)
// ---------------------------------------------------------------------------

fn open_meta(state: &AppState) -> Option<Connection> {
    db::open(&Store::new(&state.store_root)).ok()
}

/// P2 customer store, migrated idempotently (safe per request: pending
/// migrations are rare and `migrate_p2` is a no-op once applied).
fn open_p2(state: &AppState) -> Option<Connection> {
    let conn = db::open_p2(&Store::new(&state.store_root)).ok()?;
    db::migrate_p2(&conn).ok()?;
    Some(conn)
}

/// Latest published build, 0 when nothing published yet (mirrors
/// the MCP `lookup_authority` query; the answer is then explicitly
/// unbuilt, never misattributed).
fn current_build_no(meta: &Connection) -> i64 {
    meta.query_row("SELECT COALESCE(MAX(build_no), 0) FROM build", [], |row| {
        row.get(0)
    })
    .unwrap_or(0)
}

fn audit(conn: &Connection, actor: &str, action: &str, entity: &str, entity_id: i64, reason: &str) {
    let _ = conn.execute(
        "INSERT INTO audit(at, actor, action, entity, entity_id, reason)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![unix_now(), actor, action, entity, entity_id, reason],
    );
}

// ---------------------------------------------------------------------------
// Templates: DB row -> forms::FormTemplate
// ---------------------------------------------------------------------------

struct StoredTemplate {
    id: i64,
    authority_id: i64,
    authority_slug: String,
    template: FormTemplate,
}

fn parse_status(s: &str) -> Option<TemplateStatus> {
    match s {
        "draft" => Some(TemplateStatus::Draft),
        "active" => Some(TemplateStatus::Active),
        "needs_review" => Some(TemplateStatus::NeedsReview),
        "superseded" => Some(TemplateStatus::Superseded),
        "retired" => Some(TemplateStatus::Retired),
        _ => None,
    }
}

fn parse_replica(s: &str) -> Option<ReplicaAcceptance> {
    match s {
        "yes" => Some(ReplicaAcceptance::Yes),
        "no" => Some(ReplicaAcceptance::No),
        "unknown" => Some(ReplicaAcceptance::Unknown),
        _ => None,
    }
}

fn parse_signature(s: &str) -> Option<SignatureMode> {
    match s {
        "wet_ink" => Some(SignatureMode::WetInk),
        "drawn_optional" => Some(SignatureMode::DrawnOptional),
        _ => None,
    }
}

fn template_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Option<StoredTemplate>> {
    let status: String = row.get(6)?;
    let accepts_replica: String = row.get(9)?;
    let accepts_drawn: String = row.get(11)?;
    let signature_mode: String = row.get(12)?;
    let schema_json: String = row.get(7)?;
    let fact_ids_json: String = row.get(14)?;
    let fields = serde_json::from_str(&schema_json).unwrap_or_default();
    let fact_ids = serde_json::from_str(&fact_ids_json).unwrap_or_default();
    let (Some(status), Some(accepts_replica), Some(signature_mode)) = (
        parse_status(&status),
        parse_replica(&accepts_replica),
        parse_signature(&signature_mode),
    ) else {
        return Ok(None);
    };
    Ok(Some(StoredTemplate {
        id: row.get(0)?,
        authority_id: row.get(1)?,
        authority_slug: row.get(2)?,
        template: FormTemplate {
            authority: row.get(2)?,
            code: row.get(3)?,
            title: row.get(4)?,
            version: row.get::<_, i64>(5)? as u32,
            authority_revision: row.get(8)?,
            status,
            fields,
            accepts_replica,
            accepts_replica_capture_id: row.get(10)?,
            accepts_drawn_signature: accepts_drawn == "yes",
            signature_mode,
            source_capture_id: row.get(13)?,
            fact_ids,
            supersedes_id: row.get(15)?,
            last_verified_at: row.get(16)?,
            verify_cadence_days: row.get::<_, i64>(17)? as u32,
            created_by: row.get(18)?,
            created_at: row.get(19)?,
        },
    }))
}

/// Column order shared by the two template selects below.
const TEMPLATE_COLS: &str = "t.id, t.authority_id, a.slug, t.code, t.title, t.version,
    t.status, t.schema_json, t.authority_revision,
    t.accepts_replica, t.accepts_replica_capture_id, t.accepts_drawn_signature,
    t.signature_mode, t.source_capture_id, t.fact_ids_json, t.supersedes_id,
    t.last_verified_at, t.verify_cadence_days, t.created_by, t.created_at";

fn load_active_template(
    conn: &Connection,
    authority: &str,
    code: &str,
) -> rusqlite::Result<Option<StoredTemplate>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {TEMPLATE_COLS} FROM form_template t
         JOIN authority a ON a.id = t.authority_id
         WHERE a.slug = ?1 AND t.code = ?2 AND t.status = 'active'"
    ))?;
    stmt.query_row(rusqlite::params![authority, code], template_from_row)
        .optional()
        .map(|opt| opt.flatten())
}

fn load_template_by_id(conn: &Connection, id: i64) -> rusqlite::Result<Option<StoredTemplate>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {TEMPLATE_COLS} FROM form_template t
         JOIN authority a ON a.id = t.authority_id
         WHERE t.id = ?1"
    ))?;
    stmt.query_row([id], template_from_row)
        .optional()
        .map(|opt| opt.flatten())
}

/// Replica notice for the review screen and the PDF header (U3, A2).
fn replica_notice(template: &FormTemplate) -> Option<String> {
    match template.accepts_replica {
        ReplicaAcceptance::Yes => None,
        ReplicaAcceptance::No => Some(format!(
            "Print single-sided and mail this original printout — photocopies not accepted per source #{}",
            template
                .accepts_replica_capture_id
                .map_or("unrecorded".to_string(), |id| id.to_string()),
        )),
        ReplicaAcceptance::Unknown => Some(format!(
            "We haven't confirmed {} accepts printed forms — submission is flagged for staff review",
            template.authority,
        )),
        _ => Some("replica acceptance is under review".to_string()),
    }
}

// ---------------------------------------------------------------------------
// Prefill from accepted facts (U1, U4)
// ---------------------------------------------------------------------------

struct AcceptedFact {
    id: i64,
    statement: String,
}

fn accepted_facts(conn: &Connection, authority_id: i64, topic: &str) -> Vec<AcceptedFact> {
    conn.prepare(
        "SELECT id, statement FROM fact
         WHERE authority_id = ?1 AND topic = ?2 AND status = 'accepted'
         ORDER BY id",
    )
    .and_then(|mut stmt| {
        stmt.query_map(rusqlite::params![authority_id, topic], |row| {
            Ok(AcceptedFact {
                id: row.get(0)?,
                statement: row.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()
    })
    .unwrap_or_default()
}

/// Open (or reuse) a correction task for a conflicted prefill topic (U4).
/// Deduped on `p2-prefill:{slug}:{topic}` while an open task exists.
fn ensure_correction_task(
    conn: &Connection,
    authority_id: i64,
    slug: &str,
    topic: &str,
    fact_ids: &[i64],
) -> i64 {
    let dedup = format!("p2-prefill:{slug}:{topic}");
    let open: Option<i64> = conn
        .query_row(
            "SELECT id FROM task WHERE dedup_key = ?1
             AND status IN ('open', 'batched', 'in_progress', 'blocked')",
            [&dedup],
            |row| row.get(0),
        )
        .optional()
        .ok()
        .flatten();
    if let Some(id) = open {
        return id;
    }
    let detail = format!(
        "conflicting accepted facts on topic '{topic}': {:?}; prefill withheld",
        fact_ids
    );
    conn.execute(
        "INSERT INTO task(authority_id, kind, title, detail, status, priority,
                           dedup_key, created_by, created_at)
         VALUES (?1, 'correction', ?2, ?3, 'open', 0, ?4, 'p2-prefill', ?5)",
        rusqlite::params![
            authority_id,
            format!("Conflicting prefill facts: {topic}"),
            detail,
            dedup,
            unix_now(),
        ],
    )
    .ok();
    conn.last_insert_rowid()
}

/// Shape one fact statement into the field's value type. `None` means
/// "cannot prefill this field from this statement".
fn prefill_value(field: &forms::FieldDef, statement: &str) -> Option<Value> {
    use forms::FieldType;
    match field.field_type {
        FieldType::Text
        | FieldType::Textarea
        | FieldType::Email
        | FieldType::Tel
        | FieldType::Date
        | FieldType::Number
        | FieldType::Money => Some(Value::String(statement.to_string())),
        FieldType::Select | FieldType::Radio => {
            if field.options.iter().any(|o| o == statement) {
                Some(Value::String(statement.to_string()))
            } else {
                None
            }
        }
        FieldType::Checkbox => match statement.trim().to_ascii_lowercase().as_str() {
            "yes" | "true" | "1" => Some(Value::Bool(true)),
            "no" | "false" | "0" => Some(Value::Bool(false)),
            _ => None,
        },
        FieldType::AddressBlock => {
            let parsed: Value = serde_json::from_str(statement).ok()?;
            if parsed.is_object() {
                Some(parsed)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Compute prefill + withheld sets for the active template. Conflicting
/// accepted facts withhold the prefill and open a correction task (U4);
/// rejected/proposed facts never prefill.
fn compute_prefill(
    conn: &Connection,
    stored: &StoredTemplate,
) -> (BTreeMap<String, PrefillView>, Vec<WithheldView>) {
    let mut prefill = BTreeMap::new();
    let mut withheld = Vec::new();
    for field in &stored.template.fields {
        let Some(topic) = field.prefill_topic.as_deref() else {
            continue;
        };
        let facts = accepted_facts(conn, stored.authority_id, topic);
        let mut distinct: Vec<&AcceptedFact> = Vec::new();
        for fact in &facts {
            if !distinct.iter().any(|d| d.statement == fact.statement) {
                distinct.push(fact);
            }
        }
        if distinct.len() > 1 {
            let fact_ids: Vec<i64> = distinct.iter().map(|f| f.id).collect();
            ensure_correction_task(
                conn,
                stored.authority_id,
                &stored.authority_slug,
                topic,
                &fact_ids,
            );
            withheld.push(WithheldView {
                key: field.key.clone(),
                topic: topic.to_string(),
                fact_ids,
                reason: "needs confirmation — we found conflicting accepted facts".to_string(),
            });
            continue;
        }
        let Some(only) = distinct.first() else {
            continue;
        };
        match prefill_value(field, &only.statement) {
            Some(value) => {
                prefill.insert(
                    field.key.clone(),
                    PrefillView {
                        value,
                        fact_id: only.id,
                    },
                );
            }
            None => withheld.push(WithheldView {
                key: field.key.clone(),
                topic: topic.to_string(),
                fact_ids: vec![only.id],
                reason: "prefill does not fit this field — please type it manually".to_string(),
            }),
        }
    }
    (prefill, withheld)
}

/// GET template + prefill from accepted facts (U1 steps 1–2).
async fn get_form(
    State(state): State<AppState>,
    Path((authority, code)): Path<(String, String)>,
) -> impl IntoResponse {
    let Some(meta) = open_meta(&state) else {
        return store_unavailable();
    };
    let stored = match load_active_template(&meta, &authority, &code) {
        Ok(Some(stored)) => stored,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "no active template for this authority form"})),
            )
                .into_response();
        }
        Err(_) => return store_unavailable(),
    };
    if stored.template.sla_expired(unix_now()) {
        // SLA expiry flips active -> needs_review (P2-F6); the flip is
        // best-effort here and enforced again at submit time.
        let _ = meta.execute(
            "UPDATE form_template SET status = 'needs_review' WHERE id = ?1",
            [stored.id],
        );
        audit(
            &meta,
            "p2",
            "template_needs_review",
            "form_template",
            stored.id,
            "verify SLA expired",
        );
        return (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "template is under re-verification; please try again later",
                "status": "needs_review",
            })),
        )
            .into_response();
    }
    let (prefill, withheld) = compute_prefill(&meta, &stored);
    let template = &stored.template;
    let view = FormView {
        id: stored.id,
        authority: template.authority.clone(),
        code: template.code.clone(),
        title: template.title.clone(),
        version: template.version,
        authority_revision: template.authority_revision.clone(),
        status: "active".to_string(),
        fields: template.fields.clone(),
        accepts_replica: match template.accepts_replica {
            ReplicaAcceptance::Yes => "yes",
            ReplicaAcceptance::No => "no",
            ReplicaAcceptance::Unknown => "unknown",
            _ => "unknown",
        }
        .to_string(),
        signature_mode: match template.signature_mode {
            SignatureMode::WetInk => "wet_ink",
            _ => "drawn_optional",
        }
        .to_string(),
        prefill,
        withheld,
        replica_notice: replica_notice(template),
        replica_unconfirmed: template.accepts_replica == ReplicaAcceptance::Unknown,
    };
    (StatusCode::OK, Json(view)).into_response()
}

// ---------------------------------------------------------------------------
// Submissions: validate -> persist -> render -> notify
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateSubmissionBody {
    template_id: i64,
    #[serde(default)]
    values: BTreeMap<String, Value>,
    /// Save as draft without validation (autosave); submit later.
    #[serde(default)]
    draft: bool,
    /// Customer typed over a withheld/conflicting prefill (U4).
    #[serde(default)]
    customer_override: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PatchSubmissionBody {
    #[serde(default)]
    values: Option<BTreeMap<String, Value>>,
    /// Flip a draft to submitted (validates + renders).
    #[serde(default)]
    submit: bool,
    #[serde(default)]
    customer_override: Option<bool>,
}

struct StoredSubmission {
    id: i64,
    form_template_id: i64,
    customer_ref: String,
    values: BTreeMap<String, Value>,
    customer_override: bool,
    replica_unconfirmed: bool,
    pdf_sha256: String,
    status: String,
    created_at: i64,
    submitted_at: Option<i64>,
}

fn load_submission(conn: &Connection, id: i64) -> Option<StoredSubmission> {
    conn.query_row(
        "SELECT id, form_template_id, customer_ref, values_json, customer_override,
                replica_unconfirmed, pdf_sha256, status, created_at, submitted_at
         FROM form_submission WHERE id = ?1",
        [id],
        |row| {
            let values_json: String = row.get(3)?;
            Ok(StoredSubmission {
                id: row.get(0)?,
                form_template_id: row.get(1)?,
                customer_ref: row.get(2)?,
                values: serde_json::from_str(&values_json).unwrap_or_default(),
                customer_override: row.get::<_, i64>(4)? != 0,
                replica_unconfirmed: row.get::<_, i64>(5)? != 0,
                pdf_sha256: row.get(6)?,
                status: row.get(7)?,
                created_at: row.get(8)?,
                submitted_at: row.get(9)?,
            })
        },
    )
    .optional()
    .ok()
    .flatten()
}

fn submission_view(
    stored: &StoredSubmission,
    template: &FormTemplate,
    base: &str,
) -> SubmissionView {
    SubmissionView {
        id: stored.id,
        form_template_id: stored.form_template_id,
        authority: template.authority.clone(),
        code: template.code.clone(),
        template_version: template.version,
        values: stored.values.clone(),
        customer_override: stored.customer_override,
        replica_unconfirmed: stored.replica_unconfirmed,
        replica_notice: replica_notice(template),
        pdf_sha256: stored.pdf_sha256.clone(),
        pdf_url: if stored.pdf_sha256.is_empty() {
            None
        } else {
            Some(format!("{base}/submissions/{}/pdf", stored.id))
        },
        status: stored.status.clone(),
        created_at: stored.created_at,
        submitted_at: stored.submitted_at,
    }
}

/// Refuse a submit against a non-active template (U2): audit the
/// refusal and tell the client the new version + changed keys so
/// entered values carry over field-by-field.
fn stale_refusal(
    meta: &Connection,
    actor: &str,
    started: &StoredTemplate,
) -> axum::response::Response {
    audit(
        meta,
        actor,
        "submission_refused_stale",
        "form_template",
        started.id,
        &format!(
            "template status is not active (v{})",
            started.template.version
        ),
    );
    let active = load_active_template(meta, &started.authority_slug, &started.template.code)
        .ok()
        .flatten();
    match active {
        Some(current) => {
            let diff = diff_fields(&started.template, &current.template);
            (
                StatusCode::CONFLICT,
                Json(json!({
                    "error": format!(
                        "form updated to version {} ({}); please review changed fields",
                        current.template.version, current.template.authority_revision,
                    ),
                    "status": "superseded",
                    "new_template_id": current.id,
                    "new_version": current.template.version,
                    "changed_keys": {
                        "added": diff.added,
                        "removed": diff.removed,
                        "changed": diff.changed,
                    },
                })),
            )
                .into_response()
        }
        None => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "this form edition is no longer active and no replacement is published",
                "status": "stale",
            })),
        )
            .into_response(),
    }
}

/// Validate + persist + render one submission. Shared by POST (new)
/// and PATCH (submit a draft).
struct SubmitArgs<'a> {
    customer_ref: &'a str,
    stored_template: &'a StoredTemplate,
    values: &'a BTreeMap<String, Value>,
    customer_override: bool,
    existing_draft: Option<i64>,
}

fn submit_validated(
    state: &AppState,
    meta: &Connection,
    p2: &Connection,
    args: SubmitArgs<'_>,
) -> axum::response::Response {
    let SubmitArgs {
        customer_ref,
        stored_template,
        values,
        customer_override,
        existing_draft,
    } = args;
    let template = &stored_template.template;
    let errors: Vec<FieldError> = validate_submission(template, values);
    if !errors.is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"errors": errors})),
        )
            .into_response();
    }
    if template.signature_mode != SignatureMode::WetInk {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(
                json!({"error": "this form needs a drawn signature, which P2 does not render yet"}),
            ),
        )
            .into_response();
    }
    let replica_unconfirmed = template.accepts_replica == ReplicaAcceptance::Unknown;
    let values_json = serde_json::to_string(values).unwrap_or_else(|_| "{}".to_string());
    let now = unix_now();
    let submission_id = match existing_draft {
        Some(id) => {
            let ok = p2
                .execute(
                    "UPDATE form_submission SET values_json = ?1, customer_override = ?2,
                            replica_unconfirmed = ?3, signature_mode_used = 'wet_ink',
                            status = 'submitted', submitted_at = ?4
                     WHERE id = ?5 AND status = 'draft'",
                    rusqlite::params![
                        values_json,
                        i64::from(customer_override),
                        i64::from(replica_unconfirmed),
                        now,
                        id
                    ],
                )
                .is_ok();
            if !ok {
                return store_unavailable();
            }
            id
        }
        None => {
            let ok = p2
                .execute(
                    "INSERT INTO form_submission(form_template_id, authority_id, customer_ref,
                            values_json, customer_override, replica_unconfirmed,
                            signature_mode_used, status, created_at, submitted_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'wet_ink', 'submitted', ?7, ?7)",
                    rusqlite::params![
                        stored_template.id,
                        stored_template.authority_id,
                        customer_ref,
                        values_json,
                        i64::from(customer_override),
                        i64::from(replica_unconfirmed),
                        now,
                    ],
                )
                .is_ok();
            if !ok {
                return store_unavailable();
            }
            p2.last_insert_rowid()
        }
    };
    // Render the filled PDF (wet-ink: empty signature + date lines).
    let notice = replica_notice(template);
    let build_no = current_build_no(meta);
    let render = PdfRenderer::new().render(&RenderSubmission {
        template,
        values,
        generated_at_unix: now,
        build_no: Some(build_no),
        replica_notice: notice.as_deref(),
    });
    let Ok(artifact) = render else {
        return store_unavailable();
    };
    let store = Store::new(&state.store_root);
    if store.write_blob(&artifact.bytes).is_err() {
        return store_unavailable();
    }
    let _ = p2.execute(
        "UPDATE form_submission SET pdf_sha256 = ?1 WHERE id = ?2",
        rusqlite::params![artifact.sha256_hex, submission_id],
    );
    audit(
        meta,
        customer_ref,
        "submission_submit",
        "form_submission",
        submission_id,
        &format!("template v{}", template.version),
    );
    let channel = format!("submission:{submission_id}");
    let pdf_url = format!("/apostille/api/v1/p2/submissions/{submission_id}/pdf");
    state.bus.publish(
        &channel,
        json!({"type": "form.validated", "topic": channel,
               "body": {"submission_id": submission_id}, "at": now})
        .to_string(),
    );
    state.bus.publish(
        &channel,
        json!({"type": "pdf.ready", "topic": channel,
               "body": {"sha256": artifact.sha256_hex, "url": pdf_url}, "at": now})
        .to_string(),
    );
    (
        StatusCode::CREATED,
        Json(json!({
            "id": submission_id,
            "status": "submitted",
            "pdf_url": pdf_url,
            "pdf_sha256": artifact.sha256_hex,
            "replica_notice": notice,
            "replica_unconfirmed": replica_unconfirmed,
            "customer_override": customer_override,
        })),
    )
        .into_response()
}

/// POST submit (validate -> persist -> render) or save a draft.
async fn create_submission(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateSubmissionBody>,
) -> impl IntoResponse {
    let Some(customer_ref) = caller_customer_ref(&state, &headers) else {
        return unauthorized();
    };
    let (Some(meta), Some(p2)) = (open_meta(&state), open_p2(&state)) else {
        return store_unavailable();
    };
    let stored = match load_template_by_id(&meta, body.template_id) {
        Ok(Some(stored)) => stored,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "unknown template"})),
            )
                .into_response();
        }
        Err(_) => return store_unavailable(),
    };
    if body.draft {
        let values_json = serde_json::to_string(&body.values).unwrap_or_else(|_| "{}".to_string());
        let now = unix_now();
        let ok = p2
            .execute(
                "INSERT INTO form_submission(form_template_id, authority_id, customer_ref,
                        values_json, customer_override, status, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'draft', ?6)",
                rusqlite::params![
                    stored.id,
                    stored.authority_id,
                    customer_ref,
                    values_json,
                    i64::from(body.customer_override),
                    now,
                ],
            )
            .is_ok();
        if !ok {
            return store_unavailable();
        }
        let id = p2.last_insert_rowid();
        audit(
            &meta,
            &customer_ref,
            "submission_draft",
            "form_submission",
            id,
            "",
        );
        return (
            StatusCode::CREATED,
            Json(json!({"id": id, "status": "draft"})),
        )
            .into_response();
    }
    if stored.template.status != TemplateStatus::Active || stored.template.sla_expired(unix_now()) {
        return stale_refusal(&meta, &customer_ref, &stored);
    }
    submit_validated(
        &state,
        &meta,
        &p2,
        SubmitArgs {
            customer_ref: &customer_ref,
            stored_template: &stored,
            values: &body.values,
            customer_override: body.customer_override,
            existing_draft: None,
        },
    )
}

/// PATCH a draft (autosave last-write-wins) or submit it.
async fn patch_submission(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(body): Json<PatchSubmissionBody>,
) -> impl IntoResponse {
    let Some(customer_ref) = caller_customer_ref(&state, &headers) else {
        return unauthorized();
    };
    let (Some(meta), Some(p2)) = (open_meta(&state), open_p2(&state)) else {
        return store_unavailable();
    };
    let Some(submission) = load_submission(&p2, id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown submission"})),
        )
            .into_response();
    };
    if submission.customer_ref != customer_ref {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown submission"})),
        )
            .into_response();
    }
    if submission.status != "draft" {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error": "only drafts can be edited"})),
        )
            .into_response();
    }
    let stored = match load_template_by_id(&meta, submission.form_template_id) {
        Ok(Some(stored)) => stored,
        _ => return store_unavailable(),
    };
    let values = body.values.unwrap_or(submission.values);
    let customer_override = body
        .customer_override
        .unwrap_or(submission.customer_override);
    if !body.submit {
        let values_json = serde_json::to_string(&values).unwrap_or_else(|_| "{}".to_string());
        let ok = p2
            .execute(
                "UPDATE form_submission SET values_json = ?1, customer_override = ?2
                 WHERE id = ?3 AND status = 'draft'",
                rusqlite::params![values_json, i64::from(customer_override), id],
            )
            .is_ok();
        if !ok {
            return store_unavailable();
        }
        audit(
            &meta,
            &customer_ref,
            "submission_autosave",
            "form_submission",
            id,
            "",
        );
        return (StatusCode::OK, Json(json!({"id": id, "status": "draft"}))).into_response();
    }
    if stored.template.status != TemplateStatus::Active || stored.template.sla_expired(unix_now()) {
        return stale_refusal(&meta, &customer_ref, &stored);
    }
    submit_validated(
        &state,
        &meta,
        &p2,
        SubmitArgs {
            customer_ref: &customer_ref,
            stored_template: &stored,
            values: &values,
            customer_override,
            existing_draft: Some(id),
        },
    )
}

/// GET the review payload: values + template identity + replica state.
async fn get_submission(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let Some(customer_ref) = caller_customer_ref(&state, &headers) else {
        return unauthorized();
    };
    let (Some(meta), Some(p2)) = (open_meta(&state), open_p2(&state)) else {
        return store_unavailable();
    };
    let Some(submission) = load_submission(&p2, id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown submission"})),
        )
            .into_response();
    };
    if submission.customer_ref != customer_ref {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown submission"})),
        )
            .into_response();
    }
    let stored = match load_template_by_id(&meta, submission.form_template_id) {
        Ok(Some(stored)) => stored,
        _ => return store_unavailable(),
    };
    let view = submission_view(&submission, &stored.template, "/apostille/api/v1/p2");
    (StatusCode::OK, Json(view)).into_response()
}

/// GET the filled PDF (print-ready attachment, cached by sha).
async fn get_pdf(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let Some(customer_ref) = caller_customer_ref(&state, &headers) else {
        return unauthorized();
    };
    let (Some(meta), Some(p2)) = (open_meta(&state), open_p2(&state)) else {
        return store_unavailable();
    };
    let Some(submission) = load_submission(&p2, id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown submission"})),
        )
            .into_response();
    };
    if submission.customer_ref != customer_ref {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown submission"})),
        )
            .into_response();
    }
    if submission.pdf_sha256.is_empty() {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error": "no PDF yet — submit the form first"})),
        )
            .into_response();
    }
    let stored = match load_template_by_id(&meta, submission.form_template_id) {
        Ok(Some(stored)) => stored,
        _ => return store_unavailable(),
    };
    let bytes = match Store::new(&state.store_root).read_blob(&submission.pdf_sha256) {
        Ok(bytes) => bytes,
        Err(_) => return store_unavailable(),
    };
    let filename = format!(
        "{}-v{}-{}.pdf",
        stored.template.code, stored.template.version, submission.id
    );
    (
        StatusCode::OK,
        [
            ("content-type", "application/pdf".to_string()),
            (
                "content-disposition",
                format!("attachment; filename=\"{filename}\""),
            ),
            ("etag", format!("\"{}\"", submission.pdf_sha256)),
        ],
        bytes,
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Correction loop hook (U5)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorrectionBody {
    template_id: Option<i64>,
    submission_id: Option<i64>,
    topic: String,
    note: String,
}

/// Customer/staff reports a wrong prefill: `capture(kind=comment)` +
/// `task(kind=correction)` through the P1 machinery, and the affected
/// active template flips to `needs_review` until staff re-approves.
async fn post_correction(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CorrectionBody>,
) -> impl IntoResponse {
    let Some(customer_ref) = caller_customer_ref(&state, &headers) else {
        return unauthorized();
    };
    let topic = body.topic.trim();
    let note = body.note.trim();
    if topic.is_empty() || topic.len() > 64 || note.is_empty() || note.len() > 2000 {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "topic (1-64) and note (1-2000 chars) required"})),
        )
            .into_response();
    }
    let (Some(meta), p2) = (open_meta(&state), open_p2(&state)) else {
        return store_unavailable();
    };
    // Resolve the authority: explicit template wins, else the
    // submission's template (owned by the caller).
    let mut template_id = body.template_id;
    if template_id.is_none()
        && let (Some(submission_id), Some(p2conn)) = (body.submission_id, p2.as_ref())
        && let Some(submission) = load_submission(p2conn, submission_id)
        && submission.customer_ref == customer_ref
    {
        template_id = Some(submission.form_template_id);
    }
    let Some(template_id) = template_id else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "template_id or an owned submission_id required"})),
        )
            .into_response();
    };
    let stored = match load_template_by_id(&meta, template_id) {
        Ok(Some(stored)) => stored,
        _ => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "unknown template"})),
            )
                .into_response();
        }
    };
    let now = unix_now();
    let note_sha = format!("{:x}", sha2::Sha256::digest(note.as_bytes()));
    let capture_id = match meta.execute(
        "INSERT INTO capture(authority_id, captured_at, kind, path, bytes_raw,
                              bytes_stored, sha256, trust, notes)
         VALUES (?1, ?2, 'comment', '', 0, 0, ?3, 'community', ?4)",
        rusqlite::params![stored.authority_id, now, note_sha, note],
    ) {
        Ok(_) => meta.last_insert_rowid(),
        Err(_) => return store_unavailable(),
    };
    let dedup = format!(
        "p2-correction:{}:{}:{template_id}",
        stored.authority_slug, topic
    );
    let task_id: i64 = meta
        .query_row(
            "SELECT id FROM task WHERE dedup_key = ?1
             AND status IN ('open', 'batched', 'in_progress', 'blocked')",
            [&dedup],
            |row| row.get(0),
        )
        .optional()
        .ok()
        .flatten()
        .unwrap_or_else(|| {
            meta.execute(
                "INSERT INTO task(authority_id, kind, title, detail, status, priority,
                                   dedup_key, created_by, created_at)
                 VALUES (?1, 'correction', ?2, ?3, 'open', 0, ?4, ?5, ?6)",
                rusqlite::params![
                    stored.authority_id,
                    format!("Customer correction: {topic}"),
                    format!("capture #{capture_id}: {note}"),
                    dedup,
                    customer_ref,
                    now,
                ],
            )
            .ok();
            meta.last_insert_rowid()
        });
    let mut template_status = "active".to_string();
    if stored.template.status == TemplateStatus::Active {
        let _ = meta.execute(
            "UPDATE form_template SET status = 'needs_review' WHERE id = ?1",
            [stored.id],
        );
        template_status = "needs_review".to_string();
    }
    audit(
        &meta,
        &customer_ref,
        "correction_report",
        "form_template",
        stored.id,
        &format!("topic {topic}; task #{task_id}"),
    );
    (
        StatusCode::CREATED,
        Json(json!({
            "capture_id": capture_id,
            "task_id": task_id,
            "template_status": template_status,
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt as _;

    const CUSTOMER: &str = "cust-1";
    const TOKEN: &str = "test-token-1";

    fn field(
        key: &str,
        label: &str,
        field_type: forms::FieldType,
        required: bool,
    ) -> forms::FieldDef {
        forms::FieldDef {
            key: key.to_string(),
            label: label.to_string(),
            help: None,
            field_type,
            required,
            max_len: None,
            pattern: None,
            options: Vec::new(),
            prefill_topic: None,
            pdf: forms::PdfRect {
                page: 1,
                x_mm: 20.0,
                y_mm: 60.0,
                w_mm: 170.0,
                h_mm: 8.0,
                font_size_pt: 11.0,
                multiline: false,
            },
        }
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        state: AppState,
        authority_id: i64,
        capture_id: i64,
        template_id: i64,
    }

    impl Fixture {
        fn new() -> Self {
            Self::with_template(|_| {})
        }

        /// Seed a store with one authority, one provenance capture, and
        /// one active template; `tweak` adjusts the template row insert.
        fn with_template(tweak: impl FnOnce(&mut TemplateSeed)) -> Self {
            let dir = tempfile::TempDir::new().expect("tempdir");
            let store = Store::new(dir.path());
            store.init().unwrap();
            let meta = db::open(&store).unwrap();
            db::migrate(&meta).unwrap();
            let p2 = db::open_p2(&store).unwrap();
            db::migrate_p2(&p2).unwrap();
            let now = unix_now();
            let authority_id = db::add_authority(
                &meta,
                &db::NewAuthority {
                    kind: "state",
                    country: "USA",
                    region: "WA",
                    name: "WA Secretary of State",
                    slug: "usa-wa-sos",
                    actor: "test",
                    at: now,
                },
            )
            .unwrap();
            meta.execute(
                "INSERT INTO capture(authority_id, captured_at, kind, path, bytes_raw,
                                      bytes_stored, sha256, trust)
                 VALUES (?1, ?2, 'file', 'seed.pdf', 1, 1, 'aa', 'official')",
                rusqlite::params![authority_id, now],
            )
            .unwrap();
            let capture_id = meta.last_insert_rowid();
            db::create_demo_session(&meta, TOKEN, CUSTOMER, now, now + 3600).unwrap();
            let mut seed = TemplateSeed {
                status: "active",
                accepts_replica: "unknown",
                version: 1,
                fields: None,
            };
            tweak(&mut seed);
            let fields = seed.fields.unwrap_or_else(|| {
                let mut fee = field("fee", "Fee", forms::FieldType::Money, true);
                fee.prefill_topic = Some("fees".to_string());
                vec![
                    field("full_name", "Full name", forms::FieldType::Text, true),
                    fee,
                ]
            });
            let schema_json = serde_json::to_string(&fields).unwrap();
            meta.execute(
                "INSERT INTO form_template(authority_id, code, title, version,
                        authority_revision, status, schema_json, accepts_replica,
                        signature_mode, last_verified_at, verify_cadence_days,
                        created_by, created_at)
                 VALUES (?1, 'wa-apostille-request', 'WA Apostille Request', ?2,
                        'Rev 06/2025', ?3, ?4, ?5, 'wet_ink', ?6, 90, 'test', ?6)",
                rusqlite::params![
                    authority_id,
                    seed.version,
                    seed.status,
                    schema_json,
                    seed.accepts_replica,
                    now,
                ],
            )
            .unwrap();
            let template_id = meta.last_insert_rowid();
            let state = AppState {
                spa_index: dir.path().join("index.html"),
                static_dir: dir.path().to_path_buf(),
                store_root: dir.path().to_path_buf(),
                bus: crate::ws_bus::Bus::new(),
                kek: None,
                admin_mtls_off: true,
                admin_spa_dir: dir.path().to_path_buf(),
                media_terms_read: false,
                label_cfg: crate::p2_labels::LabelCfg::default(),
                voice_cfg: voice::VoiceConfig::default(), // ADMIN-VOICE
                voice_api_key: None,
                webauthn: std::sync::Arc::new(crate::passkey::test_webauthn()),
                auth_limits: std::sync::Arc::new(crate::passkey::RateBuckets::default()),
                google_cfg: None,
                apple_cfg: None,
                meta_cfg: None,
                mailer: None,
                magic_link_base: "https://example.test/apostille/login".to_string(),
            };
            Self {
                _dir: dir,
                state,
                authority_id,
                capture_id,
                template_id,
            }
        }

        fn app(&self) -> Router {
            Router::new()
                .nest("/p2", router())
                .with_state(self.state.clone())
        }

        fn meta(&self) -> Connection {
            db::open(&Store::new(&self.state.store_root)).unwrap()
        }

        fn add_fact(&self, topic: &str, statement: &str, status: &str) -> i64 {
            let meta = self.meta();
            meta.execute(
                "INSERT INTO fact(authority_id, capture_id, topic, statement, status, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    self.authority_id,
                    self.capture_id,
                    topic,
                    statement,
                    status,
                    unix_now()
                ],
            )
            .unwrap();
            meta.last_insert_rowid()
        }

        fn login_as(&self, token: &str, customer_ref: &str) {
            let meta = self.meta();
            let now = unix_now();
            db::create_demo_session(&meta, token, customer_ref, now, now + 3600).unwrap();
        }
    }

    struct TemplateSeed {
        status: &'static str,
        accepts_replica: &'static str,
        version: i64,
        fields: Option<Vec<forms::FieldDef>>,
    }

    fn authed(method: &str, uri: &str, token: &str, body: Option<Value>) -> Request<Body> {
        let mut builder = Request::builder()
            .uri(uri)
            .method(method)
            .header("cookie", format!("demo_session={token}"));
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        builder
            .body(body.map_or_else(Body::empty, |v| Body::from(v.to_string())))
            .unwrap()
    }

    async fn json_body(res: axum::response::Response) -> (StatusCode, Value) {
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, value)
    }

    #[tokio::test]
    async fn form_get_prefills_accepted_and_ignores_rejected() {
        let fx = Fixture::new();
        let accepted = fx.add_fact("fees", "25.00", "accepted");
        fx.add_fact("fees", "99.00", "rejected");
        fx.add_fact("fees", "10.00", "proposed");
        let (status, body) = json_body(
            fx.app()
                .oneshot(authed(
                    "GET",
                    "/p2/forms/usa-wa-sos/wa-apostille-request",
                    TOKEN,
                    None,
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["version"], 1);
        assert_eq!(body["code"], "wa-apostille-request");
        assert_eq!(body["prefill"]["fee"]["value"], "25.00");
        assert_eq!(body["prefill"]["fee"]["fact_id"], accepted);
        assert!(body["withheld"].as_array().unwrap().is_empty());
        assert!(body["replica_unconfirmed"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn form_get_withholds_on_conflict_and_opens_one_task() {
        let fx = Fixture::new();
        let a = fx.add_fact("fees", "25.00", "accepted");
        let b = fx.add_fact("fees", "30.00", "accepted");
        for _ in 0..2 {
            let (status, body) = json_body(
                fx.app()
                    .oneshot(authed(
                        "GET",
                        "/p2/forms/usa-wa-sos/wa-apostille-request",
                        TOKEN,
                        None,
                    ))
                    .await
                    .unwrap(),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert!(body["prefill"].as_object().unwrap().is_empty());
            let withheld = body["withheld"].as_array().unwrap();
            assert_eq!(withheld.len(), 1);
            assert_eq!(withheld[0]["key"], "fee");
            let ids: Vec<i64> = withheld[0]["fact_ids"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_i64().unwrap())
                .collect();
            assert_eq!(ids, vec![a, b]);
            assert!(
                withheld[0]["reason"]
                    .as_str()
                    .unwrap()
                    .contains("needs confirmation")
            );
        }
        let meta = fx.meta();
        let tasks: i64 = meta
            .query_row(
                "SELECT COUNT(*) FROM task WHERE kind = 'correction'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tasks, 1, "conflict opens exactly one deduped task");
    }

    #[tokio::test]
    async fn submit_happy_path_renders_pdf_and_publishes_ready() {
        let fx = Fixture::new();
        fx.add_fact("fees", "25.00", "accepted");
        let mut rx = fx.state.bus.subscribe("submission:1");
        let (status, body) = json_body(
            fx.app()
                .oneshot(authed(
                    "POST",
                    "/p2/submissions",
                    TOKEN,
                    Some(json!({
                        "template_id": fx.template_id,
                        "values": {"full_name": "Jane Doe", "fee": "25.00"},
                    })),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(body["id"], 1);
        assert_eq!(body["status"], "submitted");
        assert_eq!(body["pdf_sha256"].as_str().unwrap().len(), 64);
        assert!(body["replica_unconfirmed"].as_bool().unwrap());
        // WS fan-out on submission:1: form.validated then pdf.ready.
        let first: Value = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
        assert_eq!(first["type"], "form.validated");
        let second: Value = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
        assert_eq!(second["type"], "pdf.ready");
        assert_eq!(second["topic"], "submission:1");
        assert_eq!(second["body"]["sha256"], body["pdf_sha256"]);
        // Review screen payload.
        let (status, review) = json_body(
            fx.app()
                .oneshot(authed("GET", "/p2/submissions/1", TOKEN, None))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(review["values"]["full_name"], "Jane Doe");
        assert!(review["pdf_url"].as_str().unwrap().ends_with("/pdf"));
        // PDF bytes.
        let res = fx
            .app()
            .oneshot(authed("GET", "/p2/submissions/1/pdf", TOKEN, None))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers()["content-type"], "application/pdf");
        assert!(
            res.headers()["content-disposition"]
                .to_str()
                .unwrap()
                .contains("wa-apostille-request-v1-1.pdf")
        );
        let bytes = axum::body::to_bytes(res.into_body(), 4 * 1024 * 1024)
            .await
            .unwrap();
        assert!(bytes.starts_with(b"%PDF-"));
    }

    #[tokio::test]
    async fn submit_invalid_returns_per_field_errors() {
        let fx = Fixture::new();
        let (status, body) = json_body(
            fx.app()
                .oneshot(authed(
                    "POST",
                    "/p2/submissions",
                    TOKEN,
                    Some(json!({
                        "template_id": fx.template_id,
                        "values": {"full_name": "", "fee": "bogus"},
                    })),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let errors = body["errors"].as_array().unwrap();
        assert_eq!(errors.len(), 2);
        assert_eq!(errors[0]["key"], "full_name");
        assert_eq!(errors[0]["code"], "required");
        assert_eq!(errors[1]["key"], "fee");
        assert_eq!(errors[1]["code"], "money");
        // Nothing persisted.
        let p2 = db::open_p2(&Store::new(&fx.state.store_root)).unwrap();
        let count: i64 = p2
            .query_row("SELECT COUNT(*) FROM form_submission", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn stale_template_refused_with_changed_keys() {
        let fx = Fixture::with_template(|seed| {
            seed.status = "superseded";
        });
        // Publish v2 (the in-flight user started on v1).
        let meta = fx.meta();
        let mut email = field("email", "Email", forms::FieldType::Email, true);
        email.prefill_topic = None;
        let v2_fields = vec![
            field("full_name", "Full name", forms::FieldType::Text, true),
            field("fee", "Fee", forms::FieldType::Money, true),
            email,
        ];
        meta.execute(
            "INSERT INTO form_template(authority_id, code, title, version,
                    authority_revision, status, schema_json, accepts_replica,
                    signature_mode, last_verified_at, verify_cadence_days,
                    created_by, created_at)
             VALUES (?1, 'wa-apostille-request', 'WA Apostille Request', 2,
                    'Rev 09/2025', 'active', ?2, 'unknown', 'wet_ink', ?3, 90, 'test', ?3)",
            rusqlite::params![
                fx.authority_id,
                serde_json::to_string(&v2_fields).unwrap(),
                unix_now()
            ],
        )
        .unwrap();
        let (status, body) = json_body(
            fx.app()
                .oneshot(authed(
                    "POST",
                    "/p2/submissions",
                    TOKEN,
                    Some(json!({
                        "template_id": fx.template_id,
                        "values": {"full_name": "Jane", "fee": "25.00"},
                    })),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["status"], "superseded");
        assert_eq!(body["new_version"], 2);
        assert_eq!(body["changed_keys"]["added"], json!(["email"]));
        assert!(body["error"].as_str().unwrap().contains("changed fields"));
        let refusals: i64 = meta
            .query_row(
                "SELECT COUNT(*) FROM audit WHERE action = 'submission_refused_stale'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(refusals, 1);
    }

    #[tokio::test]
    async fn draft_autosave_then_submit() {
        let fx = Fixture::new();
        let (status, draft) = json_body(
            fx.app()
                .oneshot(authed(
                    "POST",
                    "/p2/submissions",
                    TOKEN,
                    Some(json!({
                        "template_id": fx.template_id,
                        "draft": true,
                        "values": {"full_name": "half"},
                    })),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(draft["status"], "draft");
        // Autosave last-write-wins (invalid values allowed in drafts).
        let (status, _) = json_body(
            fx.app()
                .oneshot(authed(
                    "PATCH",
                    "/p2/submissions/1",
                    TOKEN,
                    Some(json!({"values": {"full_name": ""}})),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        // Submit validates.
        let (status, _) = json_body(
            fx.app()
                .oneshot(authed(
                    "PATCH",
                    "/p2/submissions/1",
                    TOKEN,
                    Some(json!({"submit": true})),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let (status, done) = json_body(
            fx.app()
                .oneshot(authed(
                    "PATCH",
                    "/p2/submissions/1",
                    TOKEN,
                    Some(json!({
                        "submit": true,
                        "values": {"full_name": "Jane Doe", "fee": "25.00"},
                    })),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(done["status"], "submitted");
        // Submitted rows are immutable.
        let (status, _) = json_body(
            fx.app()
                .oneshot(authed(
                    "PATCH",
                    "/p2/submissions/1",
                    TOKEN,
                    Some(json!({"values": {}})),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn replica_no_notice_on_review_and_pdf() {
        let fx = Fixture::with_template(|seed| {
            seed.accepts_replica = "no";
        });
        let (status, body) = json_body(
            fx.app()
                .oneshot(authed(
                    "POST",
                    "/p2/submissions",
                    TOKEN,
                    Some(json!({
                        "template_id": fx.template_id,
                        "values": {"full_name": "Jane Doe", "fee": "25.00"},
                    })),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert!(
            body["replica_notice"]
                .as_str()
                .unwrap()
                .contains("single-sided")
        );
        assert!(!body["replica_unconfirmed"].as_bool().unwrap());
        let (status, review) = json_body(
            fx.app()
                .oneshot(authed("GET", "/p2/submissions/1", TOKEN, None))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            review["replica_notice"]
                .as_str()
                .unwrap()
                .contains("photocopies not accepted")
        );
    }

    #[tokio::test]
    async fn correction_loop_opens_task_and_parks_template() {
        let fx = Fixture::new();
        let (status, body) = json_body(
            fx.app()
                .oneshot(authed(
                    "POST",
                    "/p2/corrections",
                    TOKEN,
                    Some(json!({
                        "template_id": fx.template_id,
                        "topic": "fees",
                        "note": "the fee changed last week",
                    })),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(body["template_status"], "needs_review");
        let meta = fx.meta();
        let capture_kind: String = meta
            .query_row(
                "SELECT kind FROM capture WHERE id = ?1",
                [body["capture_id"].as_i64().unwrap()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(capture_kind, "comment");
        let task_kind: String = meta
            .query_row(
                "SELECT kind FROM task WHERE id = ?1",
                [body["task_id"].as_i64().unwrap()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(task_kind, "correction");
        let template_status: String = meta
            .query_row(
                "SELECT status FROM form_template WHERE id = ?1",
                [fx.template_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(template_status, "needs_review");
        // A repeat report reuses the open task.
        let (status, again) = json_body(
            fx.app()
                .oneshot(authed(
                    "POST",
                    "/p2/corrections",
                    TOKEN,
                    Some(json!({
                        "template_id": fx.template_id,
                        "topic": "fees",
                        "note": "still wrong",
                    })),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(again["task_id"], body["task_id"]);
    }

    #[tokio::test]
    async fn auth_and_ownership_close_the_doors() {
        let fx = Fixture::new();
        fx.login_as("other-token", "cust-2");
        // Anonymous submit.
        let res = fx
            .app()
            .oneshot(
                Request::builder()
                    .uri("/p2/submissions")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({"template_id": fx.template_id}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        // Other customer cannot see cust-1's submission.
        let (status, _) = json_body(
            fx.app()
                .oneshot(authed(
                    "POST",
                    "/p2/submissions",
                    TOKEN,
                    Some(json!({
                        "template_id": fx.template_id,
                        "draft": true,
                        "values": {},
                    })),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, _) = json_body(
            fx.app()
                .oneshot(authed("GET", "/p2/submissions/1", "other-token", None))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    /// Validation parity: the shared vectors in
    /// `web/test/forms-validate-vectors.json` must produce exactly the
    /// pinned codes and messages through the Rust validator — the same
    /// file the TS port runs against in `node --test`.
    #[test]
    fn validation_parity_with_ts_port() {
        use forms::FieldType;
        let path = format!(
            "{}/../../web/test/forms-validate-vectors.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let raw = std::fs::read_to_string(&path).expect("vectors file");
        let vectors: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(vectors["version"], 1);
        let cases = vectors["cases"].as_array().unwrap();
        assert!(cases.len() > 30, "vectors cover every rule");
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let f = &case["field"];
            let field_type = match f["type"].as_str().unwrap() {
                "text" => FieldType::Text,
                "textarea" => FieldType::Textarea,
                "email" => FieldType::Email,
                "tel" => FieldType::Tel,
                "date" => FieldType::Date,
                "number" => FieldType::Number,
                "money" => FieldType::Money,
                "select" => FieldType::Select,
                "radio" => FieldType::Radio,
                "checkbox" => FieldType::Checkbox,
                "address_block" => FieldType::AddressBlock,
                other => panic!("case {name}: unknown type {other}"),
            };
            let field_def = forms::FieldDef {
                key: f["key"].as_str().unwrap().to_string(),
                label: f["label"].as_str().unwrap().to_string(),
                help: None,
                field_type,
                required: f["required"].as_bool().unwrap_or(false),
                max_len: f["max_len"].as_u64().map(|n| n as usize),
                pattern: f["pattern"].as_str().map(str::to_string),
                options: f["options"]
                    .as_array()
                    .map(|opts| {
                        opts.iter()
                            .map(|o| o.as_str().unwrap().to_string())
                            .collect()
                    })
                    .unwrap_or_default(),
                prefill_topic: None,
                pdf: forms::PdfRect {
                    page: 1,
                    x_mm: 20.0,
                    y_mm: 60.0,
                    w_mm: 170.0,
                    h_mm: 8.0,
                    font_size_pt: 11.0,
                    multiline: false,
                },
            };
            let template = forms::FormTemplate {
                authority: "test".to_string(),
                code: "test".to_string(),
                title: "test".to_string(),
                version: 1,
                authority_revision: "test".to_string(),
                status: forms::TemplateStatus::Draft,
                fields: vec![field_def],
                accepts_replica: forms::ReplicaAcceptance::Unknown,
                accepts_replica_capture_id: None,
                accepts_drawn_signature: false,
                signature_mode: forms::SignatureMode::WetInk,
                source_capture_id: None,
                fact_ids: Vec::new(),
                supersedes_id: None,
                last_verified_at: None,
                verify_cadence_days: 90,
                created_by: String::new(),
                created_at: 0,
            };
            let mut values = BTreeMap::new();
            if !case["omit"].as_bool().unwrap_or(false) {
                values.insert(
                    f["key"].as_str().unwrap().to_string(),
                    case["value"].clone(),
                );
            }
            let got: Vec<(String, String)> = validate_submission(&template, &values)
                .into_iter()
                .map(|e| (e.code, e.message))
                .collect();
            let want: Vec<(String, String)> = case["expect"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| {
                    (
                        e["code"].as_str().unwrap().to_string(),
                        e["message"].as_str().unwrap().to_string(),
                    )
                })
                .collect();
            assert_eq!(got, want, "case: {name}");
        }
    }

    #[test]
    fn current_build_no_zero_when_unbuilt_else_max() {
        let fx = Fixture::new();
        let meta = open_meta(&fx.state).unwrap();
        assert_eq!(current_build_no(&meta), 0);
        for build_no in [1, 7] {
            meta.execute(
                "INSERT INTO build(at, code_git_commit, build_no, facts_hash)
                 VALUES (1, 'c', ?1, 'h')",
                [build_no],
            )
            .unwrap();
        }
        assert_eq!(current_build_no(&meta), 7);
    }

    #[tokio::test]
    async fn submitted_pdf_carries_current_build_stamp() {
        let fx = Fixture::new();
        let meta = open_meta(&fx.state).unwrap();
        meta.execute(
            "INSERT INTO build(at, code_git_commit, build_no, facts_hash)
             VALUES (1, 'c', 7, 'h')",
            [],
        )
        .unwrap();
        // Happy-path submit (mirrors submit_happy_path_*).
        let (status, _) = json_body(
            fx.app()
                .oneshot(authed(
                    "POST",
                    "/p2/submissions",
                    TOKEN,
                    Some(serde_json::json!({
                        "template_id": fx.template_id,
                        "values": {"full_name": "Jane Doe", "fee": "25.00"},
                    })),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, review) = json_body(
            fx.app()
                .oneshot(authed("GET", "/p2/submissions/1", TOKEN, None))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        // Served bytes (also cross-checked against the stored sha below).
        let served = fx
            .app()
            .oneshot(authed("GET", "/p2/submissions/1/pdf", TOKEN, None))
            .await
            .unwrap();
        assert_eq!(served.status(), StatusCode::OK);
        let served_bytes = axum::body::to_bytes(served.into_body(), 2 * 1024 * 1024)
            .await
            .unwrap();
        // submitted_at is not in the review view: read it from the row
        // (submit stamps render-time `now`, so re-render is byte-exact).
        let p2 = open_p2(&fx.state).unwrap();
        let submitted_at: i64 = p2
            .query_row(
                "SELECT submitted_at FROM form_submission WHERE id = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let stored = load_template_by_id(&meta, fx.template_id).unwrap().unwrap();
        let values: std::collections::BTreeMap<String, serde_json::Value> = review["values"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let rendered = |build_no| {
            forms::PdfRenderer::new()
                .render(&forms::Submission {
                    template: &stored.template,
                    values: &values,
                    generated_at_unix: submitted_at,
                    build_no,
                    replica_notice: replica_notice(&stored.template).as_deref(),
                })
                .unwrap()
        };
        let stamped = rendered(Some(7));
        let unstamped = rendered(None);
        assert_ne!(stamped.bytes, unstamped.bytes, "stamp must move bytes");
        assert_eq!(
            served_bytes.as_ref(),
            stamped.bytes.as_slice(),
            "served PDF carries build #7"
        );
        assert_eq!(review["pdf_sha256"], stamped.sha256_hex);
    }
}
