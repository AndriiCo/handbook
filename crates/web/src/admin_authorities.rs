//! Staff authorities (P4-2): find, create, edit and hide authorities
//! without a release.
//!
//! Every change goes through the store's audited helpers, so each one
//! leaves an `authority_add` / `authority_update` (before → after) /
//! `authority_hide` / `authority_unhide` row with the staff actor and a
//! reason. Hidden rows leave every public read at once (index, dossier,
//! sitemap, follows, place pages); facts and history stay. A row a pack
//! seeded is brought back in line with its pack on the next pack load,
//! so a lasting correction to such a row also belongs in the pack: the
//! screen marks those rows. Mounted with the other mission admin
//! routers, so the admin IP + host + session + CSRF guards apply.
//! (`/admin/authorities` is the read-only picker index other staff
//! pages use; this screen's API is `/admin/authority-records`.)

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use handbook_store::db;
use rusqlite::{Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{AppState, admin::AdminAuth, passkey, store_unavailable, unix_now};

const KINDS: [&str; 4] = ["country", "state", "province", "office"];
const LEVELS: [&str; 5] = ["federal", "state", "regional", "local", "consular"];
const PAGE: i64 = 200;

pub fn admin_router(state: AppState) -> Router {
    Router::<AppState>::new()
        .route(
            "/apostille/api/v1/admin/authority-records",
            get(list).post(create),
        )
        .route(
            "/apostille/api/v1/admin/authority-records/{slug}",
            get(one).post(edit),
        )
        .route(
            "/apostille/api/v1/admin/authority-records/{slug}/visibility",
            post(visibility),
        )
        .with_state(state)
}

fn err(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

/// One authority as the staff screen shows it.
fn row_json(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let aliases: String = r.get(9)?;
    Ok(json!({
        "slug": r.get::<_, String>(0)?,
        "name": r.get::<_, String>(1)?,
        "kind": r.get::<_, String>(2)?,
        "country": r.get::<_, String>(3)?,
        "region": r.get::<_, String>(4)?,
        "iso2": r.get::<_, String>(5)?,
        "subdivision": r.get::<_, String>(6)?,
        "level": r.get::<_, String>(7)?,
        "name_en": r.get::<_, String>(8)?,
        "aliases": aliases.lines().filter(|a| !a.trim().is_empty()).collect::<Vec<_>>(),
        "hidden_at": r.get::<_, Option<i64>>(10)?,
        "fact_count": r.get::<_, i64>(11)?,
        "from_pack": r.get::<_, bool>(12)?,
    }))
}

const ROW_SQL: &str = "SELECT a.slug, a.name, a.kind, a.country, a.region, a.iso2,
        a.subdivision, a.level, a.name_en, a.aliases, a.hidden_at,
        (SELECT COUNT(*) FROM fact f WHERE f.authority_id = a.id AND f.status = 'accepted'),
        EXISTS(SELECT 1 FROM task t WHERE t.authority_id = a.id AND t.dedup_key LIKE 'pack:%')
     FROM authority a";

#[derive(Debug, Deserialize)]
struct ListQuery {
    #[serde(default)]
    q: String,
    /// `all` (default) | `public` | `hidden`.
    #[serde(default)]
    show: String,
}

/// `GET …/admin/authority-records?q=&show=`: up to 200 rows matching `q`
/// in name, English name, other names, slug, country or region.
async fn list(
    State(state): State<AppState>,
    _auth: AdminAuth,
    Query(query): Query<ListQuery>,
) -> Response {
    let Some(conn) = passkey::open_conn(&state) else {
        return store_unavailable();
    };
    let needle = format!("%{}%", query.q.trim().to_lowercase());
    let show = match query.show.as_str() {
        "public" | "hidden" => query.show.as_str(),
        _ => "all",
    };
    let filter = "a.id != 0
        AND (lower(a.name) LIKE ?1 OR lower(a.slug) LIKE ?1 OR lower(a.country) LIKE ?1
             OR lower(a.region) LIKE ?1 OR lower(a.name_en) LIKE ?1 OR lower(a.aliases) LIKE ?1)
        AND (?2 = 'all' OR (?2 = 'hidden' AND a.hidden_at IS NOT NULL)
             OR (?2 = 'public' AND a.hidden_at IS NULL))";
    let total: i64 = conn
        .query_row(
            &format!("SELECT COUNT(*) FROM authority a WHERE {filter}"),
            rusqlite::params![needle, show],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let rows: Vec<Value> = conn
        .prepare(&format!(
            "{ROW_SQL} WHERE {filter} ORDER BY a.country, a.name LIMIT {PAGE}"
        ))
        .and_then(|mut stmt| {
            stmt.query_map(rusqlite::params![needle, show], row_json)?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap_or_default();
    Json(json!({ "total": total, "authorities": rows })).into_response()
}

/// `GET …/admin/authority-records/{slug}`: the row plus its audit history.
async fn one(
    State(state): State<AppState>,
    _auth: AdminAuth,
    Path(slug): Path<String>,
) -> Response {
    let Some(conn) = passkey::open_conn(&state) else {
        return store_unavailable();
    };
    let Some((id, row)) = find(&conn, &slug) else {
        return err(StatusCode::NOT_FOUND, "unknown authority");
    };
    let history: Vec<Value> = conn
        .prepare(
            "SELECT at, actor, action, before_ref, after_ref, reason FROM audit
             WHERE entity = 'authority' AND entity_id = ?1 ORDER BY id DESC LIMIT 50",
        )
        .and_then(|mut stmt| {
            stmt.query_map([id], |r| {
                Ok(json!({
                    "at": r.get::<_, i64>(0)?,
                    "actor": r.get::<_, String>(1)?,
                    "action": r.get::<_, String>(2)?,
                    "before": r.get::<_, String>(3)?,
                    "after": r.get::<_, String>(4)?,
                    "reason": r.get::<_, String>(5)?,
                }))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap_or_default();
    Json(json!({ "authority": row, "history": history })).into_response()
}

fn find(conn: &Connection, slug: &str) -> Option<(i64, Value)> {
    let id: i64 = conn
        .query_row(
            "SELECT id FROM authority WHERE slug = ?1 AND id != 0",
            [slug],
            |r| r.get(0),
        )
        .optional()
        .ok()
        .flatten()?;
    let row = conn
        .query_row(&format!("{ROW_SQL} WHERE a.id = ?1"), [id], row_json)
        .ok()?;
    Some((id, row))
}

/// Editable fields; every one optional on edit, `slug`/`name`/`kind`
/// required on create.
#[derive(Debug, Default, Deserialize)]
struct Fields {
    slug: Option<String>,
    name: Option<String>,
    kind: Option<String>,
    country: Option<String>,
    region: Option<String>,
    iso2: Option<String>,
    subdivision: Option<String>,
    level: Option<String>,
    name_en: Option<String>,
    aliases: Option<Vec<String>>,
}

impl Fields {
    /// `(column, value)` pairs to write, normalized, or why not.
    fn columns(&self) -> Result<Vec<(&'static str, String)>, String> {
        let mut out: Vec<(&'static str, String)> = Vec::new();
        let trim = |v: &Option<String>| v.as_deref().map(str::trim).map(str::to_string);
        if let Some(name) = trim(&self.name) {
            if let Some(why) = db::authority_name_lint(&name) {
                return Err(format!("name {why}"));
            }
            out.push(("name", name));
        }
        if let Some(kind) = trim(&self.kind) {
            if !KINDS.contains(&kind.as_str()) {
                return Err(format!("kind must be one of {KINDS:?}"));
            }
            out.push(("kind", kind));
        }
        if let Some(country) = trim(&self.country) {
            out.push(("country", country));
        }
        if let Some(region) = trim(&self.region) {
            out.push(("region", region));
        }
        let iso2 = trim(&self.iso2).map(|v| v.to_ascii_uppercase());
        if let Some(iso2) = &iso2 {
            if !(iso2.is_empty()
                || (iso2.len() == 2 && iso2.bytes().all(|b| b.is_ascii_uppercase())))
            {
                return Err("iso2 must be two letters (ISO 3166-1)".to_string());
            }
            out.push(("iso2", iso2.clone()));
        }
        if let Some(sub) = trim(&self.subdivision).map(|v| v.to_ascii_uppercase()) {
            let ok = sub.is_empty()
                || sub.split_once('-').is_some_and(|(country, code)| {
                    country.len() == 2
                        && country.bytes().all(|b| b.is_ascii_uppercase())
                        && (1..=3).contains(&code.len())
                        && code
                            .bytes()
                            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
                });
            if !ok {
                return Err("subdivision must be ISO 3166-2, e.g. US-WA".to_string());
            }
            out.push(("subdivision", sub));
        }
        if let Some(level) = trim(&self.level) {
            if !level.is_empty() && !LEVELS.contains(&level.as_str()) {
                return Err(format!("level must be one of {LEVELS:?}"));
            }
            out.push(("level", level));
        }
        if let Some(name_en) = trim(&self.name_en) {
            out.push(("name_en", name_en));
        }
        if let Some(aliases) = &self.aliases {
            let clean: Vec<String> = aliases
                .iter()
                .map(|a| a.trim().replace('\n', " "))
                .filter(|a| !a.is_empty())
                .collect();
            out.push(("aliases", clean.join("\n")));
        }
        Ok(out)
    }
}

fn valid_slug(slug: &str) -> bool {
    (2..=64).contains(&slug.len())
        && slug.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// `POST …/admin/authority-records`: a new authority (audited add, plus an
/// audited update for the location and search fields).
async fn create(
    State(state): State<AppState>,
    auth: AdminAuth,
    Json(body): Json<Fields>,
) -> Response {
    let Some(conn) = passkey::open_conn(&state) else {
        return store_unavailable();
    };
    let slug = body.slug.as_deref().unwrap_or("").trim().to_string();
    if !valid_slug(&slug) {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "slug must be 2-64 characters of a-z, 0-9 and -",
        );
    }
    if body.name.as_deref().is_none_or(|n| n.trim().is_empty())
        || body.kind.as_deref().is_none_or(|k| k.trim().is_empty())
    {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "name and kind are required",
        );
    }
    let columns = match body.columns() {
        Ok(columns) => columns,
        Err(why) => return err(StatusCode::UNPROCESSABLE_ENTITY, why),
    };
    if find(&conn, &slug).is_some() {
        return err(StatusCode::CONFLICT, "an authority with this slug exists");
    }
    let get = |col: &str| {
        columns
            .iter()
            .find(|(c, _)| *c == col)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    let actor = format!("staff:{}", auth.user.user_id);
    let now = unix_now();
    let added = db::add_authority(
        &conn,
        &db::NewAuthority {
            kind: &get("kind"),
            country: &get("country"),
            region: &get("region"),
            name: &get("name"),
            slug: &slug,
            actor: &actor,
            at: now,
        },
    );
    let Ok(id) = added else {
        return err(StatusCode::CONFLICT, "could not add this authority");
    };
    let rest: Vec<(&str, &str)> = columns
        .iter()
        .filter(|(c, v)| !matches!(*c, "kind" | "country" | "region" | "name") && !v.is_empty())
        .map(|(c, v)| (*c, v.as_str()))
        .collect();
    if !rest.is_empty()
        && db::in_transaction(&conn, || db::edit_authority(&conn, id, &rest, &actor, now)).is_err()
    {
        return err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "added, but the location fields failed",
        );
    }
    match find(&conn, &slug) {
        Some((_, row)) => (StatusCode::CREATED, Json(json!({ "authority": row }))).into_response(),
        None => store_unavailable(),
    }
}

/// `POST …/admin/authority-records/{slug}`: change any editable field; one
/// audited update row (before → after) per save.
async fn edit(
    State(state): State<AppState>,
    auth: AdminAuth,
    Path(slug): Path<String>,
    Json(body): Json<Fields>,
) -> Response {
    let Some(conn) = passkey::open_conn(&state) else {
        return store_unavailable();
    };
    let Some((id, _)) = find(&conn, &slug) else {
        return err(StatusCode::NOT_FOUND, "unknown authority");
    };
    if body.slug.as_deref().is_some_and(|s| s.trim() != slug) {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "the slug is the key of every table and link; it never changes",
        );
    }
    let columns = match body.columns() {
        Ok(columns) => columns,
        Err(why) => return err(StatusCode::UNPROCESSABLE_ENTITY, why),
    };
    let fields: Vec<(&str, &str)> = columns.iter().map(|(c, v)| (*c, v.as_str())).collect();
    let actor = format!("staff:{}", auth.user.user_id);
    let changed = db::in_transaction(&conn, || {
        db::edit_authority(&conn, id, &fields, &actor, unix_now())
    });
    match (changed, find(&conn, &slug)) {
        (Ok(changed), Some((_, row))) => {
            Json(json!({ "authority": row, "changed": changed })).into_response()
        }
        _ => store_unavailable(),
    }
}

#[derive(Debug, Deserialize)]
struct Visibility {
    hidden: bool,
    #[serde(default)]
    reason: String,
}

/// `POST …/admin/authority-records/{slug}/visibility`: hide (or show again)
/// with a reason; hiding closes the row's open pack tasks.
async fn visibility(
    State(state): State<AppState>,
    auth: AdminAuth,
    Path(slug): Path<String>,
    Json(body): Json<Visibility>,
) -> Response {
    let Some(conn) = passkey::open_conn(&state) else {
        return store_unavailable();
    };
    let reason = body.reason.trim();
    if reason.is_empty() || reason.len() > 300 {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "say why in a few words (300 characters at most)",
        );
    }
    let Some((id, row)) = find(&conn, &slug) else {
        return err(StatusCode::NOT_FOUND, "unknown authority");
    };
    // A page fragment never goes public again under its fragment name.
    if !body.hidden
        && let Some(why) = db::authority_name_lint(row["name"].as_str().unwrap_or_default())
    {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("rename it first: the name {why}"),
        );
    }
    let actor = format!("staff:{}", auth.user.user_id);
    let changed = db::in_transaction(&conn, || {
        db::set_authority_hidden(&conn, id, body.hidden, &actor, unix_now(), reason)
    });
    match (changed, find(&conn, &slug)) {
        (Ok(changed), Some((_, row))) => {
            Json(json!({ "authority": row, "changed": changed })).into_response()
        }
        _ => store_unavailable(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_normalize_and_refuse_bad_values() {
        let ok = Fields {
            name: Some(" Bavarian State Ministry of Justice ".into()),
            iso2: Some("de".into()),
            subdivision: Some("de-by".into()),
            level: Some("state".into()),
            aliases: Some(vec![
                "Bayern".into(),
                " ".into(),
                "Freistaat\nBayern".into(),
            ]),
            ..Fields::default()
        };
        let columns = ok.columns().unwrap();
        assert!(columns.contains(&("iso2", "DE".to_string())));
        assert!(columns.contains(&("subdivision", "DE-BY".to_string())));
        assert!(columns.contains(&("aliases", "Bayern\nFreistaat Bayern".to_string())));
        for bad in [
            Fields {
                name: Some("Land".into()),
                ..Fields::default()
            },
            Fields {
                kind: Some("planet".into()),
                ..Fields::default()
            },
            Fields {
                iso2: Some("DEU".into()),
                ..Fields::default()
            },
            Fields {
                subdivision: Some("Bavaria".into()),
                ..Fields::default()
            },
            Fields {
                level: Some("galactic".into()),
                ..Fields::default()
            },
        ] {
            assert!(bad.columns().is_err(), "{bad:?}");
        }
        assert!(valid_slug("de-by-stmj"));
        assert!(!valid_slug("DE"));
        assert!(!valid_slug("-x"));
        assert!(!valid_slug("a/b"));
    }
}
