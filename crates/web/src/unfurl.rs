//! Social unfurl + crawler surface for dossier pages.
//!
//! `GET /apostille/authority/{slug}` serves the SPA shell with an
//! injected head: HTML-escaped `<title>`, meta description,
//! canonical, `og:*` tags, a `<noscript>` guide summary built from
//! accepted facts (`{topic label}: {value}`, statement keys stripped),
//! and JSON-LD `GovernmentService`, whose postal address is only ever
//! the office's one undisputed PO box, read as orders reads it for
//! USPS labels, and whose `dateModified` is the day the description
//! names. Unknown slugs
//! get the same shell with a 404 status (client routing shows its
//! own notice). `GET /apostille/sitemap.xml` lists authorities with
//! `lastmod` from the newest accepted capture. Every human-visible
//! string comes from staff-accepted rows — no generated text.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use axum::{
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use handbook_store::db;
use serde_json::json;

use super::passkey;
use super::{AppState, store_unavailable};

/// One accepted fact, enough for the crawler surface.
pub struct MetaFact {
    pub topic: String,
    pub statement: String,
}

/// Everything the head injects for one dossier.
pub struct DossierMeta {
    /// Canonical public address (`usa/washington`; the slug when the
    /// authority has no readable path).
    pub path: String,
    pub name: String,
    pub place: String,
    pub facts: Vec<MetaFact>,
    pub sources: Vec<String>,
    /// Newest capture day, if any fact carries one: JSON-LD
    /// `dateModified` only while no accept is recorded.
    pub last_day: Option<String>,
    /// Newest staff accept day of a listed fact (audit `fact_accept`):
    /// the description's "last updated" day and `dateModified`.
    pub last_accept: Option<String>,
    /// The office's one undisputed PO box, key stripped: the address
    /// orders' label reader parses, else the box as written
    /// ([`usps_postal`]), for the JSON-LD service channel.
    pub postal: Option<String>,
}

/// One accepted `mailing`/`po_box` fact as the JSON-LD postal check
/// reads it.
struct UspsFact {
    /// The statement alone: the shared rule and the emitted value read it.
    statement: String,
    /// The fact as orders reads it ([`crate::orders::fact_texts`]): the
    /// statement, then its public quote, with the `/facts` conflict
    /// flag. The head and the USPS labels then pause together.
    text: crate::orders::FactText,
}

/// Attribute/text escape for injected HTML (`&<>"'`).
pub fn esc(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Load one dossier's crawler surface. `Ok(None)` is an unknown
/// slug; only accepted facts shape the output.
pub fn load_dossier(
    conn: &rusqlite::Connection,
    store_root: &std::path::Path,
    slug: &str,
) -> rusqlite::Result<Option<DossierMeta>> {
    use rusqlite::OptionalExtension as _;
    let row: Option<(i64, String, String, String)> = conn
        .query_row(
            "SELECT id, name, country, region FROM authority
             WHERE slug = ?1 AND hidden_at IS NULL",
            [slug],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let Some((authority_id, name, country, region)) = row else {
        return Ok(None);
    };
    let place = match (region.trim(), country.trim()) {
        ("", "") => String::new(),
        ("", c) => c.to_string(),
        (r, "") => r.to_string(),
        (r, c) => format!("{r}, {c}"),
    };
    let mut stmt = conn.prepare(
        "SELECT id, topic, statement, capture_id, citation_start, citation_end
         FROM fact WHERE authority_id = ?1 AND status = 'accepted'
           AND topic != 'phone_script' ORDER BY id",
    )?;
    let heads: Vec<(i64, String, String, i64, i64, i64)> = stmt
        .query_map([authority_id], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();
    drop(stmt);
    let ids: Vec<i64> = heads.iter().map(|head| head.0).collect();
    let last_accept = crate::citation::accepted_at(conn, &ids)
        .into_values()
        .max()
        .map(notify::iso_day);
    let mut facts = Vec::new();
    let mut sources = Vec::new();
    let mut last_day: Option<String> = None;
    let mut usps = Vec::new();
    for (id, topic, statement, capture_id, start, end) in heads {
        let (source_url, captured_at, quote) = citation(conn, store_root, capture_id, start, end);
        if db::USPS_TOPICS.contains(&topic.as_str()) {
            // The public `/facts` flag (an accepted sibling disagrees);
            // a failed read counts as flagged, so no box ships unchecked.
            let disputed = crate::conflicts::disagreeing_siblings(
                conn,
                authority_id,
                &topic,
                &statement,
                id,
                true,
            )
            .map_or(true, |found| !found.is_empty());
            let mut text = statement.clone();
            if let Some(quote) = quote.as_deref().filter(|_| source_url.is_some()) {
                text.push('\n');
                text.push_str(quote);
            }
            usps.push(UspsFact {
                statement: statement.clone(),
                text: crate::orders::FactText {
                    id,
                    topic: topic.clone(),
                    text,
                    disputed,
                },
            });
        }
        if let Some(url) = source_url
            && !sources.contains(&url)
        {
            sources.push(url);
        }
        let day = (captured_at > 0).then(|| notify::iso_day(captured_at));
        if day.as_deref() > last_day.as_deref() {
            last_day = day.clone();
        }
        facts.push(MetaFact { topic, statement });
    }
    let path = crate::authority_path::index(conn).primary(slug);
    Ok(Some(DossierMeta {
        path,
        name,
        place,
        facts,
        sources,
        last_day,
        last_accept,
        postal: usps_postal(&usps),
    }))
}

/// One fact's source, day and quote, mirroring the A1 citation rules (a
/// sidecar slice of 8–400 chars must exist; https source, or the
/// official site for a source-less page or upload, per
/// [`crate::citation`]). The quote itself stays on the page, one tap
/// away — crawlers get statements plus source URLs; the head reads it
/// only for the USPS check, as orders does.
fn citation(
    conn: &rusqlite::Connection,
    store_root: &std::path::Path,
    capture_id: i64,
    start: i64,
    end: i64,
) -> (Option<String>, i64, Option<String>) {
    if capture_id <= 0 || start < 0 || end <= start {
        return (None, 0, None);
    }
    let bytes = std::fs::read(store_root.join("text").join(format!("{capture_id}.txt")))
        .unwrap_or_default();
    if start as usize >= bytes.len() || end as usize > bytes.len() {
        return (None, 0, None);
    }
    let quote = String::from_utf8_lossy(&bytes[start as usize..end as usize]).to_string();
    if !(8..=400).contains(&quote.chars().count()) {
        return (None, 0, None);
    }
    let Some(capture) = crate::citation::CitedCapture::load(conn, capture_id) else {
        return (None, 0, None);
    };
    (
        capture.source_url(conn, capture.authority_id),
        capture.captured_at,
        Some(quote),
    )
}

/// A `po_box` value that opens with a bare box number: `40234`,
/// `Box 40234`, `#40234`, `No. 40234` (then anything, such as a city).
static BARE_BOX: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)^(?:box\b\s*)?(?:(?:#|no\b\.?|number\b)\s*)?#?\s*([0-9]+)\b")
        .expect("bare box number compiles")
});

/// The box numbers `value` writes with the PO box keyword: every
/// `box:` token of the shared rule ([`db::value_tokens`]) and the
/// first box orders reads ([`crate::orders::po_box_in`]), leading zeros
/// dropped (`PO Box #040220` is `40220`, as orders reads it).
fn box_numbers(value: &str) -> BTreeSet<String> {
    let trim = |digits: &str| match digits.trim_start_matches('0') {
        "" => "0".to_string(),
        trimmed => trimmed.to_string(),
    };
    db::value_tokens(value)
        .iter()
        .filter_map(|token| token.strip_prefix("box:"))
        .chain(crate::orders::po_box_in(value).map(|(_, digits)| digits))
        .map(trim)
        .collect()
}

/// One USPS statement as the dispute check reads it: as written, except
/// a `po_box` value with no keyword box that opens with a bare number
/// or `Box N` ([`BARE_BOX`]), which reads as `PO Box N` (the topic names
/// the kind: `po_box: 40234` is box 40234). The shared rule reads such
/// a value as a ZIP or as plain text, so it never disputes another box
/// there; the head fails closed on it instead.
fn usps_reading(topic: &str, statement: &str) -> String {
    let (key, value) = db::split_statement(statement);
    let value = value.trim();
    if topic != "po_box" || !box_numbers(value).is_empty() {
        return statement.to_string();
    }
    let Some(bare) = BARE_BOX.captures(value) else {
        return statement.to_string();
    };
    let rest = bare.get(0).map_or("", |whole| &value[whole.end()..]);
    let read = format!("PO Box {}{rest}", &bare[1]);
    match key {
        Some(key) => format!("{key}: {read}"),
        None => read,
    }
}

/// The office's postal address for JSON-LD, from its one undisputed
/// USPS value. The USPS facts are the accepted `mailing`/`po_box` facts
/// that name a PO box as orders reads one
/// ([`crate::orders::names_po_box`]: the `po_box` topic, a `box:` token,
/// or a parsed box; never the keyword alone). There is no address while
/// they are disputed: any flagged in conflict on the public `/facts`
/// read; any that pauses orders' USPS labels
/// ([`crate::orders::destination`]: a flagged box, or parsed boxes or
/// ZIPs that differ); any two that disagree under the shared rule
/// ([`db::facts_disagree`]) with bare `po_box` numbers read as boxes
/// ([`usps_reading`]); or more than one box number among them.
///
/// Only a value that writes a numbered box goes out, key stripped
/// ([`db::split_statement`]): the address orders' label reader parses
/// from it ([`crate::label_autofill::address_in`]) as `street, City, ST
/// ZIP` when that street is the box itself, else the box as written. So
/// neither a raw `key: value`, a later keyed line nor a fact's
/// instruction words ever ship. Agreeing spellings are one box: a
/// parsed address first, `mailing` before `po_box` (as orders ships),
/// then by id.
fn usps_postal(usps: &[UspsFact]) -> Option<String> {
    let boxes: Vec<&UspsFact> = usps
        .iter()
        .filter(|f| crate::orders::names_po_box(&f.text))
        .collect();
    let readings: Vec<(&str, String)> = boxes
        .iter()
        .map(|f| {
            (
                f.text.topic.as_str(),
                usps_reading(&f.text.topic, &f.statement),
            )
        })
        .collect();
    let disagree = readings.iter().enumerate().any(|(i, (topic_a, a))| {
        readings[i + 1..]
            .iter()
            .any(|(topic_b, b)| db::facts_disagree(topic_a, a, topic_b, b))
    });
    let numbers: BTreeSet<String> = readings
        .iter()
        .flat_map(|(_, statement)| box_numbers(db::split_statement(statement).1))
        .collect();
    let texts: Vec<crate::orders::FactText> = usps.iter().map(|f| f.text.clone()).collect();
    let labels_paused = crate::orders::destination(&texts).is_some_and(|d| !d.disputed.is_empty());
    if disagree || numbers.len() > 1 || labels_paused || boxes.iter().any(|f| f.text.disputed) {
        return None;
    }
    boxes
        .iter()
        .filter_map(|f| {
            let value = db::split_statement(&f.statement).1.trim();
            let (written, _) = crate::orders::po_box_in(value)?;
            let parsed = crate::label_autofill::address_in(value)
                .map(|found| found.address)
                .filter(|address| {
                    crate::orders::po_box_in(&address.street)
                        .is_some_and(|(span, _)| span == (0..address.street.len()))
                });
            Some((f, parsed, &value[written]))
        })
        .min_by_key(|(f, parsed, _)| (parsed.is_none(), f.text.topic != "mailing", f.text.id))
        .map(|(_, parsed, written)| match parsed {
            Some(a) => format!("{}, {}, {} {}", a.street, a.city, a.state, a.zip),
            None => written.to_string(),
        })
}

/// A statement-grammar key opening a line (`saturday_hours: closed`, or
/// `note:` with nothing after it). The class is JavaScript's `\s` (the
/// guide's `parseStatement`): Rust's `\s` plus U+FEFF.
static LINE_KEY: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"^([a-z][a-z0-9_]*):(?:[\s\x{FEFF}]|$)").expect("line key compiles")
});

/// A key's readable label, as [`topic_label`] reads a topic
/// (`saturday_hours` → `Saturday hours`; an instance suffix
/// `weekday__tumwater` → `Weekday tumwater`).
fn key_label(key: &str) -> String {
    let words: Vec<&str> = key.split('_').filter(|w| !w.is_empty()).collect();
    topic_label(&words.join("_"))
}

/// A statement's value as the head lists it: the key stripped
/// ([`db::split_statement`]). The drafter writes one `field: value`; a
/// statement edited into several keyed lines shows each later key as
/// its label ([`key_label`]), so no raw key reaches the head.
fn head_value(statement: &str) -> String {
    db::split_statement(statement)
        .1
        .trim()
        .lines()
        .map(|line| {
            let body = line.trim_start();
            match LINE_KEY.captures(body).and_then(|c| c.get(1)) {
                Some(key) => format!("{}{}", key_label(key.as_str()), &body[key.end()..]),
                None => line.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A topic's readable label, as the guide's chips name it (`topicName`
/// in `web/src/lib/public-ui/plan.ts`): the frozen vocabulary's own
/// spellings, else the topic humanized (`processing_time` →
/// `Processing time`).
fn topic_label(topic: &str) -> String {
    match topic {
        "walkin" => "Walk-in".to_string(),
        "po_box" => "PO Box".to_string(),
        "id_requirements" => "ID requirements".to_string(),
        _ => {
            let human = topic.replace('_', " ");
            let mut chars = human.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        }
    }
}

/// JSON-LD `GovernmentService` for one dossier. The serialized
/// form escapes `<`, `>` and `&` as unicode (`\u003c` …) so a
/// hostile statement can never break out of the script block or
/// smuggle entities (unit-tested with `</script>` input).
fn json_ld(meta: &DossierMeta, page_url: &str) -> String {
    let mut service = json!({
        "@context": "https://schema.org",
        "@type": "GovernmentService",
        "name": format!("Apostille — {}", meta.name),
        "url": page_url,
        "provider": {
            "@type": "GovernmentOrganization",
            "name": meta.name,
        },
    });
    if !meta.place.is_empty() {
        service["areaServed"] = json!(meta.place);
    }
    let mut channel = json!({
        "@type": "ServiceChannel",
        "serviceUrl": page_url,
    });
    if let Some(postal) = &meta.postal {
        channel["servicePostalAddress"] = json!(postal);
    }
    service["hasOfferCatalog"] = json!({
        "@type": "OfferCatalog",
        "name": "Apostille services",
        "itemListElement": [{
            "@type": "Offer",
            "itemOffered": {
                "@type": "Service",
                "serviceType": "Apostille",
                "availableChannel": channel,
            },
        }],
    });
    // One update day in the head: the description's newest staff
    // accept; with no accept recorded (the description then names no
    // day), the newest capture day.
    if let Some(day) = meta.last_accept.as_ref().or(meta.last_day.as_ref()) {
        service["dateModified"] = json!(day);
    }
    if !meta.sources.is_empty() {
        service["isBasedOn"] = json!(meta.sources);
    }
    serde_json::to_string(&service)
        .unwrap_or_else(|_| "{}".to_string())
        .replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
}

/// Shell anchors from `web/src/app.html`, byte for byte (pinned by
/// `shell_anchors_are_app_html_bytes` below, failing closed on drift).
pub(crate) const GEN_DESCRIPTION: &str = "<meta name=\"description\" content=\"Apostille Handbook — verified apostille guidance, quote by quote.\" />";
const GEN_OG_TITLE: &str = "<meta property=\"og:title\" content=\"Apostille Handbook\" />";
const GEN_OG_DESCRIPTION: &str = "<meta\n\t\t\tproperty=\"og:description\"\n\t\t\tcontent=\"Verified apostille guidance, quote by quote.\"\n\t\t/>";

/// Inject one dossier's head into the SPA shell. Pure
/// (unit-tested, including hostile statements).
pub fn render_dossier(shell: &str, meta: &DossierMeta, base: &str) -> String {
    let base = base.trim_end_matches('/');
    let page_url = format!("{base}/authority/{}", meta.path);
    // Only what `/facts` serves: `phone_script` stays staff-only.
    let listed: Vec<&MetaFact> = meta
        .facts
        .iter()
        .filter(|fact| fact.topic != "phone_script")
        .collect();
    let place = if meta.place.is_empty() {
        String::new()
    } else {
        format!(" ({})", meta.place)
    };
    let n = listed.len();
    let noun = if n == 1 { "fact" } else { "facts" };
    // Staff approve facts; the date is the newest accept, never a
    // capture day, and none is made up when no accept is recorded.
    let description = match &meta.last_accept {
        _ if n == 0 => format!(
            "Authority dossier for {}{} — facts pending staff verification.",
            meta.name, place
        ),
        Some(day) => format!(
            "{n} staff-approved apostille {noun} for {}{}, last updated {day}.",
            meta.name, place
        ),
        None => format!(
            "{n} staff-approved apostille {noun} for {}{}.",
            meta.name, place
        ),
    };
    let mut page = shell.to_string();
    let desc_tag = format!(
        "<meta name=\"description\" content=\"{}\" />",
        esc(&description)
    );
    if page.contains(GEN_DESCRIPTION) {
        page = page.replacen(GEN_DESCRIPTION, &desc_tag, 1);
    } else {
        page = page.replacen("<head>", &format!("<head>\n\t{desc_tag}"), 1);
    }
    let og_title = format!(
        "<meta property=\"og:title\" content=\"{}\" />",
        esc(&format!("{} — Apostille Handbook", meta.name))
    );
    if page.contains(GEN_OG_TITLE) {
        page = page.replacen(GEN_OG_TITLE, &og_title, 1);
    }
    let og_desc = format!(
        "<meta property=\"og:description\" content=\"{}\" />",
        esc(&description)
    );
    if page.contains(GEN_OG_DESCRIPTION) {
        page = page.replacen(GEN_OG_DESCRIPTION, &og_desc, 1);
    } else {
        page = page.replacen("<head>", &format!("<head>\n\t{og_desc}"), 1);
    }
    let head_extra = format!(
        "<title>{}</title>\n\t<link rel=\"canonical\" href=\"{}\" />\n\t\
         <meta property=\"og:url\" content=\"{}\" />\n\t\
         <script type=\"application/ld+json\">{}</script>",
        esc(&format!("{} — Apostille Handbook", meta.name)),
        esc(&page_url),
        esc(&page_url),
        json_ld(meta, &page_url),
    );
    page = page.replacen("<head>", &format!("<head>\n\t{head_extra}"), 1);
    // Crawler summary: every accepted fact as `{topic label}: {value}`,
    // its key stripped ([`head_value`]), so no raw `key:` text reaches
    // the head.
    let mut items = String::new();
    for fact in listed {
        items.push_str(&format!(
            "<li>{}: {}</li>",
            esc(&topic_label(&fact.topic)),
            esc(&head_value(&fact.statement))
        ));
    }
    let noscript = format!(
        "<noscript><div><h2>{} — key facts</h2><ul>{}</ul></div></noscript>",
        esc(&meta.name),
        items
    );
    if let Some(body) = page.find("<body")
        && let Some(end) = page[body..].find('>')
    {
        let at = body + end + 1;
        page.insert_str(at, &noscript);
    }
    page
}

/// `GET /apostille/authority/{*trail}` — shell with injected head for
/// any address of a dossier (slug, `usa/washington`, `us/wa`, a county
/// path), canonical link to its primary path; 404 status (same shell)
/// for an address nothing answers to.
pub async fn dossier(State(state): State<AppState>, Path(trail): Path<String>) -> Response {
    let shell = tokio::fs::read(&state.spa_index).await.unwrap_or_default();
    let shell = String::from_utf8_lossy(&shell).to_string();
    let Some(conn) = passkey::open_conn(&state) else {
        return store_unavailable();
    };
    let found = crate::authority_path::resolve(&conn, &trail);
    // A place (`usa`, P2-5) answers with its own head; the client
    // shows the place page.
    if found.is_none()
        && let Some(loc) = crate::locations::locate(&conn, &trail)
    {
        let count: usize = crate::locations::offices(&conn, &loc)
            .iter()
            .map(|g| g.offices.len())
            .sum();
        return (
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            crate::locations::render_location(&shell, &loc, count, &state.notify_base),
        )
            .into_response()
            .with_cache();
    }
    let slug = found.map_or_else(|| trail.trim().to_string(), |found| found.slug);
    let meta = load_dossier(&conn, &state.store_root, &slug).unwrap_or(None);
    match meta {
        Some(meta) => (
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            render_dossier(&shell, &meta, &state.notify_base),
        )
            .into_response()
            .with_cache(),
        None => (StatusCode::NOT_FOUND, serve_shell(&shell))
            .into_response()
            .with_cache(),
    }
}

/// Dossier heads change only on staff accept (minutes to days):
/// edge-cached briefly, always revalidated by the client app.
trait CachedResponse: IntoResponse + Sized {
    fn with_cache(self) -> Response {
        let mut res = self.into_response();
        res.headers_mut().insert(
            header::CACHE_CONTROL,
            header::HeaderValue::from_static("public, max-age=300"),
        );
        res
    }
}

impl CachedResponse for Response {}

fn serve_shell(shell: &str) -> Response {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        shell.to_string(),
    )
        .into_response()
}

/// One sitemap row: slug plus newest accepted capture day, if any.
pub fn build_sitemap(base: &str, rows: &[(String, Option<i64>)], home_day: Option<i64>) -> String {
    fn xml(raw: &str) -> String {
        raw.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }
    let base = base.trim_end_matches('/');
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );
    match home_day {
        Some(day) => out.push_str(&format!(
            "  <url><loc>{}/</loc><lastmod>{}</lastmod></url>\n",
            xml(base),
            notify::iso_day(day)
        )),
        None => out.push_str(&format!("  <url><loc>{}/</loc></url>\n", xml(base))),
    }
    for (slug, day) in rows {
        match day {
            Some(day) => out.push_str(&format!(
                "  <url><loc>{}/authority/{}</loc><lastmod>{}</lastmod></url>\n",
                xml(base),
                xml(slug),
                notify::iso_day(*day)
            )),
            None => out.push_str(&format!(
                "  <url><loc>{}/authority/{}</loc></url>\n",
                xml(base),
                xml(slug)
            )),
        }
    }
    out.push_str("</urlset>\n");
    out
}

/// `GET /apostille/sitemap.xml` — authorities with lastmod from
/// the newest accepted capture (unknown → no lastmod, still listed).
pub async fn sitemap(State(state): State<AppState>) -> Response {
    let Some(conn) = passkey::open_conn(&state) else {
        return store_unavailable();
    };
    let mut stmt = match conn.prepare(
        "SELECT a.slug, MAX(c.captured_at) FROM authority a
         LEFT JOIN fact f ON f.authority_id = a.id AND f.status = 'accepted'
         LEFT JOIN capture c ON c.id = f.capture_id
         WHERE a.id != 0 AND a.slug NOT LIKE 'sentinel-%' AND a.hidden_at IS NULL
         GROUP BY a.id ORDER BY a.slug",
    ) {
        Ok(stmt) => stmt,
        Err(_) => return store_unavailable(),
    };
    let rows: Vec<(String, Option<i64>)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default();
    // One canonical address per authority (its readable path).
    let paths = crate::authority_path::index(&conn);
    let rows: Vec<(String, Option<i64>)> = rows
        .into_iter()
        .map(|(slug, day)| (paths.primary(&slug), day))
        .collect();
    let home_day = rows.iter().filter_map(|(_, day)| *day).max();
    (
        [(header::CONTENT_TYPE, "application/xml; charset=utf-8")],
        build_sitemap(&state.notify_base, &rows, home_day),
    )
        .into_response()
        .with_cache()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> DossierMeta {
        DossierMeta {
            path: "test-sos".to_string(),
            name: "Test \"SOS\" & Co".to_string(),
            place: "WA, USA".to_string(),
            facts: vec![MetaFact {
                topic: "hours".to_string(),
                statement: "weekday: open 9-5 </script><img src=x onerror=alert(1)>".to_string(),
            }],
            sources: vec!["https://example.test/hours".to_string()],
            last_day: Some("2026-10-31".to_string()),
            last_accept: Some("2026-11-04".to_string()),
            postal: Some("PO Box 1 </script> Town".to_string()),
        }
    }

    const SHELL: &str = "<html><head>\n\t<meta name=\"description\" content=\"Apostille Handbook — verified apostille guidance, quote by quote.\" />\n\t<meta property=\"og:title\" content=\"Apostille Handbook\" />\n\t<meta\n\t\t\tproperty=\"og:description\"\n\t\t\tcontent=\"Verified apostille guidance, quote by quote.\"\n\t\t/>\n</head><body data-x=\"1\"><div id=\"app\"></div></body></html>";

    /// No statement key anywhere in `page`: none of `keys` as `key: `,
    /// and no snake_case `key: ` at all (the page's e2e pattern, plan §2).
    fn assert_no_raw_keys(page: &str, keys: &[&str]) {
        let raw = regex::Regex::new(r"\b[a-z][a-z0-9]*_[a-z0-9_]+:\s").unwrap();
        assert_eq!(raw.find(page).map(|m| m.as_str()), None, "raw key");
        for key in keys {
            assert!(!page.contains(&format!("{key}: ")), "raw `{key}: `");
        }
    }

    #[test]
    fn injection_escapes_hostile_statements() {
        let page = render_dossier(SHELL, &meta(), "https://example.test/apostille");
        assert!(page.contains("&lt;/script&gt;"), "statement escaped");
        assert!(
            page.contains("Test &quot;SOS&quot; &amp; Co"),
            "name escaped"
        );
        assert!(!page.contains("</script><img"), "no raw breakout");
        assert!(!page.contains("</script>\""), "json-ld cannot close early");
        assert!(
            page.contains("PO Box 1 \\u003c/script\\u003e Town"),
            "ld+json unicode-escaped"
        );
        assert!(
            page.contains("\\u0026 Co"),
            "ampersand unicode-escaped in json-ld"
        );
        assert!(
            page.contains("<link rel=\"canonical\" href=\"https://example.test/apostille/authority/test-sos\" />"),
            "canonical present"
        );
        assert!(page.contains("\"isBasedOn\":[\"https://example.test/hours\"]"));
        assert!(page.contains("servicePostalAddress\":\"PO Box 1 \\u003c/script\\u003e Town\""));
        assert!(page.contains("<noscript>"), "crawler summary present");
        assert!(
            page.contains(
                "<li>Hours: open 9-5 &lt;/script&gt;&lt;img src=x onerror=alert(1)&gt;</li>"
            ),
            "fact listed as label and value"
        );
        assert_no_raw_keys(&page, &["weekday"]);
    }

    #[test]
    fn injection_replaces_generic_tags() {
        let page = render_dossier(SHELL, &meta(), "https://example.test/apostille");
        assert!(
            !page.contains("Verified apostille guidance, quote by quote."),
            "generic copy gone"
        );
        let description = "1 staff-approved apostille fact for Test &quot;SOS&quot; &amp; Co \
                           (WA, USA), last updated 2026-11-04.";
        assert!(
            page.contains(&format!(
                "<meta name=\"description\" content=\"{description}\" />"
            )),
            "count + accept-day copy"
        );
        assert!(
            page.contains(&format!(
                "<meta property=\"og:description\" content=\"{description}\" />"
            )),
            "og copy matches"
        );
        assert!(
            !page.contains("verified"),
            "staff approve; nothing reads verified"
        );
        // Accepted facts with no recorded accept day: no date is made up.
        let mut undated = meta();
        undated.last_accept = None;
        let page = render_dossier(SHELL, &undated, "https://example.test/apostille");
        assert!(page.contains(
            "1 staff-approved apostille fact for Test &quot;SOS&quot; &amp; Co (WA, USA).\""
        ));
        let mut empty = meta();
        empty.facts.clear();
        let page = render_dossier(SHELL, &empty, "https://example.test/apostille");
        assert!(
            page.contains("— facts pending staff verification."),
            "empty dossier"
        );
    }

    /// The shell anchors are `web/src/app.html`'s own bytes: any drift
    /// there would leave the generic copy in every dossier head.
    #[test]
    fn shell_anchors_are_app_html_bytes() {
        let app = include_str!("../../../web/src/app.html");
        for anchor in [GEN_DESCRIPTION, GEN_OG_TITLE, GEN_OG_DESCRIPTION] {
            assert_eq!(
                app.matches(anchor).count(),
                1,
                "app.html carries {anchor:?}"
            );
        }
        let page = render_dossier(app, &meta(), "https://example.test/apostille");
        for anchor in [GEN_DESCRIPTION, GEN_OG_TITLE, GEN_OG_DESCRIPTION] {
            assert!(!page.contains(anchor), "replaced in place: {anchor:?}");
        }
        for tag in [
            "<meta name=\"description\"",
            "<meta property=\"og:title\"",
            "<meta property=\"og:description\"",
            "<noscript>",
        ] {
            assert_eq!(page.matches(tag).count(), 1, "one {tag}");
        }
    }

    /// The noscript list reads `{topic label}: {value}`, keys stripped,
    /// and holds only what `/facts` serves (no `phone_script`).
    #[test]
    fn noscript_lists_labels_and_values_without_keys() {
        let mut meta = meta();
        meta.facts = [
            ("hours", "weekday: Monday-Friday 8:00 a.m. to 4:30 p.m."),
            ("fees", "per_document: $15 per apostille"),
            ("po_box", "po_box: PO Box 40220"),
            ("processing_time", "standard: 7 to 10 business days"),
            ("walkin", "Walk-in service ends at 4 p.m."),
            ("id_requirements", "id_document: A government photo ID"),
            (
                "address",
                "apostille_main_branch_street__tumwater: 243 Israel Rd SE",
            ),
            ("phone_script", "say hello, then ask"),
        ]
        .into_iter()
        .map(|(topic, statement)| MetaFact {
            topic: topic.to_string(),
            statement: statement.to_string(),
        })
        .collect();
        let page = render_dossier(SHELL, &meta, "https://example.test/apostille");
        for item in [
            "<li>Hours: Monday-Friday 8:00 a.m. to 4:30 p.m.</li>",
            "<li>Fees: $15 per apostille</li>",
            "<li>PO Box: PO Box 40220</li>",
            "<li>Processing time: 7 to 10 business days</li>",
            "<li>Walk-in: Walk-in service ends at 4 p.m.</li>",
            "<li>ID requirements: A government photo ID</li>",
            "<li>Address: 243 Israel Rd SE</li>",
        ] {
            assert!(page.contains(item), "{item}");
        }
        assert!(!page.contains("say hello"), "phone_script stays staff-only");
        assert!(
            page.contains("7 staff-approved apostille facts"),
            "phone_script uncounted"
        );
        assert_no_raw_keys(
            &page,
            &[
                "weekday",
                "per_document",
                "po_box",
                "standard",
                "id_document",
                "apostille_main_branch_street__tumwater",
            ],
        );
    }

    /// One accepted USPS-topic fact as `load_dossier` hands it over.
    fn usps(id: i64, topic: &str, statement: &str, quote: &str) -> UspsFact {
        let mut text = statement.to_string();
        if !quote.is_empty() {
            text.push('\n');
            text.push_str(quote);
        }
        UspsFact {
            statement: statement.to_string(),
            text: crate::orders::FactText {
                id,
                topic: topic.to_string(),
                text,
                disputed: false,
            },
        }
    }

    /// The JSON-LD address is one undisputed USPS value, key stripped,
    /// and nothing while the office's own pages disagree on the box.
    #[test]
    fn postal_is_only_an_undisputed_usps_value() {
        const WA: &str = "apostille_mailing_address: PO BOX 40228 Olympia, WA 98504-0228";
        let wa = || usps(2, "mailing", WA, "PO BOX 40228 Olympia, WA 98504-0228");
        // One keyed USPS fact: its address as orders parses it, never
        // `key: value`.
        assert_eq!(
            usps_postal(&[wa()]).as_deref(),
            Some("PO Box 40228, Olympia, WA 98504-0228")
        );
        // Two boxes that disagree under the shared rule: no address.
        let other = usps(3, "po_box", "po_box: PO Box 40234", "PO Box 40234");
        assert!(db::facts_disagree(
            "mailing",
            WA,
            "po_box",
            "po_box: PO Box 40234"
        ));
        assert_eq!(usps_postal(&[wa(), other]), None);
        // A fact `/facts` banners as in conflict: no address either.
        let mut flagged = wa();
        flagged.text.disputed = true;
        assert_eq!(usps_postal(&[flagged]), None);
        // The shared rule lets these agree (one statement has no ZIP),
        // but the quote's ZIP+4 is the other box's: orders pauses USPS
        // labels, so the head names no box.
        let quoted = usps(
            21,
            "po_box",
            "po_box: PO Box 40220",
            "PO Box 40220\nOlympia, WA 98504-0220",
        );
        let mail = "apostille_mailing_address: PO Box 40220 Olympia, WA 98504-0234";
        assert!(!db::facts_disagree(
            "po_box",
            "po_box: PO Box 40220",
            "mailing",
            mail
        ));
        assert_eq!(usps_postal(&[quoted, usps(22, "mailing", mail, "")]), None);
        // Spellings of one box agree: the parsed mailing address is used.
        let one_box = [
            usps(1, "po_box", "po_box: PO Box 40220", ""),
            usps(
                2,
                "mailing",
                "apostille_mailing_address: PO BOX 40220 Olympia, WA 98504-0220",
                "",
            ),
            usps(25, "po_box", "P.O. Box 40220, Olympia, WA 98504-0220.", ""),
        ];
        assert_eq!(
            usps_postal(&one_box).as_deref(),
            Some("PO Box 40220, Olympia, WA 98504-0220")
        );
        // No box named, so no USPS value: a mailing instruction, a
        // street (the courier route), a bare number under `po_box`.
        for (topic, statement) in [
            (
                "mailing",
                "submission_requirement: Send all documents with Request form and payment:",
            ),
            ("mailing", "Mail to 801 Capitol Way S, Olympia, WA 98501."),
            ("po_box", "po_box: 40220"),
        ] {
            assert_eq!(
                usps_postal(&[usps(9, topic, statement, "")]),
                None,
                "{statement}"
            );
        }
        assert_eq!(usps_postal(&[]), None);
    }

    /// The PO box keyword alone names no box (orders never reads one as
    /// a box either): a mailing note that only mentions the PO box is
    /// never the address, alone or next to the box it talks about.
    #[test]
    fn postal_never_comes_from_a_box_mention() {
        let short_box = || usps(6, "po_box", "po_box: PO Box 40228", "");
        for note in [
            "courier_note: FedEx and UPS cannot deliver to a PO Box.",
            "submission_requirement: Do not send documents to the PO Box by courier; \
             use the Tumwater street address.",
        ] {
            assert_eq!(usps_postal(&[usps(5, "mailing", note, "")]), None, "{note}");
            assert_eq!(
                usps_postal(&[usps(5, "mailing", note, ""), short_box()]).as_deref(),
                Some("PO Box 40228"),
                "{note}"
            );
            let full = usps(
                7,
                "mailing",
                "apostille_mailing_address: PO BOX 40228 Olympia, WA 98504-0228",
                "",
            );
            assert_eq!(
                usps_postal(&[usps(5, "mailing", note, ""), full]).as_deref(),
                Some("PO Box 40228, Olympia, WA 98504-0228"),
                "{note}"
            );
        }
    }

    /// The address goes out as orders parses it for USPS labels
    /// (`street, City, ST ZIP`), never with a fact's instruction words;
    /// a box with no city, state and ZIP to read goes out as written.
    #[test]
    fn postal_is_the_parsed_address_or_the_box_as_written() {
        for (statement, postal) in [
            // An unkeyed legacy pin (orders' fixture shape).
            (
                "Mail requests to PO Box 40220, Olympia, WA 98504-0220.",
                "PO Box 40220, Olympia, WA 98504-0220",
            ),
            (
                "P.O. Box 40220, Olympia, WA 98504-0220.",
                "P.O. Box 40220, Olympia, WA 98504-0220",
            ),
            // A quote-style line break between the box and the city.
            (
                "apostille_mailing_address: PO BOX 40228\nOlympia, WA 98504-0228",
                "PO Box 40228, Olympia, WA 98504-0228",
            ),
            // Nothing to parse: the box itself, without the sentence.
            ("Mail requests to PO Box 40220.", "PO Box 40220"),
            ("po_box: PO Box #40220", "PO Box #40220"),
            // A street parsed around the box is not the box: the box alone.
            (
                "mailing_address: PO Box 40228 or 801 Capitol Way S, Olympia, WA 98501",
                "PO Box 40228",
            ),
            // A later keyed line never rides along.
            ("po_box: PO Box 40228\nzip_code: 98504-0228", "PO Box 40228"),
        ] {
            assert_eq!(
                usps_postal(&[usps(23, "po_box", statement, "")]).as_deref(),
                Some(postal),
                "{statement}"
            );
        }
    }

    /// The topic names the kind: a `po_box` value written as a bare
    /// number or `Box N` is that box to the head, so another number
    /// disputes the mailing box (the shared rule reads it as a ZIP or as
    /// text, and finds no dispute), and the same number agrees.
    #[test]
    fn postal_reads_a_bare_po_box_number_as_that_box() {
        let wa = || {
            usps(
                2,
                "mailing",
                "apostille_mailing_address: PO BOX 40228 Olympia, WA 98504-0228",
                "",
            )
        };
        for other in [
            "po_box: 40234",
            "po_box: Box 40234",
            "po_box: #40234",
            "po_box: No. 40234",
            "40234",
            // The same box with another ZIP+4 (the June 2025 form's).
            "po_box: 40228, Olympia, WA 98504-0234",
        ] {
            assert!(
                !db::facts_disagree("mailing", &wa().statement, "po_box", other),
                "the shared rule alone misses {other}"
            );
            assert_eq!(
                usps_postal(&[wa(), usps(3, "po_box", other, "")]),
                None,
                "{other}"
            );
        }
        for same in ["po_box: 40228", "po_box: Box 40228", "po_box: #40228"] {
            assert_eq!(
                usps_postal(&[wa(), usps(3, "po_box", same, "")]).as_deref(),
                Some("PO Box 40228, Olympia, WA 98504-0228"),
                "{same}"
            );
        }
        // A bare number names no address of its own.
        assert_eq!(
            usps_postal(&[usps(3, "po_box", "po_box: Box 40228", "")]),
            None
        );
        // One value naming two boxes is not one box.
        assert_eq!(
            usps_postal(&[usps(
                4,
                "mailing",
                "apostille_mailing_address: PO Box 40228 or PO Box 40234, Olympia, WA 98504",
                ""
            )]),
            None
        );
    }

    /// The head states one update day: JSON-LD `dateModified` is the
    /// description's newest accept, else the newest capture day (when
    /// the description names no day), else nothing.
    #[test]
    fn json_ld_date_modified_is_the_described_day() {
        let page = render_dossier(SHELL, &meta(), "https://example.test/apostille");
        assert!(page.contains("last updated 2026-11-04."), "described day");
        assert!(page.contains("\"dateModified\":\"2026-11-04\""), "{page}");
        assert!(!page.contains("2026-10-31"), "no second day");
        let mut undated = meta();
        undated.last_accept = None;
        let page = render_dossier(SHELL, &undated, "https://example.test/apostille");
        assert!(!page.contains("last updated"), "no day described");
        assert!(page.contains("\"dateModified\":\"2026-10-31\""), "{page}");
        undated.last_day = None;
        let page = render_dossier(SHELL, &undated, "https://example.test/apostille");
        assert!(!page.contains("dateModified"), "no day at all");
    }

    /// The drafter writes one `field: value`; a statement edited into
    /// several keyed lines shows each later key as its label, so no raw
    /// key reaches the head.
    #[test]
    fn noscript_labels_later_keys_of_a_multi_key_statement() {
        let mut meta = meta();
        meta.facts = vec![MetaFact {
            topic: "hours".to_string(),
            statement: "weekday: Mon-Fri 8-4:30\nsaturday_hours: closed\nholiday_note:".to_string(),
        }];
        let page = render_dossier(SHELL, &meta, "https://example.test/apostille");
        assert!(
            page.contains("<li>Hours: Mon-Fri 8-4:30\nSaturday hours: closed\nHoliday note:</li>"),
            "{page}"
        );
        assert_no_raw_keys(&page, &["weekday", "saturday_hours", "holiday_note"]);
    }

    #[test]
    fn sitemap_lists_authorities_with_lastmod() {
        let xml = build_sitemap(
            "https://example.test/apostille",
            &[
                ("vietnam".to_string(), Some(1_791_000_000)),
                ("new-sos".to_string(), None),
                ("a&b<script>".to_string(), Some(1_791_000_000)),
            ],
            Some(1_791_000_000),
        );
        assert!(xml.starts_with("<?xml"), "declaration first");
        assert!(
            xml.contains("<loc>https://example.test/apostille/authority/vietnam</loc><lastmod>"),
            "lastmod present"
        );
        assert!(
            xml.contains("/authority/new-sos</loc></url>"),
            "no-day row still listed"
        );
        assert!(!xml.contains("<script>"), "slug escaped");
        assert!(xml.ends_with("</urlset>\n"), "closed urlset");
    }
}
