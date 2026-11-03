//! Byte ingest: sha256 dedup, plain-blob WARC capture, text sidecar,
//! FTS row.
//!
//! "WARC plain-blob first" (roadmap item 4): the capture row says
//! `kind='warc'` but `path` points at the raw bytes under their blob
//! address — no WARC record envelope, no `warcinfo` TLS evidence yet.
//! The `notes` column says so on every row this module writes.

use rusqlite::{Connection, OptionalExtension};

use contracts::Result;
use handbook_store::db::in_transaction;
use handbook_store::store::{Store, sha256_hex};

/// Marker written into `capture.notes` by [`ingest_bytes`].
pub const PLAIN_BLOB_NOTE: &str =
    "warc-plain-blob: raw bytes at blob address, no WARC envelope (thin watch)";

/// One ingest call's inputs.
pub struct IngestInput<'a> {
    pub authority_id: i64,
    pub source_id: Option<i64>,
    /// `capture.kind` value (`warc` for F1/F2 watch fetches).
    pub kind: &'a str,
    /// `capture.trust` value (`official`, `staff_verified`, ...).
    pub trust: &'a str,
    pub bytes: &'a [u8],
    /// Sidecar + FTS body (see [`extract_text`]).
    pub text: &'a str,
    pub captured_at: i64,
    pub actor: &'a str,
    /// Short provenance for the audit row (usually the fetched URL).
    pub origin: &'a str,
}

/// Outcome of one [`ingest_bytes`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestOutcome {
    /// A new capture row (plus sidecar, FTS, audit) was stored.
    Created { capture_id: i64 },
    /// Identical bytes for this authority already exist; nothing stored.
    Duplicate { capture_id: i64 },
}

impl IngestOutcome {
    pub fn capture_id(&self) -> i64 {
        match *self {
            IngestOutcome::Created { capture_id } | IngestOutcome::Duplicate { capture_id } => {
                capture_id
            }
        }
    }
}

/// Store fetched bytes as a capture, or dedup-hit the existing row.
///
/// Dedup key is `(authority_id, sha256)`: byte-identical re-fetches
/// create no new work. The capture + FTS + audit rows commit
/// atomically; the `text/<id>.txt` sidecar is written after commit
/// (temp file + rename). A missing sidecar is a specified agent
/// skip+task state, never silent corruption.
pub fn ingest_bytes(
    conn: &Connection,
    store: &Store,
    input: &IngestInput<'_>,
) -> Result<IngestOutcome> {
    let sha = sha256_hex(input.bytes);
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM capture WHERE authority_id = ?1 AND sha256 = ?2",
            rusqlite::params![input.authority_id, sha],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(capture_id) = existing {
        return Ok(IngestOutcome::Duplicate { capture_id });
    }

    // Idempotent content write; safe to do before the transaction.
    let sha_again = store.write_blob(input.bytes)?;
    debug_assert_eq!(sha, sha_again);
    let relative = store
        .blob_path(&sha)?
        .strip_prefix(store.root())
        .expect("blob path is under the store root")
        .to_str()
        .expect("blob path is UTF-8")
        .to_string();

    let capture_id = in_transaction(conn, || {
        conn.execute(
            "INSERT INTO capture(authority_id, source_id, captured_at, kind,
                                  path, bytes_raw, bytes_stored, sha256,
                                  trust, notes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                input.authority_id,
                input.source_id,
                input.captured_at,
                input.kind,
                relative,
                input.bytes.len() as i64,
                input.bytes.len() as i64,
                sha,
                input.trust,
                PLAIN_BLOB_NOTE,
            ],
        )?;
        let capture_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO capture_fts(capture_id, body) VALUES (?1, ?2)",
            rusqlite::params![capture_id, input.text],
        )?;
        conn.execute(
            "INSERT INTO audit(at, actor, action, entity, entity_id, reason)
             VALUES (?1, ?2, 'ingest_capture', 'capture', ?3, ?4)",
            rusqlite::params![
                input.captured_at,
                input.actor,
                capture_id,
                format!("fetch {} sha256={sha}", input.origin),
            ],
        )?;
        Ok(capture_id)
    })?;

    write_sidecar(store, capture_id, input.text)?;
    Ok(IngestOutcome::Created { capture_id })
}

/// Elements whose bodies are never page text: scripts (including
/// `importmap` / JSON-LD blobs), styles, templates, embedded media.
const SKIP_ELEMENTS: &[&str] = &[
    "script", "style", "template", "noscript", "svg", "iframe", "object", "canvas",
];

/// Elements that start a new line, so headings, paragraphs and list
/// items stay apart instead of running into one another.
const BLOCK_ELEMENTS: &[&str] = &[
    "address", "article", "aside", "blockquote", "br", "caption", "dd", "div", "dl", "dt",
    "fieldset", "figcaption", "figure", "footer", "form", "h1", "h2", "h3", "h4", "h5", "h6",
    "header", "hr", "li", "main", "nav", "ol", "p", "pre", "section", "table", "title", "tr",
    "ul",
];

/// Capture text for sidecars, FTS and model input: lossy UTF-8, tags
/// stripped, the bodies of [`SKIP_ELEMENTS`] and comments dropped,
/// entities decoded, one line per block element, table cells joined
/// with " | ", whitespace collapsed within each line. Plain text (no
/// tags) comes back with only its whitespace collapsed.
pub fn extract_text(bytes: &[u8]) -> String {
    let html = String::from_utf8_lossy(bytes);
    let s = html.as_ref();
    // ASCII lowercasing keeps every byte offset, so indices found in
    // `lower` slice `s` on the same char boundaries.
    let lower = s.to_ascii_lowercase();
    let mut out = String::with_capacity(s.len() / 2);
    let mut cells_in_row = 0usize;
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        if let Some(comment) = rest.strip_prefix("<!--") {
            i = comment.find("-->").map_or(s.len(), |e| i + 4 + e + 3);
            continue;
        }
        let starts_tag = rest.starts_with('<')
            && rest[1..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || matches!(c, '/' | '!' | '?'));
        if !starts_tag {
            let next = rest[1..].find('<').map_or(s.len(), |n| i + 1 + n);
            // Source line breaks are layout, not structure: only
            // block elements start a line.
            let run = decode_entities(&s[i..next]);
            out.extend(run.chars().map(|c| if c.is_whitespace() { ' ' } else { c }));
            i = next;
            continue;
        }
        let (tag, after) = match rest.find('>') {
            Some(e) => (&lower[i + 1..i + e], i + e + 1),
            None => (&lower[i + 1..], s.len()),
        };
        let closing = tag.starts_with('/');
        let name = tag
            .trim_start_matches('/')
            .split(|c: char| !c.is_ascii_alphanumeric())
            .next()
            .unwrap_or("");
        if !closing && !tag.ends_with('/') && SKIP_ELEMENTS.contains(&name) {
            let close = format!("</{name}");
            i = match lower[after..].find(&close) {
                Some(c) => {
                    let at = after + c;
                    s[at..].find('>').map_or(s.len(), |g| at + g + 1)
                }
                None => s.len(),
            };
            continue;
        }
        if BLOCK_ELEMENTS.contains(&name) {
            out.push('\n');
        }
        if name == "tr" && !closing {
            cells_in_row = 0;
        }
        if (name == "td" || name == "th") && !closing {
            if cells_in_row > 0 {
                out.push_str(" | ");
            }
            cells_in_row += 1;
        }
        i = after;
    }
    out.lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Decode character references: numeric (`&#233;`, `&#xE9;`) and the
/// named ones official pages use. Unknown names stay as written.
fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp..];
        let semi = tail[1..]
            .char_indices()
            .take(12)
            .find(|(_, c)| *c == ';')
            .map(|(k, _)| k + 1);
        match semi.and_then(|semi| entity(&tail[1..semi]).map(|ch| (ch, semi))) {
            Some((ch, semi)) => {
                out.push(ch);
                rest = &tail[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn entity(name: &str) -> Option<char> {
    if let Some(num) = name.strip_prefix('#') {
        let code = match num.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => num.parse().ok()?,
        };
        return char::from_u32(code).filter(|c| *c != '\0');
    }
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        "ndash" => '\u{2013}',
        "mdash" => '\u{2014}',
        "lsquo" => '\u{2018}',
        "rsquo" => '\u{2019}',
        "ldquo" => '\u{201c}',
        "rdquo" => '\u{201d}',
        "hellip" => '\u{2026}',
        "bull" => '\u{2022}',
        "middot" => '\u{b7}',
        "sect" => '\u{a7}',
        "para" => '\u{b6}',
        "deg" => '\u{b0}',
        "copy" => '\u{a9}',
        "reg" => '\u{ae}',
        "trade" => '\u{2122}',
        "euro" => '\u{20ac}',
        "pound" => '\u{a3}',
        "yen" => '\u{a5}',
        "cent" => '\u{a2}',
        "times" => '\u{d7}',
        _ => return None,
    })
}

fn write_sidecar(store: &Store, capture_id: i64, text: &str) -> Result<()> {
    let dir = store.root().join("text");
    std::fs::create_dir_all(&dir)?;
    let final_path = dir.join(format!("{capture_id}.txt"));
    let tmp_path = dir.join(format!(".{capture_id}.txt.tmp"));
    std::fs::write(&tmp_path, text.as_bytes())?;
    std::fs::rename(&tmp_path, &final_path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use handbook_store::db::{NewAuthority, add_authority, migrate, open};

    fn migrated() -> (tempfile::TempDir, Store, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.init().unwrap();
        let conn = open(&store).unwrap();
        migrate(&conn).unwrap();
        (dir, store, conn)
    }

    fn authority(conn: &Connection, slug: &str) -> i64 {
        add_authority(
            conn,
            &NewAuthority {
                kind: "state",
                country: "USA",
                region: "Washington",
                name: "WA Secretary of State",
                slug,
                actor: "test",
                at: 1_800_000_000,
            },
        )
        .unwrap()
    }

    fn input<'a>(authority_id: i64, bytes: &'a [u8], text: &'a str) -> IngestInput<'a> {
        IngestInput {
            authority_id,
            source_id: None,
            kind: "warc",
            trust: "official",
            bytes,
            text,
            captured_at: 1_800_000_100,
            actor: "test",
            origin: "https://example.com/synthetic/1",
        }
    }

    #[test]
    fn creates_capture_sidecar_fts_and_audit() {
        let (_dir, store, conn) = migrated();
        let authority_id = authority(&conn, "usa-wa-sos");
        let outcome = ingest_bytes(
            &conn,
            &store,
            &input(authority_id, b"<p>PO Box 40220</p>", "PO Box 40220"),
        )
        .unwrap();
        let IngestOutcome::Created { capture_id } = outcome else {
            panic!("first ingest must create, got: {outcome:?}");
        };

        let (kind, trust, notes): (String, String, String) = conn
            .query_row(
                "SELECT kind, trust, notes FROM capture WHERE id = ?1",
                [capture_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(kind, "warc");
        assert_eq!(trust, "official");
        assert_eq!(notes, PLAIN_BLOB_NOTE);

        let sidecar = store.root().join("text").join(format!("{capture_id}.txt"));
        assert_eq!(std::fs::read_to_string(&sidecar).unwrap(), "PO Box 40220");

        let hits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM capture_fts WHERE capture_id = ?1 AND body MATCH '40220'",
                [capture_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(hits, 1);

        let audits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit WHERE action = 'ingest_capture' AND entity_id = ?1",
                [capture_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(audits, 1);
    }

    #[test]
    fn identical_bytes_dedup_to_one_capture() {
        let (_dir, store, conn) = migrated();
        let authority_id = authority(&conn, "usa-wa-sos");
        let first = ingest_bytes(
            &conn,
            &store,
            &input(authority_id, b"<p>PO Box 40220</p>", "PO Box 40220"),
        )
        .unwrap();
        let second = ingest_bytes(
            &conn,
            &store,
            &input(authority_id, b"<p>PO Box 40220</p>", "PO Box 40220"),
        )
        .unwrap();
        assert_eq!(first.capture_id(), second.capture_id());
        assert!(matches!(second, IngestOutcome::Duplicate { .. }));

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM capture WHERE id > 0", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn same_bytes_for_another_authority_are_distinct() {
        let (_dir, store, conn) = migrated();
        let a = authority(&conn, "usa-wa-sos");
        let b = authority(&conn, "usa-or-sos");
        let first = ingest_bytes(&conn, &store, &input(a, b"same", "same")).unwrap();
        let second = ingest_bytes(&conn, &store, &input(b, b"same", "same")).unwrap();
        assert_ne!(first.capture_id(), second.capture_id());
    }

    #[test]
    fn extract_text_strips_tags_and_collapses_space() {
        assert_eq!(
            extract_text(b"<html><body><p>PO  Box\n40220</p></body></html>"),
            "PO Box 40220"
        );
        assert_eq!(extract_text(b"plain"), "plain");
    }

    #[test]
    fn extract_text_drops_scripts_styles_and_comments() {
        // Shape of a Liferay portal page: an importmap and inline
        // scripts in <head>, a comment, entities, a fee table.
        let html = "<html><head><title>Trang ch&#7911; - C&#7893;ng th&ocirc;ng tin</title>\
            <script type=\"importmap\">{\"imports\":{\"react-dom\":\"/o/x.js\"}}</script>\
            <style>.a{color:red}</style><SCRIPT>var a = '<p>no</p>';</SCRIPT></head>\
            <body><!-- <p>hidden</p> --><h1>Ch&#7913;ng nh&#7853;n l&atilde;nh s&#7921;</h1>\
            <p>Fee:&nbsp;$15 &amp; up</p><table><tr><th>Item</th><th>Fee</th></tr>\
            <tr><td>Apostille</td><td>$15</td></tr></table><p>a < b</p></body></html>";
        let text = extract_text(html.as_bytes());
        assert!(!text.contains("imports"), "{text}");
        assert!(!text.contains("color:red"), "{text}");
        assert!(!text.contains("var a"), "{text}");
        assert!(!text.contains("hidden"), "{text}");
        assert!(text.starts_with("Trang ch\u{1ee7} - C\u{1ed5}ng th&ocirc;ng tin\n"), "{text}");
        assert!(text.contains("Fee: $15 & up"), "{text}");
        assert!(text.contains("Item | Fee\nApostille | $15"), "{text}");
        assert!(text.contains("a < b"), "{text}");
    }
}
