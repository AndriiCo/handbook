//! The citation-URL rule (plan §5.2), one copy for every reader that
//! attributes a fact's quote to a page: `/facts` and the crawler head
//! (crates/web `citation`) and the follower emails (crates/notify).
//!
//! A quote is attributed to its capture's own source. Only a source-less
//! fetched page (`warc`) or staff upload (`file`) borrows the
//! authority's first `website_official` URI: transcripts, comments,
//! emails and media never borrow it, and a `source_id` whose row is gone
//! (an ops script deletes source rows with foreign keys off) cites
//! nothing, so no quote is misattributed to the office. Only https URLs
//! are cited.

use rusqlite::Connection;

/// Capture kinds whose source-less rows may cite the authority's
/// official website: fetched pages (`warc`) and staff uploads (`file`).
pub const OFFICIAL_FALLBACK_KINDS: &[&str] = &["warc", "file"];

/// Where a capture came from, as its `source_id` tells it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// `source_id IS NULL`.
    Unattributed,
    /// The source row's URI.
    Source(String),
    /// `source_id` names a row that no longer exists.
    Dangling,
}

impl Origin {
    /// From `capture.source_id` and the `source.uri` a `LEFT JOIN`
    /// found: `source.uri` is NOT NULL, so no URI with a set id means
    /// the source row is gone.
    #[must_use]
    pub fn of(source_id: Option<i64>, uri: Option<String>) -> Self {
        match (source_id, uri) {
            (None, _) => Self::Unattributed,
            (Some(_), Some(uri)) => Self::Source(uri),
            (Some(_), None) => Self::Dangling,
        }
    }
}

/// The https URL a capture of `capture_kind` from `origin` is cited
/// to, or `None` (no citation). A source-less page or upload borrows
/// the first `website_official` URI of `authority_id`; nothing else
/// does.
pub fn source_url(
    conn: &Connection,
    origin: &Origin,
    capture_kind: &str,
    authority_id: i64,
) -> Option<String> {
    let uri: Option<String> = match origin {
        Origin::Source(uri) => Some(uri.clone()),
        Origin::Dangling => None,
        Origin::Unattributed if OFFICIAL_FALLBACK_KINDS.contains(&capture_kind) => conn
            .query_row(
                "SELECT uri FROM source WHERE authority_id = ?1 AND kind = 'website_official'
                 ORDER BY id LIMIT 1",
                [authority_id],
                |row| row.get(0),
            )
            .ok(),
        Origin::Unattributed => None,
    };
    uri.filter(|u| u.starts_with("https://"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE source(id INTEGER PRIMARY KEY, authority_id INTEGER, kind TEXT, uri TEXT);
             INSERT INTO source VALUES (1, 5, 'website_thirdparty', 'https://example.test/');
             INSERT INTO source VALUES (2, 5, 'website_official', 'https://www.sos.wa.gov/apostille');
             INSERT INTO source VALUES (3, 5, 'website_official', 'https://www.sos.wa.gov/');
             INSERT INTO source VALUES (4, 6, 'website_official', 'http://plain.example/');",
        )
        .unwrap();
        conn
    }

    #[test]
    fn only_pages_and_uploads_borrow_the_official_site() {
        let conn = conn();
        let site = Some("https://www.sos.wa.gov/apostille".to_string());
        for kind in OFFICIAL_FALLBACK_KINDS {
            assert_eq!(source_url(&conn, &Origin::Unattributed, kind, 5), site);
        }
        for kind in [
            "transcript",
            "comment",
            "email",
            "photo",
            "video",
            "audio",
            "scan",
        ] {
            assert_eq!(source_url(&conn, &Origin::Unattributed, kind, 5), None);
        }
        // A dangling source cites nothing, even for a page.
        assert_eq!(source_url(&conn, &Origin::Dangling, "warc", 5), None);
        // The capture's own source wins, https only; no fallback after.
        let own = Origin::Source("https://www.sos.wa.gov/fees".into());
        assert_eq!(
            source_url(&conn, &own, "transcript", 5).as_deref(),
            Some("https://www.sos.wa.gov/fees")
        );
        let plain = Origin::Source("http://www.sos.wa.gov/fees".into());
        assert_eq!(source_url(&conn, &plain, "warc", 5), None);
        // A non-https official site is never cited either.
        assert_eq!(source_url(&conn, &Origin::Unattributed, "warc", 6), None);
        assert_eq!(source_url(&conn, &Origin::Unattributed, "warc", 7), None);
    }

    #[test]
    fn origin_reads_the_left_join() {
        assert_eq!(Origin::of(None, None), Origin::Unattributed);
        assert_eq!(
            Origin::of(Some(3), Some("https://a.test/".into())),
            Origin::Source("https://a.test/".into())
        );
        assert_eq!(Origin::of(Some(3), None), Origin::Dangling);
    }
}
