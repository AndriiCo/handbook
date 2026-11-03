//! Readable dossier addresses: `usa/washington` for `usa-wa-sos`.
//!
//! URL ONLY. The internal slug stays the key of every table, API route
//! and WS topic (`authority:usa-wa-sos`), so nothing that stores or
//! subscribes changes. A path is an address on top:
//!
//! - **Derived**: a US state authority (kind `state`, country USA,
//!   region a state name) gets `usa/<state>` as its primary path and
//!   `us/<code>`, `usa/<code>`, `us/<state>` as aliases. The state table
//!   below is reference data (names and postal codes), not facts.
//! - **Explicit** rows in `authority_path` (CREATE TABLE IF NOT EXISTS)
//!   for everything else, including a third level where steps differ
//!   below a state: `usa/new_york/chemung` for a county clerk.
//! - The bare slug always resolves too.
//!
//! Every address answers with the same dossier; its canonical link is
//! the primary path. No HTTP redirect: a rollback binary has no
//! multi-segment route, and a browser-cached 301 would strand users.

use std::collections::HashMap;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{AppState, store_unavailable};

/// Deepest path: country / state / county.
pub const MAX_SEGMENTS: usize = 3;
const MAX_SEGMENT_LEN: usize = 48;

/// US states, DC and the territories with their own apostille
/// authority: (name as `authority.region` spells it, postal code).
const US_STATES: &[(&str, &str)] = &[
    ("Alabama", "al"),
    ("Alaska", "ak"),
    ("Arizona", "az"),
    ("Arkansas", "ar"),
    ("California", "ca"),
    ("Colorado", "co"),
    ("Connecticut", "ct"),
    ("Delaware", "de"),
    ("District of Columbia", "dc"),
    ("Florida", "fl"),
    ("Georgia", "ga"),
    ("Hawaii", "hi"),
    ("Idaho", "id"),
    ("Illinois", "il"),
    ("Indiana", "in"),
    ("Iowa", "ia"),
    ("Kansas", "ks"),
    ("Kentucky", "ky"),
    ("Louisiana", "la"),
    ("Maine", "me"),
    ("Maryland", "md"),
    ("Massachusetts", "ma"),
    ("Michigan", "mi"),
    ("Minnesota", "mn"),
    ("Mississippi", "ms"),
    ("Missouri", "mo"),
    ("Montana", "mt"),
    ("Nebraska", "ne"),
    ("Nevada", "nv"),
    ("New Hampshire", "nh"),
    ("New Jersey", "nj"),
    ("New Mexico", "nm"),
    ("New York", "ny"),
    ("North Carolina", "nc"),
    ("North Dakota", "nd"),
    ("Ohio", "oh"),
    ("Oklahoma", "ok"),
    ("Oregon", "or"),
    ("Pennsylvania", "pa"),
    ("Puerto Rico", "pr"),
    ("Rhode Island", "ri"),
    ("South Carolina", "sc"),
    ("South Dakota", "sd"),
    ("Tennessee", "tn"),
    ("Texas", "tx"),
    ("Utah", "ut"),
    ("Vermont", "vt"),
    ("Virginia", "va"),
    ("Washington", "wa"),
    ("West Virginia", "wv"),
    ("Wisconsin", "wi"),
    ("Wyoming", "wy"),
];

fn is_usa(country: &str) -> bool {
    matches!(
        country.trim().to_ascii_lowercase().as_str(),
        "usa" | "us" | "united states" | "united states of america"
    )
}

/// `New York` → `new_york`: lowercase ASCII words joined by `_`.
pub fn segment_of(label: &str) -> String {
    let mut out = String::new();
    for word in label
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
    {
        if !out.is_empty() {
            out.push('_');
        }
        out.push_str(&word.to_ascii_lowercase());
    }
    out
}

/// Canonical form of a typed or linked path: lowercase, no empty
/// segments, 1-3 segments of `[a-z0-9_-]`. `None` when it cannot be a
/// path at all (no lookup is made for it).
pub fn normalize(raw: &str) -> Option<String> {
    let lowered = raw.trim().to_ascii_lowercase();
    let segments: Vec<&str> = lowered
        .split('/')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let ok = !segments.is_empty()
        && segments.len() <= MAX_SEGMENTS
        && segments.iter().all(|s| {
            s.len() <= MAX_SEGMENT_LEN
                && s.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        });
    ok.then(|| segments.join("/"))
}

pub fn migrate_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS authority_path (
            path TEXT PRIMARY KEY,
            authority_slug TEXT NOT NULL,
            is_primary INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE UNIQUE INDEX IF NOT EXISTS authority_path_one_primary
            ON authority_path(authority_slug) WHERE is_primary = 1;",
    )
}

/// One authority's addresses.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Paths {
    /// The canonical address (the slug when nothing better exists).
    pub primary: String,
    /// Every other address that resolves here (the slug included when
    /// it is not the primary).
    pub aliases: Vec<String>,
}

struct Row {
    slug: String,
    kind: String,
    country: String,
    region: String,
}

fn rows(conn: &Connection) -> rusqlite::Result<Vec<Row>> {
    let mut stmt = conn
        .prepare("SELECT slug, kind, country, region FROM authority WHERE id != 0 ORDER BY id")?;
    stmt.query_map([], |r| {
        Ok(Row {
            slug: r.get(0)?,
            kind: r.get(1)?,
            country: r.get(2)?,
            region: r.get(3)?,
        })
    })?
    .collect()
}

fn explicit(conn: &Connection) -> Vec<(String, String, bool)> {
    let has_table = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'authority_path'",
            [],
            |_| Ok(()),
        )
        .optional()
        .ok()
        .flatten()
        .is_some();
    if !has_table {
        return Vec::new();
    }
    conn.prepare("SELECT path, authority_slug, is_primary FROM authority_path ORDER BY path")
        .and_then(|mut stmt| {
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? == 1)))?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap_or_default()
}

/// Every authority's addresses, slug → [`Paths`]. Fail-soft: a store
/// problem yields an empty map, never an error the index would show.
pub fn all_paths(conn: &Connection) -> HashMap<String, Paths> {
    let Ok(rows) = rows(conn) else {
        return HashMap::new();
    };
    let mut out: HashMap<String, Paths> = HashMap::new();
    // Every address already claimed (a derived path never shadows an
    // explicit one, and the first state authority in a state wins).
    let mut taken: HashMap<String, String> = HashMap::new();
    let explicit = explicit(conn);
    for (path, slug, _) in &explicit {
        taken.insert(path.clone(), slug.clone());
    }
    for row in &rows {
        out.insert(row.slug.clone(), Paths::default());
    }
    for (path, slug, primary) in &explicit {
        let Some(entry) = out.get_mut(slug) else {
            continue;
        };
        if *primary && entry.primary.is_empty() {
            entry.primary = path.clone();
        } else {
            entry.aliases.push(path.clone());
        }
    }
    for row in &rows {
        if row.kind != "state" || !is_usa(&row.country) {
            continue;
        }
        let Some((name, code)) = US_STATES
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(row.region.trim()))
        else {
            continue;
        };
        let state = segment_of(name);
        let derived = [
            format!("usa/{state}"),
            format!("us/{code}"),
            format!("usa/{code}"),
            format!("us/{state}"),
        ];
        if taken.contains_key(&derived[0]) {
            continue;
        }
        let entry = out.entry(row.slug.clone()).or_default();
        for (i, path) in derived.into_iter().enumerate() {
            if taken.contains_key(&path) {
                continue;
            }
            taken.insert(path.clone(), row.slug.clone());
            if i == 0 && entry.primary.is_empty() {
                entry.primary = path;
            } else {
                entry.aliases.push(path);
            }
        }
    }
    for (slug, entry) in out.iter_mut() {
        if entry.primary.is_empty() {
            entry.primary = slug.clone();
        } else if !entry.aliases.contains(slug) {
            entry.aliases.push(slug.clone());
        }
        entry.aliases.retain(|a| a != &entry.primary);
    }
    out
}

/// What an address resolves to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Resolved {
    pub slug: String,
    /// The canonical address for this authority.
    pub canonical: String,
    pub aliases: Vec<String>,
}

/// Resolve any address (path, alias or bare slug) to its authority.
pub fn resolve(conn: &Connection, raw: &str) -> Option<Resolved> {
    let path = normalize(raw)?;
    let paths = all_paths(conn);
    let slug = paths
        .iter()
        .find(|(_, p)| p.primary == path || p.aliases.contains(&path))
        .map(|(slug, _)| slug.clone())?;
    let entry = paths.get(&slug)?;
    Some(Resolved {
        canonical: entry.primary.clone(),
        aliases: entry.aliases.clone(),
        slug,
    })
}

/// Breadcrumbs for a canonical path: one per prefix, linked when some
/// authority's canonical address is that prefix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Crumb {
    pub path: String,
    pub label: String,
    pub slug: Option<String>,
}

pub fn crumbs(conn: &Connection, canonical: &str) -> Vec<Crumb> {
    let paths = all_paths(conn);
    let names: HashMap<String, String> = conn
        .prepare("SELECT slug, name FROM authority")
        .and_then(|mut stmt| {
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<HashMap<_, _>>>()
        })
        .unwrap_or_default();
    let segments: Vec<&str> = canonical.split('/').collect();
    let mut out = Vec::new();
    for depth in 1..=segments.len() {
        let prefix = segments[..depth].join("/");
        let owner = paths
            .iter()
            .find(|(_, p)| p.primary == prefix)
            .map(|(slug, _)| slug.clone());
        let label = match (&owner, depth) {
            (Some(slug), _) => names.get(slug).cloned().unwrap_or_else(|| slug.clone()),
            (None, 1) if prefix == "usa" || prefix == "us" => "United States".to_string(),
            (None, 2) if segments[0] == "usa" || segments[0] == "us" => US_STATES
                .iter()
                .find(|(name, code)| segment_of(name) == segments[1] || *code == segments[1])
                .map_or_else(
                    || segments[1].replace('_', " "),
                    |(name, _)| name.to_string(),
                ),
            (None, _) => segments[depth - 1].replace('_', " "),
        };
        out.push(Crumb {
            path: prefix,
            label,
            slug: owner,
        });
    }
    out
}

#[derive(Debug, Deserialize)]
pub struct ResolveQuery {
    pub path: String,
}

/// `GET /apostille/api/v1/authorities/resolve?path=us/wa` (public,
/// model-free, one indexed read): `{slug, canonical, aliases, crumbs}`
/// or 404 for an address nothing answers to.
pub async fn resolve_route(
    State(state): State<AppState>,
    Query(q): Query<ResolveQuery>,
) -> Response {
    let Some(conn) = crate::passkey::open_conn(&state) else {
        return store_unavailable();
    };
    match resolve(&conn, &q.path) {
        Some(found) => {
            let crumbs = crumbs(&conn, &found.canonical);
            Json(json!({
                "slug": found.slug,
                "canonical": found.canonical,
                "aliases": found.aliases,
                "crumbs": crumbs,
            }))
            .into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "no authority at this address"})),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use handbook_store::{db, store::Store};

    fn store() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.init().unwrap();
        let conn = db::open(&store).unwrap();
        db::migrate(&conn).unwrap();
        migrate_tables(&conn).unwrap();
        for (kind, country, region, name, slug) in [
            (
                "state",
                "USA",
                "Washington",
                "Synthetic WA SoS",
                "usa-wa-sos",
            ),
            (
                "state",
                "United States",
                "New York",
                "Synthetic NY DoS",
                "usa-ny-dos",
            ),
            (
                "office",
                "USA",
                "New York",
                "Synthetic Chemung County Clerk",
                "usa-ny-chemung-clerk",
            ),
            (
                "country",
                "Viet Nam",
                "",
                "Synthetic Viet Nam MOFA",
                "vietnam",
            ),
        ] {
            db::add_authority(
                &conn,
                &db::NewAuthority {
                    kind,
                    country,
                    region,
                    name,
                    slug,
                    actor: "test",
                    at: 0,
                },
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO authority_path(path, authority_slug, is_primary)
             VALUES ('usa/new_york/chemung', 'usa-ny-chemung-clerk', 1),
                    ('us/ny/chemung', 'usa-ny-chemung-clerk', 0)",
            [],
        )
        .unwrap();
        (dir, conn)
    }

    #[test]
    fn paths_normalize_or_refuse() {
        assert_eq!(
            normalize(" /USA//Washington/ ").as_deref(),
            Some("usa/washington")
        );
        assert_eq!(normalize("usa-wa-sos").as_deref(), Some("usa-wa-sos"));
        assert_eq!(normalize("a/b/c/d"), None);
        assert_eq!(normalize("usa/wash ington"), None);
        assert_eq!(normalize("../etc"), None);
        assert_eq!(normalize(""), None);
        assert_eq!(segment_of("New York"), "new_york");
        assert_eq!(segment_of("District of Columbia"), "district_of_columbia");
    }

    #[test]
    fn states_derive_paths_and_every_alias_resolves_to_the_slug() {
        let (_dir, conn) = store();
        let wa = all_paths(&conn).remove("usa-wa-sos").unwrap();
        assert_eq!(wa.primary, "usa/washington");
        for alias in ["us/wa", "usa/wa", "us/washington", "usa-wa-sos"] {
            assert!(wa.aliases.contains(&alias.to_string()), "{alias}");
            let found = resolve(&conn, alias).unwrap();
            assert_eq!(found.slug, "usa-wa-sos", "{alias}");
            assert_eq!(found.canonical, "usa/washington");
        }
        assert_eq!(resolve(&conn, "US/NY").unwrap().slug, "usa-ny-dos");
        // Three levels from explicit rows; the county is not a state.
        let county = resolve(&conn, "us/ny/chemung").unwrap();
        assert_eq!(county.slug, "usa-ny-chemung-clerk");
        assert_eq!(county.canonical, "usa/new_york/chemung");
        // A country keeps its slug as its address.
        let vn = resolve(&conn, "vietnam").unwrap();
        assert_eq!(
            (vn.slug.as_str(), vn.canonical.as_str()),
            ("vietnam", "vietnam")
        );
        assert_eq!(resolve(&conn, "usa/oregon"), None);
    }

    #[test]
    fn crumbs_link_the_prefixes_that_are_dossiers() {
        let (_dir, conn) = store();
        let crumbs = crumbs(&conn, "usa/new_york/chemung");
        let labels: Vec<&str> = crumbs.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "United States",
                "Synthetic NY DoS",
                "Synthetic Chemung County Clerk"
            ]
        );
        assert_eq!(crumbs[0].slug, None);
        assert_eq!(crumbs[1].slug.as_deref(), Some("usa-ny-dos"));
    }

    #[test]
    fn a_store_without_the_table_still_derives_state_paths() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.init().unwrap();
        let conn = db::open(&store).unwrap();
        db::migrate(&conn).unwrap();
        db::add_authority(
            &conn,
            &db::NewAuthority {
                kind: "state",
                country: "USA",
                region: "Washington",
                name: "Synthetic WA SoS",
                slug: "usa-wa-sos",
                actor: "test",
                at: 0,
            },
        )
        .unwrap();
        assert_eq!(resolve(&conn, "us/wa").unwrap().canonical, "usa/washington");
    }
}
