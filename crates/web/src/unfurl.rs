//! Social unfurl + crawler surface for dossier pages.
//!
//! `GET /apostille/authority/{slug}` serves the SPA shell with an
//! injected head: HTML-escaped `<title>`, meta description,
//! canonical, `og:*` tags, a `<noscript>` guide summary built from
//! accepted facts, and JSON-LD `GovernmentService`. Unknown slugs
//! get the same shell with a 404 status (client routing shows its
//! own notice). `GET /apostille/sitemap.xml` lists authorities with
//! `lastmod` from the newest accepted capture. Every human-visible
//! string comes from staff-accepted rows — no generated text.

use axum::{
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
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
    /// Newest capture day, if any fact carries one.
    pub last_day: Option<String>,
    /// Authority postal address (accepted `address`/`mailing` fact),
    /// for the JSON-LD service channel.
    pub postal: Option<String>,
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
        "SELECT topic, statement, capture_id, citation_start, citation_end
         FROM fact WHERE authority_id = ?1 AND status = 'accepted'
           AND topic != 'phone_script' ORDER BY id",
    )?;
    let heads: Vec<(String, String, i64, i64, i64)> = stmt
        .query_map([authority_id], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();
    drop(stmt);
    let mut facts = Vec::new();
    let mut sources = Vec::new();
    let mut last_day: Option<String> = None;
    let mut postal: Option<String> = None;
    for (topic, statement, capture_id, start, end) in heads {
        if (topic == "address" || topic == "mailing") && postal.is_none() {
            postal = Some(statement.clone());
        }
        let (source_url, captured_at) = citation(conn, store_root, capture_id, start, end);
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
        postal,
    }))
}

/// One fact's source/day, mirroring the A1 citation rules (a
/// sidecar slice of 8–400 chars must exist; https source, or the
/// official site for a source-less page or upload, per
/// [`crate::citation`]). The quote itself stays on the page, one tap
/// away — crawlers get statements plus source URLs.
fn citation(
    conn: &rusqlite::Connection,
    store_root: &std::path::Path,
    capture_id: i64,
    start: i64,
    end: i64,
) -> (Option<String>, i64) {
    if capture_id <= 0 || start < 0 || end <= start {
        return (None, 0);
    }
    let bytes = std::fs::read(store_root.join("text").join(format!("{capture_id}.txt")))
        .unwrap_or_default();
    if start as usize >= bytes.len() || end as usize > bytes.len() {
        return (None, 0);
    }
    let quote = String::from_utf8_lossy(&bytes[start as usize..end as usize]);
    if !(8..=400).contains(&quote.chars().count()) {
        return (None, 0);
    }
    let Some(capture) = crate::citation::CitedCapture::load(conn, capture_id) else {
        return (None, 0);
    };
    (
        capture.source_url(conn, capture.authority_id),
        capture.captured_at,
    )
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
    if let Some(day) = &meta.last_day {
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

/// Shell anchors from `web/src/app.html` (kept in sync by the
/// unknown-shell test below failing closed on drift).
pub(crate) const GEN_DESCRIPTION: &str = "<meta name=\"description\" content=\"Apostille Handbook — verified apostille guidance, quote by quote.\" />";
const GEN_OG_TITLE: &str = "<meta property=\"og:title\" content=\"Apostille Handbook\" />";
const GEN_OG_DESCRIPTION: &str = "<meta\n\t\t\tproperty=\"og:description\"\n\t\t\tcontent=\"Verified apostille guidance, quote by quote.\"\n\t\t/>";

/// Inject one dossier's head into the SPA shell. Pure
/// (unit-tested, including hostile statements).
pub fn render_dossier(shell: &str, meta: &DossierMeta, base: &str) -> String {
    let base = base.trim_end_matches('/');
    let page_url = format!("{base}/authority/{}", meta.path);
    let count = meta.facts.len();
    let place = if meta.place.is_empty() {
        String::new()
    } else {
        format!(" ({})", meta.place)
    };
    let description = match (&meta.last_day, count) {
        (Some(day), n) => {
            format!(
                "{n} staff-verified apostille facts for {}{}, last verified {}.",
                meta.name, place, day
            )
        }
        (None, _) => format!(
            "Authority dossier for {}{} — facts pending staff verification.",
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
    // Crawler summary: every accepted statement, plain list.
    let mut items = String::new();
    for fact in &meta.facts {
        items.push_str(&format!(
            "<li>{}: {}</li>",
            esc(&fact.topic),
            esc(&fact.statement)
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
                statement: "open 9-5 </script><img src=x onerror=alert(1)>".to_string(),
            }],
            sources: vec!["https://example.test/hours".to_string()],
            last_day: Some("2026-10-31".to_string()),
            postal: Some("Box 1 </script> Town".to_string()),
        }
    }

    const SHELL: &str = "<html><head>\n\t<meta name=\"description\" content=\"Apostille Handbook — verified apostille guidance, quote by quote.\" />\n\t<meta property=\"og:title\" content=\"Apostille Handbook\" />\n\t<meta\n\t\t\tproperty=\"og:description\"\n\t\t\tcontent=\"Verified apostille guidance, quote by quote.\"\n\t\t/>\n</head><body data-x=\"1\"><div id=\"app\"></div></body></html>";

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
            page.contains("Box 1 \\u003c/script\\u003e Town"),
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
        assert!(page.contains("servicePostalAddress\":\"Box 1 \\u003c/script\\u003e Town\""));
        assert!(page.contains("<noscript>"), "crawler summary present");
        assert!(
            page.contains("<li>hours: open 9-5 &lt;/script&gt;"),
            "fact listed"
        );
    }

    #[test]
    fn injection_replaces_generic_tags() {
        let page = render_dossier(SHELL, &meta(), "https://example.test/apostille");
        assert!(
            !page.contains("Verified apostille guidance, quote by quote."),
            "generic copy gone"
        );
        assert!(
            page.contains("1 staff-verified apostille facts"),
            "count + date copy"
        );
        assert!(page.contains("last verified 2026-10-31"), "date copy");
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
