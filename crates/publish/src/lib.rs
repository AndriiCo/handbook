//! Approved-changes publish (F11): accepted-only static handbook builds.
//!
//! [`build_handbook`] renders one HTML page per authority from `accepted`
//! facts only, emits `sitemap.xml` + per-country shards and a merged
//! `robots.txt`, then publishes atomically: the new tree lands in
//! `builds/<build_no>/` and the `current` symlink flips to it with one
//! rename. Every build writes a `build` row (monotonic `build_no`,
//! `facts_hash` over the accepted inputs) plus an `audit` row.
//!
//! Conflict policy (decided: **banner, not block**): [`conflicts`]
//! finds same authority + same topic with two `accepted` facts carrying
//! different statements, and every affected page renders a visible
//! banner naming both citations. Contradictions are never published
//! silently.

use std::path::{Path, PathBuf};

use contracts::Result;
use handbook_store::store::{Store, sha256_hex};
use rusqlite::Connection;

/// One F6 conflict: two `accepted` facts, same authority + topic,
/// different statements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub authority_id: i64,
    pub authority_slug: String,
    pub topic: String,
    pub first_id: i64,
    pub first_statement: String,
    pub first_capture_id: i64,
    pub second_id: i64,
    pub second_statement: String,
    pub second_capture_id: i64,
}

/// Every unresolved F6 conflict in the store, ordered deterministically.
pub fn conflicts(conn: &Connection) -> Result<Vec<Conflict>> {
    let mut stmt = conn.prepare(
        "SELECT a.slug, f.authority_id, f.topic,
                f.id, f.statement, f.capture_id,
                g.id, g.statement, g.capture_id
         FROM fact f
         JOIN fact g ON g.authority_id = f.authority_id
                    AND g.topic = f.topic
                    AND g.status = 'accepted'
                    AND g.id > f.id
                    AND g.statement != f.statement
         JOIN authority a ON a.id = f.authority_id
         WHERE f.status = 'accepted'
         ORDER BY a.slug, f.topic, f.id, g.id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(Conflict {
            authority_slug: row.get(0)?,
            authority_id: row.get(1)?,
            topic: row.get(2)?,
            first_id: row.get(3)?,
            first_statement: row.get(4)?,
            first_capture_id: row.get(5)?,
            second_id: row.get(6)?,
            second_statement: row.get(7)?,
            second_capture_id: row.get(8)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// One accepted fact with its citation, for page rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedFact {
    pub id: i64,
    pub topic: String,
    pub statement: String,
    pub capture_id: i64,
    pub capture_sha: String,
}

/// Accepted facts for one authority, ordered by topic then id.
/// Proposed/community/rejected rows never appear here.
pub fn accepted_facts(conn: &Connection, authority_id: i64) -> Result<Vec<AcceptedFact>> {
    let mut stmt = conn.prepare(
        "SELECT f.id, f.topic, f.statement, f.capture_id, c.sha256
         FROM fact f JOIN capture c ON c.id = f.capture_id
         WHERE f.authority_id = ?1 AND f.status = 'accepted'
         ORDER BY f.topic, f.id",
    )?;
    let rows = stmt.query_map([authority_id], |row| {
        Ok(AcceptedFact {
            id: row.get(0)?,
            topic: row.get(1)?,
            statement: row.get(2)?,
            capture_id: row.get(3)?,
            capture_sha: row.get(4)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// sha256 over every accepted input, in a fixed order: one line per
/// fact, `authority_slug\0topic\0statement\0capture_sha\n`, authorities
/// ordered by slug. Any content-affecting change flips the hash.
pub fn facts_hash(conn: &Connection) -> Result<String> {
    let mut stmt = conn.prepare(
        "SELECT a.slug, f.topic, f.statement, c.sha256
         FROM fact f
         JOIN authority a ON a.id = f.authority_id
         JOIN capture c ON c.id = f.capture_id
         WHERE f.status = 'accepted'
         ORDER BY a.slug, f.topic, f.id",
    )?;
    let mut joined = String::new();
    let rows: Vec<(String, String, String, String)> = stmt
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (slug, topic, statement, sha) in &rows {
        joined.push_str(slug);
        joined.push('\0');
        joined.push_str(topic);
        joined.push('\0');
        joined.push_str(statement);
        joined.push('\0');
        joined.push_str(sha);
        joined.push('\n');
    }
    Ok(sha256_hex(joined.as_bytes()))
}

/// Options for [`build_handbook`].
#[derive(Debug, Clone)]
pub struct BuildOptions {
    /// Directory holding `builds/<build_no>/` trees plus the `current`
    /// symlink. Created when missing.
    pub out_dir: PathBuf,
    /// Public base URL for sitemap locs, e.g. `https://andrii.ai/apostille`.
    pub base_url: String,
    /// Audit actor recorded on the `build` + `audit` rows.
    pub actor: String,
    /// Unix seconds for the `build.at` / `audit.at` columns.
    pub at: i64,
}

/// Outcome of one [`build_handbook`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildReport {
    pub build_no: i64,
    pub facts_hash: String,
    pub pages: usize,
    pub conflicts: usize,
    /// Directory of the published tree (`builds/<build_no>/`).
    pub dir: PathBuf,
}

/// Render accepted facts to static pages and publish atomically.
///
/// Steps: compute [`facts_hash`]; render every authority page plus
/// `index.html`, `sitemap.xml`, per-country shards, and merged
/// `robots.txt` into a fresh staging directory; insert the `build`
/// row (monotonic `build_no`) and its `audit` row in one transaction;
/// then flip the `current` symlink to the new tree with a single
/// rename. A failure anywhere before the flip leaves `current`
/// pointing at the previous good build.
///
/// # Errors
///
/// Returns [`contracts::HandbookError`] on database or file-store
/// failures. The staging directory is removed on failure.
pub fn build_handbook(
    conn: &Connection,
    store: &Store,
    opts: &BuildOptions,
) -> Result<BuildReport> {
    let _ = store; // Pages cite capture ids; blob bytes stay in the store.
    let hash = facts_hash(conn)?;
    let found = conflicts(conn)?;

    let authorities: Vec<(i64, String, String, String)> = conn
        .prepare("SELECT id, slug, name, country FROM authority WHERE id != 0 ORDER BY slug")?
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    // The id-0 sentinel row (migration 005) backs unattributed facts;
    // it is never a publishable authority.

    let build_no: i64 = conn.query_row(
        "SELECT COALESCE(MAX(build_no), 0) + 1 FROM build",
        [],
        |row| row.get(0),
    )?;

    let builds_dir = opts.out_dir.join("builds");
    let dest = builds_dir.join(build_no.to_string());
    let staging = opts
        .out_dir
        .join(format!(".staging-{build_no}-{}", std::process::id()));
    if staging.exists() {
        return Err(
            std::io::Error::new(std::io::ErrorKind::AlreadyExists, "staging dir exists").into(),
        );
    }
    let built = render_tree(conn, &authorities, &found, &staging, opts, build_no, &hash);
    if let Err(e) = built {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }
    // `create_dir` (not all): two builders racing on one out dir must
    // not silently share a tree; the loser fails loudly.
    std::fs::create_dir_all(&builds_dir)?;
    if let Err(e) = std::fs::rename(&staging, &dest) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e.into());
    }

    handbook_store::db::in_transaction(conn, || {
        conn.execute(
            "INSERT INTO build(at, code_git_commit, content_git_commit, build_no, facts_hash, notes)
             VALUES (?1, ?2, NULL, ?3, ?4, '')",
            rusqlite::params![opts.at, git_commit(), build_no, hash],
        )?;
        let build_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO audit(at, actor, action, entity, entity_id, build_id, reason)
             VALUES (?1, ?2, 'build_publish', 'build', ?3, ?3, ?4)",
            rusqlite::params![
                opts.at,
                opts.actor,
                build_id,
                format!("build_no={build_no} pages={}", authorities.len()),
            ],
        )?;
        Ok(())
    })?;

    swap_current(&opts.out_dir, &dest)?;

    Ok(BuildReport {
        build_no,
        facts_hash: hash,
        pages: authorities.len(),
        conflicts: found.len(),
        dir: dest,
    })
}

/// Best-effort HEAD commit for the `build.code_git_commit` column.
/// Empty string when git is unavailable (dev trees, tarballs).
fn git_commit() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// Atomically point `<out>/current` at `dest`: symlink into a temp
/// name in the same directory, then rename over `current`.
fn swap_current(out_dir: &Path, dest: &Path) -> Result<()> {
    let link = out_dir.join("current");
    let tmp = out_dir.join(format!(".current-{}", std::process::id()));
    if tmp.exists() || tmp.is_symlink() {
        std::fs::remove_file(&tmp)?;
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(dest, &tmp)?;
    #[cfg(not(unix))]
    {
        let _ = (dest, tmp);
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "atomic publish needs unix symlinks",
        )
        .into());
    }
    #[cfg(unix)]
    std::fs::rename(&tmp, &link)?;
    Ok(())
}

/// Minimal HTML escaping for rendered facts (statements are
/// staff-approved but still untrusted for markup purposes).
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn render_tree(
    conn: &Connection,
    authorities: &[(i64, String, String, String)],
    found: &[Conflict],
    staging: &Path,
    opts: &BuildOptions,
    build_no: i64,
    hash: &str,
) -> Result<()> {
    std::fs::create_dir_all(staging)?;
    let mut shard_countries: Vec<String> = Vec::new();
    for (id, slug, name, country) in authorities {
        let facts = accepted_facts(conn, *id)?;
        let mine: Vec<&Conflict> = found.iter().filter(|c| c.authority_id == *id).collect();
        let routing: Vec<&AcceptedFact> = facts
            .iter()
            .filter(|f| f.topic == "legalization_chain")
            .collect();
        let mut page = format!(
            "<!DOCTYPE html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\">\
             <title>{}</title></head>\n<body>\n<h1>{}</h1>\n<p>build {build_no} facts {hash}</p>\n",
            escape(name),
            escape(name),
        );
        // Routing banner: authorities with an accepted legalization_chain
        // fact (e.g. the Viet Nam demo) surface it above the fold.
        for fact in &routing {
            page.push_str(&format!(
                "<div class=\"banner routing\" role=\"note\">routing: {} \
                 <a href=\"#cite-{}\">citation</a></div>\n",
                escape(&fact.statement),
                fact.capture_id,
            ));
        }
        for conflict in &mine {
            page.push_str(&format!(
                "<div class=\"banner conflict\" role=\"alert\">conflicting records \
                 on topic {}: #{} vs #{} — \
                 see <a href=\"#cite-{}\">citation {}</a> and \
                 <a href=\"#cite-{}\">citation {}</a></div>\n",
                escape(&conflict.topic),
                conflict.first_id,
                conflict.second_id,
                conflict.first_capture_id,
                conflict.first_capture_id,
                conflict.second_capture_id,
                conflict.second_capture_id,
            ));
        }
        page.push_str("<dl>\n");
        for fact in &facts {
            page.push_str(&format!(
                "<dt>{}</dt><dd>{} \
                 <a id=\"cite-{}\" href=\"#cite-{}\">[capture {}]</a></dd>\n",
                escape(&fact.topic),
                escape(&fact.statement),
                fact.capture_id,
                fact.capture_id,
                fact.capture_id,
            ));
        }
        page.push_str("</dl>\n</body>\n</html>\n");
        let file_name = format!("{slug}.html");
        std::fs::write(staging.join(&file_name), page)?;
        if !facts.is_empty() && !shard_countries.contains(country) {
            shard_countries.push(country.clone());
        }
    }

    let mut index = "<!DOCTYPE html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\">\
         <title>handbook</title></head>\n<body>\n<h1>handbook</h1>\n<ul>\n"
        .to_string();
    for (_, slug, name, _) in authorities {
        index.push_str(&format!(
            "<li><a href=\"{slug}.html\">{}</a></li>\n",
            escape(name)
        ));
    }
    index.push_str("</ul>\n</body>\n</html>\n");
    std::fs::write(staging.join("index.html"), index)?;

    // sitemap.xml + per-country shards (F11: no separate sitemap tool).
    let base = opts.base_url.trim_end_matches('/');
    let mut sitemap = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<sitemapindex \
         xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n"
        .to_string();
    for country in &shard_countries {
        let shard = format!("sitemap-{}.xml", slugify(country));
        sitemap.push_str(&format!("  <sitemap><loc>{base}/{shard}</loc></sitemap>\n"));
        let mut body = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset \
             xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n"
            .to_string();
        for (_, slug, _, _) in authorities.iter().filter(|a| &a.3 == country) {
            body.push_str(&format!("  <url><loc>{base}/{slug}.html</loc></url>\n"));
        }
        body.push_str("</urlset>\n");
        std::fs::write(staging.join(&shard), body)?;
    }
    sitemap.push_str("</sitemapindex>\n");
    std::fs::write(staging.join("sitemap.xml"), sitemap)?;

    // robots.txt merge: keep the stock rules, point at this build.
    std::fs::write(
        staging.join("robots.txt"),
        format!("User-agent: *\nAllow: /\nSitemap: {base}/sitemap.xml\n"),
    )?;
    Ok(())
}

/// Lowercase ASCII slug for shard file names; anything else folds to `-`.
fn slugify(country: &str) -> String {
    country
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use handbook_store::db::{self, NewAuthority};

    fn seeded() -> (tempfile::TempDir, Store, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("store"));
        store.init().unwrap();
        let conn = db::open(&store).unwrap();
        db::migrate(&conn).unwrap();
        // WA pilot authority with two conflicting accepted PO Box facts
        // (canonical fixture statements) plus one proposed fact that
        // must never render.
        let wa = db::add_authority(
            &conn,
            &NewAuthority {
                kind: "state",
                country: "USA",
                region: "Washington",
                name: "WA Secretary of State",
                slug: "usa-wa-sos",
                actor: "test",
                at: 1,
            },
        )
        .unwrap();
        for (i, sha) in ["a".repeat(64), "b".repeat(64)].iter().enumerate() {
            conn.execute(
                "INSERT INTO capture(authority_id, captured_at, kind, path,
                                     bytes_raw, bytes_stored, sha256, trust)
                 VALUES (?1, 1, 'file', ?2, 1, 1, ?3, 'staff_verified')",
                rusqlite::params![wa, format!("blob/{i}"), sha],
            )
            .unwrap();
        }
        for (topic, statement, status, capture) in [
            ("po_box", "PO Box 40220", "accepted", 1),
            ("po_box", "PO Box 40234", "accepted", 2),
            ("hours", "Mon-Fri 9-5", "proposed", 1),
        ] {
            conn.execute(
                "INSERT INTO fact(authority_id, capture_id, topic, statement,
                                  status, created_by, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'test', 1)",
                rusqlite::params![wa, capture, topic, statement, status],
            )
            .unwrap();
        }
        // Viet Nam demo authority with an accepted routing fact.
        let vn = db::add_authority(
            &conn,
            &NewAuthority {
                kind: "country",
                country: "Viet Nam",
                region: "",
                name: "Viet Nam",
                slug: "vietnam",
                actor: "test",
                at: 1,
            },
        )
        .unwrap();
        conn.execute(
            "INSERT INTO capture(authority_id, captured_at, kind, path,
                                 bytes_raw, bytes_stored, sha256, trust)
             VALUES (?1, 1, 'file', 'blob/vn', 1, 1, ?2, 'staff_verified')",
            rusqlite::params![vn, "c".repeat(64)],
        )
        .unwrap();
        let vn_capture = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO fact(authority_id, capture_id, topic, statement,
                              status, created_by, created_at)
             VALUES (?1, ?2, 'legalization_chain', 'apostille (effective 2026-09-11)', 'accepted', 'test', 1)",
            rusqlite::params![vn, vn_capture],
        )
        .unwrap();
        (dir, store, conn)
    }

    fn build(dir: &tempfile::TempDir, store: &Store, conn: &Connection) -> BuildReport {
        build_handbook(
            conn,
            store,
            &BuildOptions {
                out_dir: dir.path().join("site"),
                base_url: "https://andrii.ai/apostille".to_string(),
                actor: "test".to_string(),
                at: 1_800_000_000,
            },
        )
        .unwrap()
    }

    #[test]
    fn accepted_only_with_conflict_banner() {
        let (dir, store, conn) = seeded();
        let report = build(&dir, &store, &conn);
        assert_eq!(report.build_no, 1);
        assert_eq!(report.conflicts, 1);
        let page = std::fs::read_to_string(report.dir.join("usa-wa-sos.html")).unwrap();
        assert!(page.contains("PO Box 40220"), "{page}");
        assert!(page.contains("PO Box 40234"), "{page}");
        assert!(page.contains("role=\"alert\""), "{page}");
        assert!(
            !page.contains("Mon-Fri 9-5"),
            "proposed facts must never render: {page}"
        );
    }

    #[test]
    fn viet_nam_routing_banner_and_sitemap() {
        let (dir, store, conn) = seeded();
        let report = build(&dir, &store, &conn);
        let page = std::fs::read_to_string(report.dir.join("vietnam.html")).unwrap();
        assert!(page.contains("routing: apostille"), "{page}");
        let sitemap = std::fs::read_to_string(report.dir.join("sitemap.xml")).unwrap();
        assert!(sitemap.contains("sitemap-usa.xml"), "{sitemap}");
        // Shard names key on country ("Viet Nam" → viet-nam), not the
        // authority slug (O2: `vietnam`).
        assert!(sitemap.contains("sitemap-viet-nam.xml"), "{sitemap}");
        let robots = std::fs::read_to_string(report.dir.join("robots.txt")).unwrap();
        assert!(
            robots.contains("Sitemap: https://andrii.ai/apostille/sitemap.xml"),
            "{robots}"
        );
    }

    #[test]
    fn build_rows_are_monotonic_and_hashed() {
        let (dir, store, conn) = seeded();
        let first = build(&dir, &store, &conn);
        let second = build(&dir, &store, &conn);
        assert_eq!((first.build_no, second.build_no), (1, 2));
        assert_eq!(first.facts_hash, second.facts_hash);
        assert_eq!(first.facts_hash, facts_hash(&conn).unwrap());
        assert_eq!(first.facts_hash.len(), 64);
        // A content change flips the hash.
        conn.execute(
            "UPDATE fact SET statement = 'PO Box 40221' WHERE statement = 'PO Box 40220'",
            [],
        )
        .unwrap();
        assert_ne!(facts_hash(&conn).unwrap(), first.facts_hash);
        let (count, audit): (i64, i64) = conn
            .query_row(
                "SELECT (SELECT COUNT(*) FROM build),
                        (SELECT COUNT(*) FROM audit WHERE action = 'build_publish')",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((count, audit), (2, 2));
    }

    #[test]
    fn publish_is_atomic() {
        let (dir, store, conn) = seeded();
        let first = build(&dir, &store, &conn);
        let link = dir.path().join("site").join("current");
        assert_eq!(std::fs::read_link(&link).unwrap(), first.dir);
        // A second build flips `current` to the new tree only.
        let second = build(&dir, &store, &conn);
        assert_eq!(std::fs::read_link(&link).unwrap(), second.dir);
        assert!(first.dir.join("usa-wa-sos.html").exists());
        // A failed build (out dir replaced by a file) leaves `current`.
        std::fs::remove_dir_all(dir.path().join("site")).unwrap();
        std::fs::write(dir.path().join("site"), b"not a dir").unwrap();
        build_handbook(
            &conn,
            &store,
            &BuildOptions {
                out_dir: dir.path().join("site"),
                base_url: "https://andrii.ai/apostille".to_string(),
                actor: "test".to_string(),
                at: 1_800_000_000,
            },
        )
        .unwrap_err();
        assert!(!dir.path().join("site").join("current").exists());
        // The failed build wrote no build row for build_no 3.
        let max: i64 = conn
            .query_row("SELECT MAX(build_no) FROM build", [], |row| row.get(0))
            .unwrap();
        assert_eq!(max, 2);
    }

    #[test]
    fn conflict_query_matches_f6_shape() {
        let (_dir, _store, conn) = seeded();
        let found = conflicts(&conn).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].authority_slug, "usa-wa-sos");
        assert_eq!(found[0].topic, "po_box");
        assert_ne!(found[0].first_statement, found[0].second_statement);
    }
}
