//! `GET /apostille/api/v1/admin/stats` — the admin dashboard's one
//! read (B3), shaped as `docs/lanes/contracts/admin-stats.json`: KPIs,
//! Meta telemetry tiles, two series and the recent activity feed.
//! Staff-only, model-free. Tables a lane has not landed yet count as
//! zero, so the dashboard renders on any store.

use axum::{
    Router,
    extract::State,
    response::{IntoResponse, Json, Response},
    routing::get,
};
use handbook_store::db;
use rusqlite::Connection;
use serde_json::json;

use crate::admin::AdminAuth;
use crate::{AppState, store_unavailable, unix_now};

/// `SELECT COUNT(*)`-style scalar, or 0 when the table is missing.
fn scalar(conn: &Connection, sql: &str, params: impl rusqlite::Params) -> i64 {
    conn.query_row(sql, params, |r| r.get::<_, Option<i64>>(0))
        .ok()
        .flatten()
        .unwrap_or(0)
}

/// Human line for one audit row (no staff identity on the dashboard).
fn activity_text(action: &str, entity: &str, id: i64) -> String {
    match action {
        "fact_accept" => format!("Fact #{id} accepted"),
        "fact_reject" => format!("Fact #{id} rejected"),
        "config_set" => "Config value updated".to_string(),
        other => format!("{} {entity} #{id}", other.replace(['_', '.'], " ")),
    }
}

fn activity_href(entity: &str, id: i64) -> Option<String> {
    match entity {
        "fact" => Some(format!("/review?fact={id}")),
        "config" => Some("/config".to_string()),
        _ => None,
    }
}

/// The dashboard body for `now` (unit-tested without HTTP).
pub fn stats(
    conn: &Connection,
    p2: Option<&Connection>,
    store_root: &std::path::Path,
    now: i64,
) -> serde_json::Value {
    let day = 86_400;
    let today = now - now.rem_euclid(day);
    let kpis = json!({
        "authorities": scalar(conn, "SELECT COUNT(*) FROM authority WHERE id != 0", []),
        "facts_accepted": scalar(conn, "SELECT COUNT(*) FROM fact WHERE status = 'accepted'", []),
        "proposals_open": scalar(conn, "SELECT COUNT(*) FROM fact WHERE status = 'proposed'", []),
        "plans_accepted": scalar(conn, "SELECT COUNT(*) FROM page_plan WHERE status = 'accepted'", []),
        "offices_accepted": scalar(conn, "SELECT COUNT(*) FROM office_record WHERE status = 'accepted'", []),
        "forms_active": scalar(conn, "SELECT COUNT(*) FROM form_template WHERE status = 'active'", []),
        "orders_today": p2.map_or(0, |p2| scalar(p2, "SELECT COUNT(*) FROM customer_order WHERE created_at >= ?1", [today])),
        "paid_cents_7d": p2.map_or(0, |p2| scalar(
            p2,
            "SELECT SUM(json_extract(amount_json, '$.total_cents')) FROM customer_order
             WHERE created_at >= ?1 AND status NOT IN ('draft', 'quoted', 'awaiting_payment', 'canceled', 'refunded')",
            [now - 7 * day],
        )),
        "needs_staff": p2.map_or(0, |p2| scalar(p2, "SELECT COUNT(*) FROM customer_order WHERE status = 'needs_staff'", [])),
    });

    let since = now - day;
    let calls = scalar(
        conn,
        "SELECT COUNT(*) FROM model_call WHERE at >= ?1",
        [since],
    );
    let errors = scalar(
        conn,
        "SELECT COUNT(*) FROM model_call WHERE at >= ?1 AND outcome != 'ok'",
        [since],
    );
    let input = scalar(
        conn,
        "SELECT SUM(input_tokens) FROM model_call WHERE at >= ?1",
        [since],
    );
    let cached = scalar(
        conn,
        "SELECT SUM(cached_tokens) FROM model_call WHERE at >= ?1",
        [since],
    );
    let cache_share = if input > 0 {
        ((cached as f64 / input as f64) * 100.0).round() / 100.0
    } else {
        0.0
    };
    let by_pipeline: Vec<serde_json::Value> = conn
        .prepare("SELECT pipeline, COUNT(*) FROM model_call WHERE at >= ?1 GROUP BY pipeline ORDER BY COUNT(*) DESC")
        .and_then(|mut s| {
            s.query_map([since], |r| Ok(json!({"pipeline": r.get::<_, String>(0)?, "calls": r.get::<_, i64>(1)?})))?
                .collect()
        })
        .unwrap_or_default();
    // A5 grade: `xtask eval` writes quality.json at the store root.
    let a5 = std::fs::read_to_string(store_root.join("quality.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .map(|q| {
            json!({
                "quote_found_rate": q["quote_found_rate"],
                "value_in_quote_rate": q["value_in_quote_rate"],
                "topic_accuracy": q["topic_accuracy"],
                "cases": q["cases"],
                "graded_at": q["graded_at"],
            })
        })
        .unwrap_or(serde_json::Value::Null);

    let calls_by_hour: Vec<serde_json::Value> = (0..12)
        .map(|i| {
            let end = now - i * 3_600;
            json!({"t": end, "n": scalar(conn, "SELECT COUNT(*) FROM model_call WHERE at > ?1 AND at <= ?2", [end - 3_600, end])})
        })
        .collect();
    let orders_by_day: Vec<serde_json::Value> = (0..7)
        .map(|i| {
            let start = today - i * day;
            let (n, cents) = p2.map_or((0, 0), |p2| {
                (
                    scalar(p2, "SELECT COUNT(*) FROM customer_order WHERE created_at >= ?1 AND created_at < ?2", [start, start + day]),
                    scalar(
                        p2,
                        "SELECT SUM(json_extract(amount_json, '$.total_cents')) FROM customer_order
                         WHERE created_at >= ?1 AND created_at < ?2
                           AND status NOT IN ('draft', 'quoted', 'awaiting_payment', 'canceled', 'refunded')",
                        [start, start + day],
                    ),
                )
            });
            json!({"t": start, "n": n, "cents": cents})
        })
        .collect();

    let activity: Vec<serde_json::Value> = conn
        .prepare("SELECT at, action, entity, entity_id FROM audit ORDER BY id DESC LIMIT 12")
        .and_then(|mut s| {
            s.query_map([], |r| {
                let (at, action, entity, id): (i64, String, String, i64) =
                    (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?);
                Ok(json!({
                    "at": at,
                    "kind": action,
                    "text": activity_text(&action, &entity, id),
                    "href": activity_href(&entity, id),
                }))
            })?
            .collect()
        })
        .unwrap_or_default();

    json!({
        "generated_at": now,
        "kpis": kpis,
        "meta": {
            "calls_24h": calls,
            "errors_24h": errors,
            "cache_share": cache_share,
            "by_model": crate::telemetry::by_model(conn, since).unwrap_or_default(),
            "by_pipeline": by_pipeline,
            "a5": a5,
        },
        "series": {"calls_by_hour": calls_by_hour, "orders_by_day": orders_by_day},
        "activity": activity,
    })
}

async fn get_stats(_auth: AdminAuth, State(state): State<AppState>) -> Response {
    let store = handbook_store::store::Store::new(&state.store_root);
    let Ok(conn) = db::open(&store) else {
        return store_unavailable();
    };
    let p2 = db::open_p2(&store).ok();
    Json(stats(&conn, p2.as_ref(), store.root(), unix_now())).into_response()
}

/// Staff route on the ADMIN listener only.
pub fn admin_router(state: AppState) -> Router {
    Router::<AppState>::new()
        .route("/apostille/api/v1/admin/stats", get(get_stats))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_render_on_a_fresh_store_and_match_the_contract_keys() {
        let dir = std::env::temp_dir().join(format!("hb-stats-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = handbook_store::store::Store::new(&dir);
        store.init().unwrap();
        let conn = db::open(&store).unwrap();
        db::migrate(&conn).unwrap();
        crate::telemetry::migrate_tables(&conn).unwrap();
        let p2 = db::open_p2(&store).unwrap();
        db::migrate_p2(&p2).unwrap();
        let v = stats(&conn, Some(&p2), store.root(), 1_793_750_400);
        let seed: serde_json::Value = serde_json::from_str(include_str!(
            "../../../docs/lanes/contracts/admin-stats.json"
        ))
        .unwrap();
        for key in seed["kpis"].as_object().unwrap().keys() {
            assert!(v["kpis"].get(key).is_some(), "kpi {key}");
        }
        for key in [
            "calls_24h",
            "errors_24h",
            "cache_share",
            "by_model",
            "by_pipeline",
            "a5",
        ] {
            assert!(v["meta"].get(key).is_some(), "meta {key}");
        }
        assert_eq!(v["series"]["calls_by_hour"].as_array().unwrap().len(), 12);
        assert_eq!(v["series"]["orders_by_day"].as_array().unwrap().len(), 7);
        assert_eq!(
            v["kpis"]["plans_accepted"], 0,
            "missing lane table counts as zero"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn activity_lines_name_no_staff() {
        assert_eq!(activity_text("fact_accept", "fact", 7), "Fact #7 accepted");
        assert_eq!(
            activity_text("plan.accept", "page_plan", 3),
            "plan accept page_plan #3"
        );
        assert_eq!(activity_href("fact", 7).as_deref(), Some("/review?fact=7"));
    }
}
