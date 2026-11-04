//! Place pages: "which office here handles my document?" (P2-5, P4-1).
//!
//! A location is a country (`usa`, `mx`, `de`) or a state, province or
//! Land under it (`usa/washington`, `de/by`). Its page lists every
//! public office there grouped by level (federal, state, local, …); a
//! state page also shows the federal offices above it.
//!
//! - **Country segment**: `usa` (alias `us`) for the United States,
//!   else the lowercase ISO 3166-1 code of the rows' `authority.iso2`.
//!   US rows from packs that predate `iso2` match by country name.
//! - **State segment**: for the US the state name (`washington`) or its
//!   postal code (`wa`), from the same reference table the dossier
//!   paths use; elsewhere the ISO 3166-2 suffix of `authority.
//!   subdivision` (`by` for `DE-BY`).
//!
//! **Routes** answer "which office handles my document?": the
//! `authority_scope` rows packs propose (document class, where it was
//! issued, prerequisite, the page it was read at). A state page shows
//! the state's own routes and inherits the federal ones for the rest; a
//! country page shows the federal ones and says which classes go by
//! state. A route is `verified` only once staff accepted a
//! `jurisdiction` fact `doc_rule: <doc_class> …` on that office.
//!
//! `GET /apostille/api/v1/locations/{*path}` (public, model-free, a few
//! indexed reads) answers the page; `GET /apostille/in/{*path}` serves
//! the SPA shell with a crawler head, 404 for a place nothing lists.
//! Hidden rows never appear.

use axum::{
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Json, Response},
};
use rusqlite::Connection;
use serde::Serialize;
use serde_json::json;

use crate::authority_path::{US_STATES, index, normalize, segment_of};
use crate::{AppState, passkey, store_unavailable};

/// Country names US rows carry in packs that predate `iso2`.
const US_NAMES: &str = "'USA', 'US', 'United States', 'United States of America'";

/// One place: its canonical address, label and ISO codes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Location {
    /// Canonical address (`usa`, `usa/washington`, `de/by`).
    pub path: String,
    pub label: String,
    /// ISO 3166-1 alpha-2 (`US`).
    pub iso2: String,
    /// ISO 3166-2 for a state page (`US-WA`), else `None`.
    pub subdivision: Option<String>,
    /// The state's name as `authority.region` spells it.
    #[serde(skip)]
    region: Option<String>,
}

/// One office row on a place page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Office {
    pub slug: String,
    pub name: String,
    /// Dossier address (readable path, else the slug).
    pub path: String,
    pub kind: String,
    pub region: String,
    pub level: String,
    pub fact_count: i64,
}

/// Offices at one level, in display order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Group {
    pub level: String,
    pub label: String,
    pub offices: Vec<Office>,
}

/// The office a route names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RouteOffice {
    pub slug: String,
    pub name: String,
    pub path: String,
}

/// One answer to "which office here handles this document?".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Route {
    pub doc_class: String,
    pub office: RouteOffice,
    pub prereq: String,
    pub source_uri: String,
    /// Accepted `doc_rule` facts behind it (empty while unverified).
    pub fact_ids: Vec<i64>,
    pub verified: bool,
    /// A federal route shown on a state page.
    pub inherited: bool,
}

/// A sub-place one level down (a state under a country).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Place {
    pub path: String,
    pub label: String,
}

fn country_segment(iso2: &str) -> String {
    if iso2 == "US" {
        "usa".to_string()
    } else {
        iso2.to_ascii_lowercase()
    }
}

/// SQL predicate (alias `a`) for "this row is in country `?1`".
fn in_country(iso2: &str) -> String {
    if iso2 == "US" {
        format!("(a.iso2 = ?1 OR (a.iso2 = '' AND a.country IN ({US_NAMES})))")
    } else {
        "a.iso2 = ?1".to_string()
    }
}

/// Resolve an address to a place with at least one public office.
pub fn locate(conn: &Connection, raw: &str) -> Option<Location> {
    let path = normalize(raw)?;
    let segments: Vec<&str> = path.split('/').collect();
    if segments.len() > 2 {
        return None;
    }
    let iso2 = match segments[0] {
        "usa" | "us" => "US".to_string(),
        seg if seg.len() == 2 && seg.bytes().all(|b| b.is_ascii_lowercase()) => {
            seg.to_ascii_uppercase()
        }
        _ => return None,
    };
    let filter = in_country(&iso2);
    let country: String = conn
        .query_row(
            &format!(
                "SELECT a.country FROM authority a
                 WHERE a.id != 0 AND a.hidden_at IS NULL AND {filter}
                 GROUP BY a.country ORDER BY COUNT(*) DESC, a.country LIMIT 1"
            ),
            [&iso2],
            |r| r.get(0),
        )
        .ok()?;
    let country_label = if iso2 == "US" {
        "United States".to_string()
    } else {
        country
    };
    let base = country_segment(&iso2);
    let Some(state) = segments.get(1) else {
        return Some(Location {
            path: base,
            label: country_label,
            iso2,
            subdivision: None,
            region: None,
        });
    };
    let (code, region) = if iso2 == "US" {
        let (name, code) = US_STATES
            .iter()
            .find(|(name, code)| segment_of(name) == *state || code == state)?;
        (
            format!("US-{}", code.to_ascii_uppercase()),
            name.to_string(),
        )
    } else {
        let code = format!("{iso2}-{}", state.to_ascii_uppercase());
        let region: String = conn
            .query_row(
                "SELECT region FROM authority
                 WHERE subdivision = ?1 AND hidden_at IS NULL AND region != ''
                 GROUP BY region ORDER BY COUNT(*) DESC LIMIT 1",
                [&code],
                |r| r.get(0),
            )
            .ok()?;
        (code, region)
    };
    let segment = if iso2 == "US" {
        segment_of(&region)
    } else {
        state.to_string()
    };
    let any: bool = conn
        .query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM authority a
                 WHERE a.id != 0 AND a.hidden_at IS NULL AND {filter}
                   AND (a.subdivision = ?2 OR (a.subdivision = '' AND a.region = ?3)))"
            ),
            rusqlite::params![iso2, code, region],
            |r| r.get(0),
        )
        .unwrap_or(false);
    any.then(|| Location {
        path: format!("{base}/{segment}"),
        label: region.clone(),
        iso2,
        subdivision: Some(code),
        region: Some(region),
    })
}

/// The country place page of a row with this `iso2` / `country`.
pub fn country_place(conn: &Connection, iso2: &str, country: &str) -> Option<Location> {
    let us_name = matches!(
        country.trim().to_ascii_lowercase().as_str(),
        "usa" | "us" | "united states" | "united states of america"
    );
    let segment = if iso2 == "US" || (iso2.is_empty() && us_name) {
        "usa".to_string()
    } else if iso2.len() == 2 {
        iso2.to_ascii_lowercase()
    } else {
        return None;
    };
    locate(conn, &segment)
}

/// The level a row sits at: its stored `level`, else derived from its
/// kind (a state or province row is state level; a nationwide office
/// or country row is federal; an office with a region is local).
fn level_of(level: &str, kind: &str, region: &str) -> &'static str {
    match level {
        "federal" => "federal",
        "state" => "state",
        "regional" => "regional",
        "local" => "local",
        "consular" => "consular",
        _ => match kind {
            "state" | "province" => "state",
            "office" if !region.is_empty() => "local",
            _ => "federal",
        },
    }
}

const LEVEL_ORDER: [&str; 5] = ["federal", "state", "regional", "local", "consular"];

fn level_label(level: &str, federal: bool) -> &'static str {
    match level {
        "federal" if federal => "Federal",
        "federal" => "National",
        "state" => "State",
        "regional" => "Regional",
        "local" => "Local",
        _ => "Consular",
    }
}

/// Every public office of `loc`, grouped by level. A state page lists
/// the state's own offices plus the federal ones above it.
pub fn offices(conn: &Connection, loc: &Location) -> Vec<Group> {
    let filter = in_country(&loc.iso2);
    let Ok(mut stmt) = conn.prepare(&format!(
        "SELECT a.slug, a.name, a.kind, a.region, a.level, a.subdivision,
                (SELECT COUNT(*) FROM fact f WHERE f.authority_id = a.id
                   AND f.status = 'accepted')
         FROM authority a
         WHERE a.id != 0 AND a.hidden_at IS NULL AND {filter}
         ORDER BY a.name"
    )) else {
        return Vec::new();
    };
    type RowTuple = (String, String, String, String, String, String, i64);
    let rows: Vec<RowTuple> = stmt
        .query_map([&loc.iso2], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
            ))
        })
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default();
    let paths = index(conn);
    let federal = rows
        .iter()
        .any(|(_, _, kind, region, level, _, _)| level_of(level, kind, region) == "state");
    let mut groups: Vec<Group> = Vec::new();
    for (slug, name, kind, region, level, subdivision, fact_count) in rows {
        let level = level_of(&level, &kind, &region);
        if let (Some(code), Some(state)) = (&loc.subdivision, &loc.region) {
            let here = subdivision == *code || (subdivision.is_empty() && region == *state);
            if level != "federal" && !here {
                continue;
            }
        }
        let path = paths.primary(&slug);
        let office = Office {
            slug,
            name,
            path,
            kind,
            region,
            level: level.to_string(),
            fact_count,
        };
        match groups.iter_mut().find(|g| g.level == level) {
            Some(group) => group.offices.push(office),
            None => groups.push(Group {
                level: level.to_string(),
                label: level_label(level, federal).to_string(),
                offices: vec![office],
            }),
        }
    }
    groups.sort_by_key(|g| LEVEL_ORDER.iter().position(|l| *l == g.level));
    // On a state page the state's own offices come first.
    if loc.subdivision.is_some() {
        groups.sort_by_key(|g| g.level == "federal");
    }
    groups
}

/// The routes of `loc`: on a state page its own (`issued_in` = the
/// state) plus the national ones for classes the state does not route;
/// on a country page the national ones (`issued_in` = the country or
/// empty). One per document class, own before inherited.
pub fn routes(conn: &Connection, loc: &Location) -> Vec<Route> {
    let filter = in_country(&loc.iso2);
    let Ok(mut stmt) = conn.prepare(&format!(
        "SELECT s.doc_class, s.issued_in, s.prereq, s.source_uri, a.id, a.slug, a.name
         FROM authority_scope s JOIN authority a ON a.id = s.authority_id
         WHERE a.id != 0 AND a.hidden_at IS NULL AND {filter}
         ORDER BY s.id"
    )) else {
        return Vec::new();
    };
    type RouteRow = (String, String, String, String, i64, String, String);
    let rows: Vec<RouteRow> = stmt
        .query_map([&loc.iso2], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
            ))
        })
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default();
    let paths = index(conn);
    let mut out: Vec<Route> = Vec::new();
    for (doc_class, issued_in, prereq, source_uri, id, slug, name) in rows {
        let own = loc.subdivision.as_deref() == Some(issued_in.as_str());
        let national = issued_in.is_empty() || issued_in == loc.iso2;
        if !own && !national {
            continue;
        }
        let inherited = loc.subdivision.is_some() && !own;
        match out.iter().position(|r| r.doc_class == doc_class) {
            Some(i) if out[i].inherited && !inherited => {
                out.remove(i);
            }
            Some(_) => continue,
            None => {}
        }
        let fact_ids = doc_rule_facts(conn, id, &doc_class);
        out.push(Route {
            office: RouteOffice {
                path: paths.primary(&slug),
                slug,
                name,
            },
            verified: !fact_ids.is_empty(),
            fact_ids,
            doc_class,
            prereq,
            source_uri,
            inherited,
        });
    }
    out
}

/// Accepted `jurisdiction` facts on `authority_id` that state the rule
/// for `doc_class` (`doc_rule: <doc_class> …`).
fn doc_rule_facts(conn: &Connection, authority_id: i64, doc_class: &str) -> Vec<i64> {
    let prefix = format!("doc_rule: {doc_class}");
    conn.prepare(
        "SELECT id FROM fact WHERE authority_id = ?1 AND topic = 'jurisdiction'
           AND status = 'accepted' AND substr(statement, 1, length(?2)) = ?2
           AND substr(statement, length(?2) + 1, 1) IN ('', ' ', ':', ';', ',')
         ORDER BY id",
    )
    .and_then(|mut stmt| {
        stmt.query_map(rusqlite::params![authority_id, prefix], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<i64>>>()
    })
    .unwrap_or_default()
}

/// On a country page: the document classes this country routes by
/// state (the answer is "pick the state that issued it").
pub fn by_place(conn: &Connection, loc: &Location) -> Vec<String> {
    if loc.subdivision.is_some() {
        return Vec::new();
    }
    let filter = in_country(&loc.iso2);
    conn.prepare(&format!(
        "SELECT DISTINCT s.doc_class FROM authority_scope s
         JOIN authority a ON a.id = s.authority_id
         WHERE a.id != 0 AND a.hidden_at IS NULL AND {filter}
           AND s.issued_in LIKE ?1 || '-%'
         ORDER BY s.doc_class"
    ))
    .and_then(|mut stmt| {
        stmt.query_map([&loc.iso2], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()
    })
    .unwrap_or_default()
}

/// States (provinces, Länder) under a country page, A→Z.
pub fn places(conn: &Connection, loc: &Location) -> Vec<Place> {
    if loc.subdivision.is_some() {
        return Vec::new();
    }
    let mut out: Vec<Place> = Vec::new();
    for group in offices(conn, loc) {
        if group.level != "state" {
            continue;
        }
        for office in group.offices {
            let segment = if loc.iso2 == "US" {
                US_STATES
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case(office.region.trim()))
                    .map(|(name, _)| segment_of(name))
            } else {
                conn.query_row(
                    "SELECT subdivision FROM authority WHERE slug = ?1",
                    [&office.slug],
                    |r| r.get::<_, String>(0),
                )
                .ok()
                .and_then(|code| {
                    code.split_once('-')
                        .map(|(_, sub)| sub.to_ascii_lowercase())
                })
            };
            let Some(segment) = segment.filter(|s| !s.is_empty()) else {
                continue;
            };
            let path = format!("{}/{segment}", loc.path.split('/').next().unwrap_or(""));
            if !out.iter().any(|p| p.path == path) {
                out.push(Place {
                    path,
                    label: office.region.clone(),
                });
            }
        }
    }
    out.sort_by(|a, b| a.label.cmp(&b.label));
    out
}

/// Breadcrumbs: the country, then the state.
pub fn crumbs(loc: &Location) -> Vec<Place> {
    let mut out = Vec::new();
    let country = loc.path.split('/').next().unwrap_or("").to_string();
    if loc.subdivision.is_some() {
        out.push(Place {
            path: country.clone(),
            label: String::new(),
        });
    }
    out.push(Place {
        path: loc.path.clone(),
        label: loc.label.clone(),
    });
    out
}

/// `GET /apostille/api/v1/locations/{*path}`: `{location, crumbs,
/// groups, places}` or 404 for a place with no public office.
pub async fn location_route(State(state): State<AppState>, Path(path): Path<String>) -> Response {
    let Some(conn) = passkey::open_conn(&state) else {
        return store_unavailable();
    };
    let Some(loc) = locate(&conn, &path) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "no offices listed at this place"})),
        )
            .into_response();
    };
    let mut crumbs = crumbs(&loc);
    if let Some(first) = crumbs.first_mut()
        && first.label.is_empty()
    {
        first.label = locate(&conn, &first.path)
            .map(|l| l.label)
            .unwrap_or_default();
    }
    Json(json!({
        "location": loc,
        "crumbs": crumbs,
        "groups": offices(&conn, &loc),
        "places": places(&conn, &loc),
        "routes": routes(&conn, &loc),
        "by_place": by_place(&conn, &loc),
    }))
    .into_response()
}

/// Inject a place page's head into the SPA shell (title, description,
/// canonical). Pure; every value is escaped.
pub fn render_location(shell: &str, loc: &Location, offices: usize, base: &str) -> String {
    let base = base.trim_end_matches('/');
    let url = format!("{base}/in/{}", loc.path);
    let title = format!("Apostille offices in {} — Apostille Handbook", loc.label);
    let description = format!(
        "{offices} apostille {} in {}, grouped by level, each with its staff-verified dossier.",
        if offices == 1 { "office" } else { "offices" },
        loc.label
    );
    let esc = crate::unfurl::esc;
    let mut page = shell.to_string();
    let desc_tag = format!(
        "<meta name=\"description\" content=\"{}\" />",
        esc(&description)
    );
    if page.contains(crate::unfurl::GEN_DESCRIPTION) {
        page = page.replacen(crate::unfurl::GEN_DESCRIPTION, &desc_tag, 1);
    }
    page.replacen(
        "<head>",
        &format!(
            "<head>\n\t<title>{}</title>\n\t<link rel=\"canonical\" href=\"{}\" />\n\t\
             <meta property=\"og:url\" content=\"{}\" />",
            esc(&title),
            esc(&url),
            esc(&url)
        ),
        1,
    )
}

/// `GET /apostille/in/{*path}`: the SPA shell with the place's head,
/// or the same shell with 404 for a place nothing lists.
pub async fn location_page(State(state): State<AppState>, Path(path): Path<String>) -> Response {
    let shell = tokio::fs::read(&state.spa_index).await.unwrap_or_default();
    let shell = String::from_utf8_lossy(&shell).to_string();
    let Some(conn) = passkey::open_conn(&state) else {
        return store_unavailable();
    };
    let html = [(header::CONTENT_TYPE, "text/html; charset=utf-8")];
    match locate(&conn, &path) {
        Some(loc) => {
            let count: usize = offices(&conn, &loc).iter().map(|g| g.offices.len()).sum();
            (
                html,
                render_location(&shell, &loc, count, &state.notify_base),
            )
                .into_response()
        }
        None => (StatusCode::NOT_FOUND, html, shell).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use handbook_store::{db, store::Store};

    fn store(rows: &[(&str, &str, &str, &str, &str, &str)]) -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.init().unwrap();
        let conn = db::open(&store).unwrap();
        db::migrate(&conn).unwrap();
        for (kind, country, region, name, slug, iso2) in rows {
            let id = db::add_authority(
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
            conn.execute(
                "UPDATE authority SET iso2 = ?1 WHERE id = ?2",
                rusqlite::params![iso2, id],
            )
            .unwrap();
        }
        (dir, conn)
    }

    fn us() -> (tempfile::TempDir, Connection) {
        store(&[
            (
                "office",
                "USA",
                "",
                "U.S. Department of State — Office of Authentications",
                "usdos-authentications",
                "",
            ),
            (
                "state",
                "United States of America",
                "Washington",
                "Washington Secretary of State",
                "usa-wa-sos",
                "US",
            ),
            (
                "state",
                "United States of America",
                "Oregon",
                "Oregon Secretary of State",
                "us-or-sos",
                "US",
            ),
            (
                "state",
                "Mexico",
                "Jalisco",
                "Gobierno del Estado de Jalisco",
                "mx-jalisco-sg",
                "MX",
            ),
            ("office", "", "", "Mail inbox", "mail-inbox", ""),
        ])
    }

    #[test]
    fn usa_is_a_place_with_federal_and_state_offices() {
        let (_dir, conn) = us();
        let loc = locate(&conn, "usa").unwrap();
        assert_eq!(
            (loc.path.as_str(), loc.label.as_str()),
            ("usa", "United States")
        );
        assert_eq!(locate(&conn, "US").unwrap().path, "usa");
        let groups = offices(&conn, &loc);
        let shape: Vec<(&str, &str, Vec<&str>)> = groups
            .iter()
            .map(|g| {
                (
                    g.level.as_str(),
                    g.label.as_str(),
                    g.offices.iter().map(|o| o.slug.as_str()).collect(),
                )
            })
            .collect();
        assert_eq!(
            shape,
            [
                ("federal", "Federal", vec!["usdos-authentications"]),
                ("state", "State", vec!["us-or-sos", "usa-wa-sos"]),
            ]
        );
        let places = places(&conn, &loc);
        let paths: Vec<&str> = places.iter().map(|p| p.path.as_str()).collect();
        assert_eq!(paths, ["usa/oregon", "usa/washington"]);
    }

    #[test]
    fn a_state_page_shows_its_office_and_the_federal_ones() {
        let (_dir, conn) = us();
        let loc = locate(&conn, "usa/wa").unwrap();
        assert_eq!(loc.path, "usa/washington");
        assert_eq!(loc.subdivision.as_deref(), Some("US-WA"));
        let groups = offices(&conn, &loc);
        let slugs: Vec<Vec<&str>> = groups
            .iter()
            .map(|g| g.offices.iter().map(|o| o.slug.as_str()).collect())
            .collect();
        assert_eq!(slugs, [vec!["usa-wa-sos"], vec!["usdos-authentications"]]);
        assert_eq!(locate(&conn, "usa/narnia"), None);
        assert_eq!(locate(&conn, "usa/washington/king"), None);
    }

    #[test]
    fn hidden_rows_and_unknown_places_stay_out() {
        let (_dir, conn) = us();
        conn.execute(
            "UPDATE authority SET hidden_at = 1 WHERE slug = 'us-or-sos'",
            [],
        )
        .unwrap();
        let loc = locate(&conn, "usa").unwrap();
        let all: Vec<String> = offices(&conn, &loc)
            .into_iter()
            .flat_map(|g| g.offices.into_iter().map(|o| o.slug))
            .collect();
        assert!(!all.contains(&"us-or-sos".to_string()), "{all:?}");
        assert_eq!(locate(&conn, "usa/oregon"), None);
        assert_eq!(locate(&conn, "zz"), None);
        let mx = locate(&conn, "mx").unwrap();
        assert_eq!((mx.label.as_str(), mx.iso2.as_str()), ("Mexico", "MX"));
        assert_eq!(offices(&conn, &mx)[0].label, "State");
    }

    fn route(conn: &Connection, slug: &str, class: &str, issued_in: &str) {
        conn.execute(
            "INSERT INTO authority_scope(authority_id, doc_class, issued_in, source_uri)
             SELECT id, ?2, ?3, 'https://www.hcch.net/x' FROM authority WHERE slug = ?1",
            rusqlite::params![slug, class, issued_in],
        )
        .unwrap();
    }

    /// P4-1: a state page answers with its own office for state
    /// documents and inherits the federal office for the rest; a route
    /// is verified only by an accepted `doc_rule` fact.
    #[test]
    fn a_state_page_routes_documents_and_inherits_federal_ones() {
        let (_dir, conn) = us();
        route(&conn, "usdos-authentications", "fbi_check", "US");
        route(&conn, "usdos-authentications", "vital_record", "US");
        route(&conn, "usa-wa-sos", "vital_record", "US-WA");
        route(&conn, "us-or-sos", "vital_record", "US-OR");
        let wa = locate(&conn, "usa/washington").unwrap();
        let answer: Vec<(String, String, bool, bool)> = routes(&conn, &wa)
            .into_iter()
            .map(|r| (r.doc_class, r.office.slug, r.inherited, r.verified))
            .collect();
        assert_eq!(
            answer,
            [
                (
                    "fbi_check".into(),
                    "usdos-authentications".into(),
                    true,
                    false
                ),
                ("vital_record".into(), "usa-wa-sos".into(), false, false),
            ]
        );
        let usa = locate(&conn, "usa").unwrap();
        assert_eq!(by_place(&conn, &usa), ["vital_record"]);
        assert!(by_place(&conn, &wa).is_empty());
        // Staff accept the rule: the route turns verified.
        conn.execute(
            "INSERT INTO fact(authority_id, capture_id, topic, statement, status, created_at)
             SELECT id, 0, 'jurisdiction', 'doc_rule: vital_record issued in US-WA', 'accepted', 1
             FROM authority WHERE slug = 'usa-wa-sos'",
            [],
        )
        .unwrap();
        let vital = routes(&conn, &wa)
            .into_iter()
            .find(|r| r.doc_class == "vital_record")
            .unwrap();
        assert!(vital.verified);
        assert_eq!(vital.fact_ids.len(), 1);
        assert_eq!(vital.office.path, "usa/washington");
    }

    #[test]
    fn the_crawler_head_is_escaped() {
        let loc = Location {
            path: "usa".to_string(),
            label: "<United> \"States\"".to_string(),
            iso2: "US".to_string(),
            subdivision: None,
            region: None,
        };
        let page = render_location(
            "<html><head></head></html>",
            &loc,
            2,
            "https://x/apostille/",
        );
        assert!(page.contains("<link rel=\"canonical\" href=\"https://x/apostille/in/usa\" />"));
        assert!(page.contains("&lt;United&gt; &quot;States&quot;"));
        assert!(!page.contains("<United>"));
    }
}
