//! Citation provenance for the public fact reads (plan §5.2), shared by
//! `/facts` ([`approval`](crate::approval)) and the crawler head
//! ([`unfurl`](crate::unfurl)) so the two never disagree:
//!
//! - the https URL a capture's quotes are attributed to: the capture's
//!   own source, else the authority's official website, but only for a
//!   source-less fetched page or staff upload. Transcripts, comments,
//!   emails and media never borrow that URL, and a `source_id` whose
//!   row is gone (an ops script deletes source rows with foreign keys
//!   off) cites nothing, so no quote is misattributed to the office.
//!   The rule itself is [`handbook_store::citation`], which the
//!   follower emails (crates/notify) share;
//! - what kind of record the citation is (`web`, `pdf` or `upload`);
//! - when staff accepted each fact (`accepted_at`).

use std::collections::HashMap;
use std::path::Path;

use handbook_store::citation::Origin;
use rusqlite::{Connection, OptionalExtension as _};

/// One capture row as the citation rules read it.
#[derive(Debug, Clone)]
pub struct CitedCapture {
    pub authority_id: i64,
    pub captured_at: i64,
    /// `capture.kind`: `warc`, `file`, `transcript`, ...
    pub kind: String,
    /// Blob path under the store root (the PDF sniff reads its head).
    path: String,
    origin: Origin,
}

impl CitedCapture {
    /// The capture row, or `None` when it is missing or unreadable
    /// (nothing to cite).
    pub fn load(conn: &Connection, capture_id: i64) -> Option<Self> {
        conn.query_row(
            "SELECT c.authority_id, c.captured_at, c.kind, c.path, c.source_id, s.uri
             FROM capture c LEFT JOIN source s ON s.id = c.source_id
             WHERE c.id = ?1",
            [capture_id],
            |row| {
                // `source.uri` is NOT NULL, so a NULL here with a set
                // `source_id` means the source row is gone.
                let origin = Origin::of(row.get(4)?, row.get(5)?);
                Ok(CitedCapture {
                    authority_id: row.get(0)?,
                    captured_at: row.get(1)?,
                    kind: row.get(2)?,
                    path: row.get(3)?,
                    origin,
                })
            },
        )
        .optional()
        .unwrap_or(None)
    }

    /// The https URL this capture's quotes are attributed to, or `None`
    /// (no citation). A source-less page or upload borrows the first
    /// `website_official` URI of `authority_id`; nothing else does.
    pub fn source_url(&self, conn: &Connection, authority_id: i64) -> Option<String> {
        handbook_store::citation::source_url(conn, &self.origin, &self.kind, authority_id)
    }

    /// `pdf` when the cited URL or the stored bytes are a PDF, `upload`
    /// for any other staff upload (a `file` capture), else `web`.
    pub fn citation_kind(&self, store_root: &Path, url: &str) -> &'static str {
        if url_names_pdf(url) || blob_is_pdf(store_root, &self.path) {
            "pdf"
        } else if self.kind == "file" {
            "upload"
        } else {
            "web"
        }
    }
}

/// True when the URL's path ends in `.pdf` (any case; the query and
/// fragment do not count).
fn url_names_pdf(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|u| u.path().to_ascii_lowercase().ends_with(".pdf"))
}

/// True when the stored blob starts with the PDF magic (`%PDF`, the
/// upload lane's own test). A missing or short blob is not a PDF, and
/// a path that would leave the store root is never opened.
fn blob_is_pdf(store_root: &Path, path: &str) -> bool {
    use std::io::Read as _;
    let rel = Path::new(path);
    if path.is_empty()
        || rel.is_absolute()
        || rel
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return false;
    }
    let mut head = [0u8; 4];
    std::fs::File::open(store_root.join(rel))
        .and_then(|mut file| file.read_exact(&mut head))
        .is_ok()
        && &head == b"%PDF"
}

/// Newest staff accept per fact: `MAX(audit.at)` over each fact's
/// `fact_accept` rows (the authority index's `verified_at` rule,
/// grouped per fact), for all of one read's facts in ONE grouped
/// query. `audit` is indexed on `at` only, so a read pays one scan of
/// it, never one per fact. Facts never accepted are absent.
pub fn accepted_at(conn: &Connection, fact_ids: &[i64]) -> HashMap<i64, i64> {
    if fact_ids.is_empty() {
        return HashMap::new();
    }
    let placeholders = vec!["?"; fact_ids.len()].join(",");
    conn.prepare(&format!(
        "SELECT entity_id, MAX(at) FROM audit
         WHERE entity = 'fact' AND action = 'fact_accept'
           AND entity_id IN ({placeholders})
         GROUP BY entity_id"
    ))
    .and_then(|mut stmt| {
        stmt.query_map(rusqlite::params_from_iter(fact_ids), |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect::<rusqlite::Result<HashMap<_, _>>>()
    })
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use handbook_store::{db, store::Store};

    const SITE: &str = "https://www.sos.wa.gov/corporations-charities/apostilles";
    const FORM: &str = "https://www.sos.wa.gov/sites/default/files/2025-06/6.2025%20-%20Apostilles%20request%20form.pdf";

    /// Migrated temp store: one authority with an official website.
    struct Seed {
        tmp: tempfile::TempDir,
        conn: Connection,
        authority: i64,
    }

    impl Seed {
        fn new() -> Self {
            let tmp = tempfile::tempdir().unwrap();
            let store = Store::new(tmp.path());
            store.init().unwrap();
            let conn = db::open(&store).unwrap();
            db::migrate(&conn).unwrap();
            let authority = db::add_authority(
                &conn,
                &db::NewAuthority {
                    kind: "state",
                    country: "USA",
                    region: "Washington",
                    name: "WA Secretary of State",
                    slug: "usa-wa-sos",
                    actor: "t",
                    at: 1,
                },
            )
            .unwrap();
            let seed = Seed {
                tmp,
                conn,
                authority,
            };
            seed.source("website_official", SITE);
            seed
        }

        fn root(&self) -> &Path {
            self.tmp.path()
        }

        fn source(&self, kind: &str, uri: &str) -> i64 {
            self.conn
                .execute(
                    "INSERT INTO source(authority_id, kind, uri) VALUES (?1, ?2, ?3)",
                    rusqlite::params![self.authority, kind, uri],
                )
                .unwrap();
            self.conn.last_insert_rowid()
        }

        /// One capture (blob `bytes`, sidecar `text`) plus an accepted
        /// fact citing the whole sidecar. Returns `(capture, fact)`.
        fn cited(&self, kind: &str, source: Option<i64>, bytes: &[u8], text: &str) -> (i64, i64) {
            let next: i64 = self
                .conn
                .query_row("SELECT COALESCE(MAX(id), 0) + 1 FROM capture", [], |row| {
                    row.get(0)
                })
                .unwrap();
            let path = format!("blob-{next}");
            std::fs::write(self.root().join(&path), bytes).unwrap();
            self.conn
                .execute(
                    "INSERT INTO capture(authority_id, source_id, captured_at, kind, path,
                                         bytes_raw, bytes_stored, sha256, trust)
                     VALUES (?1, ?2, 1793664000, ?3, ?4, 1, 1, 's', 'official')",
                    rusqlite::params![self.authority, source, kind, path],
                )
                .unwrap();
            let capture = self.conn.last_insert_rowid();
            std::fs::create_dir_all(self.root().join("text")).unwrap();
            std::fs::write(
                self.root().join("text").join(format!("{capture}.txt")),
                text,
            )
            .unwrap();
            self.conn
                .execute(
                    "INSERT INTO fact(authority_id, capture_id, topic, statement, status,
                                      created_by, created_at, citation_start, citation_end)
                     VALUES (?1, ?2, 'po_box', ?3, 'accepted', 't', 1, 0, ?4)",
                    rusqlite::params![self.authority, capture, text, text.len() as i64],
                )
                .unwrap();
            (capture, self.conn.last_insert_rowid())
        }

        /// The public `/facts` views, keyed by fact id.
        fn views(&self) -> HashMap<i64, crate::approval::FactView> {
            crate::approval::accepted_fact_views(&self.conn, self.root(), "usa-wa-sos")
                .unwrap()
                .into_iter()
                .map(|f| (f.id, f))
                .collect()
        }

        /// Delete a source row the way the ops script does (foreign
        /// keys off), leaving its captures' `source_id` dangling.
        fn drop_source(&self, source: i64) {
            self.conn
                .pragma_update(None, "foreign_keys", "OFF")
                .unwrap();
            self.conn
                .execute("DELETE FROM source WHERE id = ?1", [source])
                .unwrap();
            self.conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        }
    }

    #[test]
    fn citation_kind_names_web_pdf_and_upload() {
        let seed = Seed::new();
        let page = seed.source("website_official", "https://www.sos.wa.gov/fees");
        let form = seed.source("document", FORM);
        let (web_cap, web) = seed.cited("warc", Some(page), b"<html>", "Mail to PO Box 40220.");
        let (pdf_cap, pdf) = seed.cited(
            "file",
            Some(form),
            b"%PDF-1.7",
            "Mail to PO Box 40234, Olympia.",
        );
        // A staff upload of an HTML page under a non-PDF URL.
        let (_, upload) = seed.cited(
            "file",
            Some(page),
            b"<html>",
            "PO Box 40220 is the mail box.",
        );
        // PDF bytes behind a URL that does not say so still read as PDF.
        let (_, sniffed) = seed.cited(
            "warc",
            None,
            b"%PDF-1.4",
            "Requests go to PO Box 40234 now.",
        );
        let views = seed.views();
        let cite = |id: i64| views[&id].citations[0].clone();
        assert_eq!(cite(web).kind, "web");
        assert_eq!(cite(web).capture_id, web_cap);
        assert_eq!(cite(pdf).kind, "pdf");
        assert_eq!(cite(pdf).capture_id, pdf_cap);
        assert_eq!(cite(pdf).source_url, FORM);
        assert_eq!(cite(upload).kind, "upload");
        assert_eq!(cite(sniffed).kind, "pdf");
        assert_eq!(
            cite(sniffed).source_url,
            SITE,
            "source-less page: official site"
        );
    }

    #[test]
    fn citation_pdf_sniff_stays_inside_the_store() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("store");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("form"), b"%PDF-1.7").unwrap();
        std::fs::write(root.join("page"), b"<html>").unwrap();
        std::fs::write(tmp.path().join("outside"), b"%PDF-1.7").unwrap();
        assert!(blob_is_pdf(&root, "form"));
        assert!(!blob_is_pdf(&root, "page"));
        assert!(!blob_is_pdf(&root, "missing"));
        assert!(!blob_is_pdf(&root, ""));
        assert!(!blob_is_pdf(&root, "../outside"));
        let outside = tmp.path().join("outside");
        assert!(!blob_is_pdf(&root, outside.to_str().unwrap()));
    }

    #[test]
    fn citation_urls_name_pdfs_by_path_only() {
        assert!(url_names_pdf(FORM));
        assert!(url_names_pdf("https://example.gov/Form.PDF?v=2#page=1"));
        assert!(!url_names_pdf("https://example.gov/form?file=a.pdf"));
        assert!(!url_names_pdf("https://example.gov/pdf"));
        assert!(!url_names_pdf("not a url.pdf"));
    }

    #[test]
    fn citation_fallback_covers_only_pages_and_uploads() {
        let seed = Seed::new();
        let (_, warc) = seed.cited("warc", None, b"<p>", "Mail to PO Box 40220.");
        let (_, file) = seed.cited("file", None, b"<p>", "Mail to PO Box 40234.");
        let mut silent = Vec::new();
        for kind in [
            "transcript",
            "comment",
            "email",
            "photo",
            "video",
            "audio",
            "scan",
        ] {
            let text = format!("{kind}: they said PO Box 40220");
            silent.push(seed.cited(kind, None, b"x", &text).1);
        }
        let views = seed.views();
        assert_eq!(views[&warc].citations[0].source_url, SITE);
        assert_eq!(views[&file].citations[0].source_url, SITE);
        for id in silent {
            assert!(
                views[&id].citations.is_empty(),
                "fact {id} ({}) must not borrow the official site",
                views[&id].statement
            );
        }
    }

    #[test]
    fn citation_dangling_source_never_falls_back() {
        let seed = Seed::new();
        let gone = seed.source("website_official", "https://www.sos.wa.gov/old-fees");
        let (_, fact) = seed.cited("warc", Some(gone), b"<p>", "Mail to PO Box 40220.");
        assert_eq!(
            seed.views()[&fact].citations.len(),
            1,
            "cited while the source lives"
        );
        seed.drop_source(gone);
        assert!(seed.views()[&fact].citations.is_empty());
        // The capture's own non-https source never falls back either.
        let mail = seed.source("email_official", "apostilles@sos.wa.gov");
        let (_, emailed) = seed.cited("warc", Some(mail), b"<p>", "Mail to PO Box 40234.");
        assert!(seed.views()[&emailed].citations.is_empty());
    }

    #[test]
    fn unfurl_sources_follow_the_citation_rules() {
        let seed = Seed::new();
        let page = seed.source("website_official", "https://www.sos.wa.gov/fees");
        seed.cited("warc", Some(page), b"<p>", "Mail to PO Box 40220.");
        let gone = seed.source("website_official", "https://www.sos.wa.gov/old-fees");
        seed.cited("warc", Some(gone), b"<p>", "Mail to PO Box 40234.");
        seed.drop_source(gone);
        seed.cited("transcript", None, b"x", "caller: PO Box 40234 is right");
        let meta = crate::unfurl::load_dossier(&seed.conn, seed.root(), "usa-wa-sos")
            .unwrap()
            .unwrap();
        assert_eq!(meta.facts.len(), 3, "every accepted fact is listed");
        // Only the live page source: neither the dangling capture nor the
        // transcript borrows the official site.
        assert_eq!(
            meta.sources,
            vec!["https://www.sos.wa.gov/fees".to_string()]
        );
        assert_eq!(meta.last_day.as_deref(), Some("2026-11-03"));
    }

    #[test]
    fn citation_accepted_at_is_the_newest_accept_per_fact() {
        let seed = Seed::new();
        let (_, twice) = seed.cited("warc", None, b"<p>", "Mail to PO Box 40220.");
        let (_, seeded) = seed.cited("warc", None, b"<p>", "Mail to PO Box 40234.");
        for (fact, at, action) in [
            (twice, 1_793_700_000, "fact_accept"),
            (twice, 1_793_800_000, "fact_accept"),
            // Other audit rows never count as an accept.
            (twice, 1_793_900_000, "fact_retire"),
            (seeded, 1_793_750_000, "fact_reject"),
        ] {
            seed.conn
                .execute(
                    "INSERT INTO audit(at, actor, action, entity, entity_id)
                     VALUES (?1, 'staff-1', ?2, 'fact', ?3)",
                    rusqlite::params![at, action, fact],
                )
                .unwrap();
        }
        let map = accepted_at(&seed.conn, &[twice, seeded, 999]);
        assert_eq!(map.get(&twice), Some(&1_793_800_000));
        assert_eq!(map.get(&seeded), None);
        assert!(accepted_at(&seed.conn, &[]).is_empty());
        let views = seed.views();
        assert_eq!(views[&twice].accepted_at, Some(1_793_800_000));
        assert_eq!(views[&seeded].accepted_at, None);
    }

    /// The grouped query at scale: 200 accepted facts (the `/facts`
    /// cap) among a million audit rows. Prints timings, asserts only
    /// correctness (timings are the machine's, not a gate).
    #[test]
    #[ignore = "perf: seeds 1M audit rows; run with --ignored --nocapture"]
    fn citation_accepted_at_scales_to_a_million_audit_rows() {
        let seed = Seed::new();
        let mut ids = Vec::new();
        for n in 0..200 {
            let text = format!("Mail to PO Box 40220 ({n:03})");
            ids.push(seed.cited("warc", None, b"<p>", &text).1);
        }
        seed.conn.execute_batch("BEGIN").unwrap();
        seed.conn
            .execute(
                "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 1000000)
                 INSERT INTO audit(at, actor, action, entity, entity_id, reason)
                 SELECT 1700000000 + i, 'sweep',
                        CASE i % 3 WHEN 0 THEN 'ingest_capture' WHEN 1 THEN 'fact_reject'
                                   ELSE 'analysis_run' END,
                        CASE i % 3 WHEN 1 THEN 'fact' ELSE 'capture' END,
                        i % 5000, 'fetch https://example.gov/page sha256=abc' FROM n",
                [],
            )
            .unwrap();
        for (k, id) in ids.iter().enumerate() {
            seed.conn
                .execute(
                    "INSERT INTO audit(at, actor, action, entity, entity_id)
                     VALUES (?1, 'staff-1', 'fact_accept', 'fact', ?2)",
                    rusqlite::params![1_793_000_000 + k as i64, id],
                )
                .unwrap();
        }
        seed.conn.execute_batch("COMMIT").unwrap();
        let started = std::time::Instant::now();
        let map = accepted_at(&seed.conn, &ids);
        let grouped = started.elapsed();
        assert_eq!(map.len(), 200);
        let started = std::time::Instant::now();
        let views = seed.views();
        let read = started.elapsed();
        assert!(views.values().all(|v| v.accepted_at.is_some()));
        eprintln!("accepted_at over 1M audit rows: {grouped:?}; whole /facts read: {read:?}");
    }
}
