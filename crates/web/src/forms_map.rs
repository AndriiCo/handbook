//! `FormMapV1`: Spark maps an official PDF's AcroForm fields to labels,
//! help and prefill topics, grounded in the lopdf text layer; staff
//! accept the map; the customer fill is fill-and-flatten of the
//! official PDF.
//!
//! Next: prefill from accepted facts, the `form_template` bridge, and a
//! redraw for flat PDFs.
//!
//! The model never invents a field: names are an enum of the PDF's own
//! AcroForm names, every label must be quoted from the PDF's text layer
//! (`label_quote`), and the filled PDF is the official one with values
//! drawn in, not a re-creation.

use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock};

use axum::{
    Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Json, Response},
    routing::{get, post},
};
use handbook_agent::model::ChatJsonRequest;
use handbook_store::{db, store::Store};
use rusqlite::{Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::admin::AdminAuth;
use crate::plan::Check;
use crate::{AppState, store_unavailable, unix_now};

pub const FORMS_MODEL: &str = "muse-spark-1.3";
const PROMPT: &str = include_str!("../prompts/forms-v1.md");
const PER_RUN_MARKER: &str = "<!-- PER-RUN -->";
const TYPES: &[&str] = &["text", "date", "number", "checkbox", "select"];
/// Function words a lone label quote may not be (they ground nothing).
const STOPWORDS: &[&str] = &[
    "the", "a", "an", "of", "to", "and", "or", "for", "in", "on", "by", "at", "is", "be", "if",
    "no", "yes", "any", "all", "not", "are", "your", "this", "that", "with", "from", "please",
    "form",
];
const MAX_FILL_VALUE: usize = 200;

fn norm(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Guard one model-mapped field against the PDF's own fields and text.
fn guard_field(
    raw: &Value,
    acro: &[forms::acro::AcroField],
    text_norm: &str,
) -> Result<Value, String> {
    let s = |k: &str| raw[k].as_str().unwrap_or("").trim().to_string();
    let name = s("name");
    let Some(field) = acro.iter().find(|f| f.name == name) else {
        return Err(format!("{name:?} is not a field of this PDF"));
    };
    // The PDF computes or locks it: a fill would refuse the value, so
    // it is never asked for (re-review #6, also covers older proposals).
    if field.read_only {
        return Err(format!("{name}: read-only field"));
    }
    let (label, quote, help, ty) = (s("label"), s("label_quote"), s("help"), s("type"));
    if label.is_empty() || label.chars().count() > 80 {
        return Err(format!("{name}: label empty or too long"));
    }
    let q = norm(&quote);
    // A real printed label found in the text layer on word boundaries
    // (review #24). One-word labels (Name, Date, Signature) are the
    // common case on request forms (re-review #2), so only an empty,
    // 1-2 char or function-word quote is refused. The visitor label
    // stays a plain-language paraphrase shown next to this quote.
    // A quote made only of function words grounds nothing, whatever its
    // length ("of the", "if any"; re-review of P349 #8).
    if q.len() < 3
        || q.split_whitespace().all(|w| STOPWORDS.contains(&w))
        || !format!(" {text_norm} ").contains(&format!(" {q} "))
    {
        return Err(format!("{name}: label quote is not in the PDF's text"));
    }
    let lower_help = help.to_lowercase();
    if help.chars().count() > 200 || lower_help.contains("http") || lower_help.contains("www.") {
        return Err(format!("{name}: help too long or carries a URL"));
    }
    // Radio groups and multi-state boxes are a choice of export values:
    // they map to `select` with the PDF's own options (review #1).
    let multi = field.kind == "radio" || (field.kind == "checkbox" && field.options.len() > 1);
    let ty = if multi && matches!(ty.as_str(), "checkbox" | "select") {
        "select".to_string()
    } else {
        ty
    };
    let ty_ok = match field.kind.as_str() {
        "checkbox" | "radio" if multi => ty == "select",
        "checkbox" => ty == "checkbox",
        "choice" => ty == "select",
        "text" => matches!(ty.as_str(), "text" | "date" | "number"),
        _ => false,
    };
    if !ty_ok || !TYPES.contains(&ty.as_str()) {
        return Err(format!(
            "{name}: type {ty:?} does not fit a {} field",
            field.kind
        ));
    }
    let topic = s("prefill_topic");
    Ok(json!({
        "name": name, "label": label, "help": help, "type": ty,
        "required": raw["required"].as_bool().unwrap_or(false),
        "options": field.options, "prefill_topic": if topic.is_empty() { Value::Null } else { Value::String(topic) },
        "label_quote": quote, "max_len": field.max_len,
    }))
}

/// Guard a whole proposal: (kept fields, guard lines).
pub fn guard_map(
    raw: &Value,
    acro: &[forms::acro::AcroField],
    text: &str,
) -> (Vec<Value>, Vec<Check>) {
    let text_norm = norm(text);
    let mut kept = Vec::new();
    let mut dropped = Vec::new();
    for f in raw["fields"].as_array().into_iter().flatten() {
        match guard_field(f, acro, &text_norm) {
            Ok(v) if !kept.iter().any(|k: &Value| k["name"] == v["name"]) => kept.push(v),
            Ok(_) => dropped.push("duplicate field".to_string()),
            Err(e) => dropped.push(e),
        }
    }
    let fillable = acro
        .iter()
        .filter(|f| matches!(f.kind.as_str(), "text" | "checkbox" | "radio" | "choice"))
        .count();
    let checks = vec![
        Check {
            id: "names_from_acroform".into(),
            ok: dropped.iter().all(|d| !d.contains("is not a field")),
            detail: format!("{}/{} fields mapped", kept.len(), fillable),
        },
        Check {
            id: "label_in_span".into(),
            ok: dropped.iter().all(|d| !d.contains("quote")),
            detail: if dropped.is_empty() {
                format!("{} labels found in the text layer", kept.len())
            } else {
                dropped.join("; ").chars().take(300).collect()
            },
        },
    ];
    (kept, checks)
}

// ------------------------------------------------------------------ DB

/// Boot DDL on the metadata DB (`handbook.sqlite`): `CREATE TABLE IF
/// NOT EXISTS` only — no ALTER, DROP or RENAME (docs/ops/deploy.md § Rails).
pub fn migrate_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS form_map (
            id INTEGER PRIMARY KEY,
            authority_slug TEXT NOT NULL,
            capture_id INTEGER NOT NULL,
            status TEXT NOT NULL,
            title TEXT NOT NULL,
            fields_json TEXT NOT NULL,
            guards_json TEXT NOT NULL,
            acroform_fields INTEGER NOT NULL,
            run_id INTEGER,
            created_at INTEGER NOT NULL,
            decided_at INTEGER
        );
        CREATE INDEX IF NOT EXISTS form_map_slug ON form_map(authority_slug, status);",
    )
}

/// Boot DDL on the P2 DB: one row per customer fill (values are PII).
pub fn migrate_p2_tables(p2: &Connection) -> rusqlite::Result<()> {
    p2.execute_batch(
        "CREATE TABLE IF NOT EXISTS form_fill (
            id TEXT PRIMARY KEY,
            customer_ref TEXT NOT NULL,
            form_map_id INTEGER NOT NULL,
            values_json TEXT NOT NULL,
            pdf_path TEXT NOT NULL,
            sha256 TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS form_fill_customer ON form_fill(customer_ref, created_at);",
    )
}

fn source_url(conn: &Connection, capture_id: i64) -> Option<String> {
    conn.query_row(
        "SELECT s.uri FROM capture c JOIN source s ON s.id = c.source_id WHERE c.id = ?1",
        [capture_id],
        |r| r.get(0),
    )
    .optional()
    .ok()
    .flatten()
}

/// (id, title, fields_json, capture_id, decided_at) of one `form_map`.
type MapRow = (i64, String, String, i64, Option<i64>);

fn form_view(conn: &Connection, slug: &str, status: &str, row: &MapRow) -> Value {
    let (id, title, fields, capture_id, decided) = row;
    let (id, capture_id, decided) = (*id, *capture_id, *decided);
    json!({
        "id": id, "authority_slug": slug, "title": title, "status": if status == "accepted" { "active" } else { status },
        "official_url": source_url(conn, capture_id), "source_fact_id": Value::Null, "fillable": true,
        "fields": serde_json::from_str::<Value>(fields).unwrap_or(json!([])), "accepted_at": decided,
    })
}

/// Published forms for one authority, shaped as
/// `docs/contracts/form.json` (`plan.rs` reads this).
pub fn accepted_for_slug(conn: &Connection, slug: &str) -> rusqlite::Result<Vec<Value>> {
    let has: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='form_map'",
            [],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    if !has {
        return Ok(Vec::new());
    }
    let rows: Vec<MapRow> = conn
        .prepare("SELECT id, title, fields_json, capture_id, decided_at FROM form_map WHERE authority_slug = ?1 AND status = 'accepted' ORDER BY id")?
        .query_map([slug], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows
        .into_iter()
        .map(|row| form_view(conn, slug, "accepted", &row))
        .collect())
}

/// Official PDF bytes for a capture (`capture.path` is store-relative).
fn capture_pdf(
    store: &Store,
    conn: &Connection,
    capture_id: i64,
) -> Result<(String, Vec<u8>), String> {
    let (slug, path): (String, String) = conn
        .query_row(
            "SELECT a.slug, c.path FROM capture c JOIN authority a ON a.id = c.authority_id WHERE c.id = ?1",
            [capture_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| "unknown capture".to_string())?;
    let bytes =
        std::fs::read(store.root().join(&path)).map_err(|_| "capture bytes missing".to_string())?;
    if !bytes.starts_with(b"%PDF") {
        return Err("capture is not a PDF".to_string());
    }
    Ok((slug, bytes))
}

/// Map one captured official PDF: AcroForm list + text layer → Spark
/// `FormMapV1` → guards → a proposed `form_map` row.
pub fn compose_blocking(
    store_root: &std::path::Path,
    kek: Option<&config::Kek>,
    capture_id: i64,
    run_id: u64,
) -> Result<Value, String> {
    let store = Store::new(store_root);
    let conn = db::open(&store).map_err(|e| format!("store unavailable: {e}"))?;
    migrate_tables(&conn).map_err(|e| format!("store: {e}"))?;
    let (slug, bytes) = capture_pdf(&store, &conn, capture_id)?;
    let acro = forms::acro::fields(&bytes).map_err(|e| e.to_string())?;
    let fillable: Vec<&forms::acro::AcroField> = acro
        .iter()
        .filter(|f| {
            !f.read_only && matches!(f.kind.as_str(), "text" | "checkbox" | "radio" | "choice")
        })
        .collect();
    if fillable.is_empty() {
        return Err(
            "this PDF has no fillable fields (a flat form: print the official blank)".to_string(),
        );
    }
    let text = forms::acro::text_layer(&bytes).map_err(|e| e.to_string())?;
    let names: Vec<&str> = fillable.iter().map(|f| f.name.as_str()).collect();
    let topics: Vec<&str> = std::iter::once("")
        .chain(crate::plan::TOPICS.iter().copied())
        .collect();
    let schema = json!({"type": "object", "additionalProperties": false, "required": ["title", "fields"],
    "properties": {"title": {"type": "string"}, "fields": {"type": "array", "items": {
        "type": "object", "additionalProperties": false,
        "required": ["name", "label", "help", "type", "required", "prefill_topic", "label_quote"],
        "properties": {
            "name": {"type": "string", "enum": names},
            "label": {"type": "string"}, "help": {"type": "string"},
            "type": {"type": "string", "enum": TYPES}, "required": {"type": "boolean"},
            "prefill_topic": {"type": "string", "enum": topics},
            "label_quote": {"type": "string"},
        }}}}});
    let field_lines: Vec<String> = fillable
        .iter()
        .map(|f| {
            format!(
                "- {} · {}{} · page {}",
                f.name,
                f.kind,
                if f.options.is_empty() {
                    String::new()
                } else {
                    format!(" · options {:?}", f.options)
                },
                f.page.unwrap_or(0)
            )
        })
        .collect();
    let (developer, per_run) = PROMPT
        .split_once(PER_RUN_MARKER)
        .expect("forms prompt carries a PER-RUN part");
    let clipped: String = text.chars().take(24_000).collect();
    let user = per_run
        .replace("{{fields}}", &field_lines.join("\n"))
        .replace("{{text}}", &clipped)
        .trim()
        .to_string();
    let client = crate::draft::live_client(&conn, kek)?.with_observer(
        crate::telemetry::DbObserver::new(store_root.to_path_buf()),
        "forms",
        Some(run_id),
    );
    let resp = client
        .chat_json(&ChatJsonRequest {
            model: FORMS_MODEL,
            developer,
            user,
            schema_name: "FormMapV1",
            schema,
            reasoning_effort: "medium",
            prompt_cache_key: "official-form-map-v1".to_string(),
            max_completion_tokens: 12_288,
            pipeline: "forms",
            run_id: Some(run_id),
        })
        .map_err(|e| format!("form map failed: {e}"))?;
    let (kept, checks) = guard_map(&resp.value, &acro, &text);
    if kept.is_empty() {
        return Err("no field survived the label guard".to_string());
    }
    let title = resp.value["title"]
        .as_str()
        .unwrap_or("Official form")
        .chars()
        .take(120)
        .collect::<String>();
    conn.execute(
        "INSERT INTO form_map(authority_slug, capture_id, status, title, fields_json, guards_json, acroform_fields, run_id, created_at)
         VALUES (?1, ?2, 'proposed', ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![slug, capture_id, title, Value::Array(kept.clone()).to_string(), serde_json::to_string(&checks).unwrap_or_default(), fillable.len() as i64, run_id as i64, unix_now()],
    )
    .map_err(|e| format!("store: {e}"))?;
    Ok(
        json!({"form_map_id": conn.last_insert_rowid(), "mapped": kept.len(), "acroform_fields": fillable.len(),
              "model": FORMS_MODEL, "latency_ms": resp.latency_ms, "input_tokens": resp.usage.input_tokens,
              "output_tokens": resp.usage.output_tokens, "cached_tokens": resp.usage.cached_tokens,
              "reasoning_tokens": resp.usage.reasoning_tokens}),
    )
}

// ------------------------------------------------------------- staff

#[derive(Debug, Deserialize)]
pub struct ComposeBody {
    pub capture_id: i64,
}

async fn compose(
    _auth: AdminAuth,
    State(state): State<AppState>,
    Json(body): Json<ComposeBody>,
) -> Response {
    let Ok(conn) = db::open(&Store::new(&state.store_root)) else {
        return store_unavailable();
    };
    if crate::telemetry::over_daily_cap(&conn, &state, unix_now()) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "daily model-call cap reached (meta.daily_call_cap)"})),
        )
            .into_response();
    }
    drop(conn);
    let run_id = state.draft_runs.reserve_id();
    let (bus, root, kek): (_, _, Option<Arc<config::Kek>>) = (
        state.bus.clone(),
        state.store_root.clone(),
        state.kek.clone(),
    );
    let capture_id = body.capture_id;
    tokio::spawn(async move {
        let topic = format!("run:{run_id}");
        bus.publish(&topic, json!({"type": "forms.started", "topic": topic, "run_id": run_id, "capture_id": capture_id}).to_string());
        let out = tokio::task::spawn_blocking(move || {
            compose_blocking(&root, kek.as_deref(), capture_id, run_id)
        })
        .await;
        let (kind, body) = match out {
            Ok(Ok(summary)) => ("forms.done", summary),
            Ok(Err(e)) => ("forms.failed", json!({"error": e})),
            Err(e) => (
                "forms.failed",
                json!({"error": format!("forms task failed: {e}")}),
            ),
        };
        let mut event =
            json!({"type": kind, "topic": topic, "run_id": run_id, "capture_id": capture_id});
        if let (Some(m), Some(extra)) = (event.as_object_mut(), body.as_object()) {
            m.extend(extra.clone());
        }
        bus.publish(&topic, event.to_string());
    });
    (
        StatusCode::ACCEPTED,
        Json(json!({"run_id": run_id, "topic": format!("run:{run_id}")})),
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub status: Option<String>,
    pub slug: Option<String>,
}

async fn list(
    _auth: AdminAuth,
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> Response {
    let Ok(conn) = db::open(&Store::new(&state.store_root)) else {
        return store_unavailable();
    };
    let status = q.status.unwrap_or_else(|| "proposed".to_string());
    let slug = q.slug.unwrap_or_default();
    let rows: Vec<Value> = conn
        .prepare(
            "SELECT id, authority_slug, status, title, fields_json, guards_json, capture_id, acroform_fields, created_at
             FROM form_map WHERE (?1 = 'all' OR status = ?1) AND (?2 = '' OR authority_slug = ?2) ORDER BY id DESC LIMIT 50",
        )
        .and_then(|mut s| {
            s.query_map(rusqlite::params![status, slug], |r| {
                let fields: Value = serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or(json!([]));
                Ok(json!({
                    "id": r.get::<_, i64>(0)?, "authority_slug": r.get::<_, String>(1)?, "status": r.get::<_, String>(2)?,
                    "title": r.get::<_, String>(3)?, "mapped": fields.as_array().map_or(0, Vec::len), "fields": fields,
                    "guards": serde_json::from_str::<Value>(&r.get::<_, String>(5)?).unwrap_or(json!([])),
                    "capture_id": r.get::<_, i64>(6)?, "acroform_fields": r.get::<_, i64>(7)?, "created_at": r.get::<_, i64>(8)?,
                }))
            })?
            .collect()
        })
        .unwrap_or_default();
    let rows: Vec<Value> = rows
        .into_iter()
        .map(|mut v| {
            v["source_url"] = json!(v["capture_id"].as_i64().and_then(|c| source_url(&conn, c)));
            v
        })
        .collect();
    Json(json!({"form_maps": rows})).into_response()
}

async fn decide(auth: AdminAuth, state: AppState, id: i64, accept: bool) -> Response {
    let Ok(conn) = db::open(&Store::new(&state.store_root)) else {
        return store_unavailable();
    };
    let now = unix_now();
    let to = if accept { "accepted" } else { "rejected" };
    let slug: Option<String> = conn
        .query_row(
            "SELECT authority_slug FROM form_map WHERE id = ?1 AND status = 'proposed'",
            [id],
            |r| r.get(0),
        )
        .optional()
        .unwrap_or(None);
    let Some(slug) = slug else {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error": "form map is not proposed"})),
        )
            .into_response();
    };
    let ok = conn.execute("UPDATE form_map SET status = ?1, decided_at = ?2 WHERE id = ?3", rusqlite::params![to, now, id]).is_ok()
        && conn
            .execute(
                "INSERT INTO audit(at, actor, action, entity, entity_id, before_ref, after_ref, reason)
                 VALUES (?1, ?2, ?3, 'form_map', ?4, 'proposed', ?5, ?6)",
                rusqlite::params![now, auth.user.user_id, if accept { "form_map_accept" } else { "form_map_reject" }, id, to, slug],
            )
            .is_ok();
    if !ok {
        return store_unavailable();
    }
    if accept {
        crate::ws_bus::publish_ops(
            &state.bus,
            "form_published",
            json!({"slug": slug, "form_id": id}),
            now,
        );
    }
    Json(json!({"id": id, "authority_slug": slug, "status": to})).into_response()
}

async fn accept(auth: AdminAuth, State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    decide(auth, state, id, true).await
}

async fn reject(auth: AdminAuth, State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    decide(auth, state, id, false).await
}

/// Staff routes merged onto the ADMIN listener only (the builder wraps
/// them in the admin guard stack; handlers take `AdminAuth`).
pub fn admin_router(state: AppState) -> Router {
    Router::<AppState>::new()
        .route("/apostille/api/v1/admin/form-maps", get(list))
        .route("/apostille/api/v1/admin/form-maps/compose", post(compose))
        .route(
            "/apostille/api/v1/admin/form-maps/{id}/accept",
            post(accept),
        )
        .route(
            "/apostille/api/v1/admin/form-maps/{id}/reject",
            post(reject),
        )
        .with_state(state)
}

// ---------------------------------------------------------- customer

static FILL_LIMITS: LazyLock<crate::passkey::RateBuckets> =
    LazyLock::new(crate::passkey::RateBuckets::default);
/// Network-keyed fill limits, in their own map (re-review #9).
static FILL_NET_LIMITS: LazyLock<crate::passkey::RateBuckets> =
    LazyLock::new(crate::passkey::RateBuckets::default);

#[derive(Debug, Deserialize)]
pub struct FillBody {
    /// Strings, numbers, or booleans for tick boxes (true → `yes`,
    /// false/null → left blank), as the order page sends them.
    pub values: BTreeMap<String, Value>,
}

/// Normalise posted values to the strings `fill_flatten` takes.
pub fn fill_values(raw: &BTreeMap<String, Value>) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    for (k, v) in raw {
        match v {
            Value::Bool(true) => {
                out.insert(k.clone(), "yes".to_string());
            }
            Value::Bool(false) | Value::Null => {}
            Value::String(s) if !s.trim().is_empty() => {
                out.insert(k.clone(), s.clone());
            }
            Value::String(_) => {}
            Value::Number(n) => {
                out.insert(k.clone(), n.to_string());
            }
            _ => return Err(format!("field {k:?} needs text, a number or true/false")),
        }
    }
    Ok(out)
}

/// `YYYY-MM-DD` as `(year, month, day)` string slices.
fn iso_date(v: &str) -> Option<(&str, &str, &str)> {
    let b = v.as_bytes();
    let shape = b.len() == 10
        && b.iter().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                *c == b'-'
            } else {
                c.is_ascii_digit()
            }
        });
    shape.then(|| (&v[..4], &v[5..7], &v[8..10]))
}

fn fill_id() -> String {
    use rand::Rng as _;
    const ALPHABET: &[u8] = b"abcdefghijkmnopqrstuvwxyz23456789";
    let mut rng = rand::rng();
    let tail: String = (0..10)
        .map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char)
        .collect();
    format!("fil_{tail}")
}

/// `POST /forms/{form_id}/fill` — signed-in customer, CSRF, 10/min:
/// the official PDF with the values drawn in and the form removed.
async fn fill(
    State(state): State<AppState>,
    headers: HeaderMap,
    crate::admin_ip::PeerIp(peer): crate::admin_ip::PeerIp,
    Path(form_id): Path<i64>,
    Json(body): Json<FillBody>,
) -> Response {
    if !crate::passkey::origin_ok(&headers, state.admin_mtls_off)
        || !crate::passkey::csrf_echo_ok(&headers)
    {
        return crate::passkey::csrf_fail();
    }
    let Some(customer_ref) = crate::p2::caller_customer_ref(&state, &headers) else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized"})),
        )
            .into_response();
    };
    let now = unix_now();
    // Network first, in its own map: a flood of fresh customer refs
    // from one network is refused before it can fill the per-customer
    // map (re-review #9).
    if !FILL_NET_LIMITS.check_write(
        &format!("fn:{}", crate::admin_ip::rate_key(peer)),
        now,
        20,
        60,
    ) || !FILL_LIMITS.check_write(&format!("ff:{customer_ref}"), now, 10, 60)
    {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "rate limited"})),
        )
            .into_response();
    }
    let store = Store::new(&state.store_root);
    let (Ok(conn), Ok(p2)) = (db::open(&store), db::open_p2(&store)) else {
        return store_unavailable();
    };
    let _ = migrate_p2_tables(&p2);
    // Stored fills are files: cap them per customer and day (#15).
    let recent: i64 = p2
        .query_row(
            "SELECT COUNT(*) FROM form_fill WHERE customer_ref = ?1 AND created_at >= ?2",
            rusqlite::params![customer_ref, now - 86_400],
            |r| r.get(0),
        )
        .unwrap_or(i64::MAX);
    if recent >= 30 {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "too many filled forms today"})),
        )
            .into_response();
    }
    let row: Option<(i64, String, String)> = conn
        .query_row(
            "SELECT capture_id, fields_json, authority_slug FROM form_map WHERE id = ?1 AND status = 'accepted'",
            [form_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .unwrap_or(None);
    let Some((capture_id, fields, slug)) = row else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown form"})),
        )
            .into_response();
    };
    let mapped: Vec<Value> = serde_json::from_str(&fields).unwrap_or_default();
    let allowed: Vec<&str> = mapped.iter().filter_map(|f| f["name"].as_str()).collect();
    // Every posted key and string is checked before normalising, which
    // drops false/null/blank entries (re-review #10).
    if let Some((bad, _)) = body.values.iter().find(|(k, v)| {
        !allowed.contains(&k.as_str())
            || v.as_str()
                .is_some_and(|s| s.chars().count() > MAX_FILL_VALUE)
    }) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": format!("field {bad:?} is not on this form or too long")})),
        )
            .into_response();
    }
    let mut values = match fill_values(&body.values) {
        Ok(v) => v,
        Err(e) => {
            return (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({"error": e}))).into_response();
        }
    };
    if let Some((bad, _)) = values
        .iter()
        .find(|(_, v)| v.chars().count() > MAX_FILL_VALUE)
    {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": format!("field {bad:?} is not on this form or too long")})),
        )
            .into_response();
    }
    // Date boxes printed shorter than an ISO date take the form's compact
    // US order on US forms: MaxLen 8 → MMDDYYYY, 6 → MMDDYY (the date
    // picker always posts YYYY-MM-DD; re-review of P349 #2). Elsewhere
    // the order is unknown, so the MaxLen refusal stands rather than
    // printing a wrong date (re-review of P351 #2).
    let us_form = slug.starts_with("usa-") || slug.starts_with("usdos");
    for f in mapped.iter().filter(|_| us_form) {
        let (Some(name), Some(max)) = (f["name"].as_str(), f["max_len"].as_u64()) else {
            continue;
        };
        if f["type"] != "date" || max >= 10 {
            continue;
        }
        if let Some(v) = values.get_mut(name)
            && let Some((y, m, d)) = iso_date(v)
        {
            let compact = match max {
                8..=9 => Some(format!("{m}{d}{y}")),
                6..=7 => Some(format!("{m}{d}{}", &y[2..])),
                _ => None,
            };
            if let Some(c) = compact {
                *v = c;
            }
        }
    }

    let Ok((_, bytes)) = capture_pdf(&store, &conn, capture_id) else {
        return store_unavailable();
    };
    let pdf = match tokio::task::spawn_blocking(move || {
        // The form computes or locks read-only fields: a map accepted
        // before the compose-time drop may still list one, so its value
        // is left out rather than failing every fill (re-review of P349 #9).
        let mut values = values;
        if let Ok(acro) = forms::acro::fields(&bytes) {
            values.retain(|k, _| !acro.iter().any(|f| f.read_only && &f.name == k));
        }
        // Only the validated, normalised values that were printed are
        // stored (re-review of P351 #3).
        let stored = serde_json::to_string(&values).unwrap_or_default();
        forms::acro::fill_flatten(&bytes, &values).map(|pdf| (pdf, stored))
    })
    .await
    {
        Ok(r) => r,
        Err(_) => return store_unavailable(),
    };
    let (pdf, stored) = match pdf {
        Ok(done) => done,
        Err(e) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"error": e.to_string()})),
            )
                .into_response();
        }
    };
    let id = fill_id();
    let dir = store.root().join("fills");
    let rel = format!("fills/{id}.pdf");
    if std::fs::create_dir_all(&dir).is_err()
        || std::fs::write(store.root().join(&rel), &pdf).is_err()
    {
        return store_unavailable();
    }
    use sha2::Digest as _;
    let sha = hex_of(&sha2::Sha256::digest(&pdf));
    if p2
        .execute(
            "INSERT INTO form_fill(id, customer_ref, form_map_id, values_json, pdf_path, sha256, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![id, customer_ref, form_id, stored, rel, sha, now],
        )
        .is_err()
    {
        return store_unavailable();
    }
    (
        StatusCode::CREATED,
        Json(json!({"fill_id": id, "form_id": form_id, "status": "ready", "pdf_url": format!("/apostille/api/v1/p2/forms/fills/{id}.pdf"), "flattened": true, "created_at": now})),
    )
        .into_response()
}

fn hex_of(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `GET /forms/fills/{fill}.pdf` — the filler only.
async fn get_fill(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(file): Path<String>,
) -> Response {
    let Some(customer_ref) = crate::p2::caller_customer_ref(&state, &headers) else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized"})),
        )
            .into_response();
    };
    let Some(id) = file
        .strip_suffix(".pdf")
        .filter(|id| id.starts_with("fil_") && id.len() == 14)
    else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown fill"})),
        )
            .into_response();
    };
    let store = Store::new(&state.store_root);
    let Ok(p2) = db::open_p2(&store) else {
        return store_unavailable();
    };
    let row: Option<(String, String)> = p2
        .query_row(
            "SELECT customer_ref, pdf_path FROM form_fill WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .unwrap_or(None);
    match row {
        Some((owner, path)) if owner == customer_ref => {
            match std::fs::read(store.root().join(path)) {
                Ok(pdf) => (
                    [
                        (header::CONTENT_TYPE, "application/pdf"),
                        (header::CACHE_CONTROL, "private, no-store"),
                    ],
                    pdf,
                )
                    .into_response(),
                Err(_) => store_unavailable(),
            }
        }
        _ => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown fill"})),
        )
            .into_response(),
    }
}

/// Customer routes, merged into the `/api/v1/p2` nest.
pub fn p2_routes() -> Router<AppState> {
    Router::new()
        .route("/forms/{form_id}/fill", post(fill))
        .route("/forms/fills/{file}", get(get_fill))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_dates_split_only_when_well_formed() {
        assert_eq!(iso_date("2026-11-04"), Some(("2026", "11", "04")));
        assert_eq!(iso_date("2026/11/04"), None);
        assert_eq!(iso_date("2026-11-4"), None);
        assert_eq!(iso_date("２０２６-11-04"), None);
    }

    #[test]
    fn one_word_labels_ground_and_read_only_fields_drop() {
        // Re-review #2/#7: Name, Date, Signature are the common labels.
        let text = norm("Name: ____ Date: ____ Signature: ____ Total amount enclosed: ____ the");
        let field = |name: &str, read_only: bool| forms::acro::AcroField {
            name: name.into(),
            kind: "text".into(),
            options: vec![],
            page: Some(1),
            max_len: None,
            read_only,
        };
        let acro = vec![
            field("Name", false),
            field("Date", false),
            field("Total", true),
        ];
        let raw = |name: &str, quote: &str| json!({"name": name, "label": "Your name", "help": "", "type": "text", "required": true, "prefill_topic": "", "label_quote": quote});
        assert!(guard_field(&raw("Name", "Name"), &acro, &text).is_ok());
        assert!(guard_field(&raw("Date", "Date"), &acro, &text).is_ok());
        assert!(
            guard_field(&raw("Name", "the"), &acro, &text).is_err(),
            "a function word grounds nothing"
        );
        assert!(guard_field(&raw("Name", "Na"), &acro, &text).is_err());
        // Re-review of P349 #8: filler phrases of any length ground nothing.
        let filler = norm("Name of the applicant, if any, please sign this form");
        assert!(guard_field(&raw("Name", "of the"), &acro, &filler).is_err());
        assert!(guard_field(&raw("Name", "if any"), &acro, &filler).is_err());
        assert!(guard_field(&raw("Name", "Name of the applicant"), &acro, &filler).is_ok());
        assert!(
            guard_field(&raw("Name", "Address"), &acro, &text).is_err(),
            "not in the text"
        );
        // Re-review #6: a computed field is never asked for.
        let total = guard_field(&raw("Total", "Total amount enclosed"), &acro, &text);
        assert!(total.unwrap_err().contains("read-only"));
    }

    fn acro() -> Vec<forms::acro::AcroField> {
        vec![
            forms::acro::AcroField {
                name: "RequesterName".into(),
                kind: "text".into(),
                options: vec![],
                page: Some(1),
                max_len: None,
                read_only: false,
            },
            forms::acro::AcroField {
                name: "ReturnMail".into(),
                kind: "checkbox".into(),
                options: vec!["Yes".into()],
                page: Some(1),
                max_len: None,
                read_only: false,
            },
        ]
    }

    #[test]
    fn labels_must_be_quoted_and_names_must_exist() {
        let text = "SYNTHETIC Apostille Request Form\nName of person requesting\nReturn by mail";
        let raw = json!({"fields": [
            {"name": "RequesterName", "label": "Your full name", "help": "", "type": "text", "required": true, "prefill_topic": "", "label_quote": "Name of person requesting"},
            {"name": "ReturnMail", "label": "Return by mail", "help": "", "type": "checkbox", "required": false, "prefill_topic": "mailing", "label_quote": "Return by mail"},
            {"name": "Invented", "label": "Passport number", "help": "", "type": "text", "required": true, "prefill_topic": "", "label_quote": "Passport number"},
            {"name": "RequesterName", "label": "Dup", "help": "", "type": "text", "required": true, "prefill_topic": "", "label_quote": "Name of person requesting"},
        ]});
        let (kept, checks) = guard_map(&raw, &acro(), text);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[1]["prefill_topic"], "mailing");
        assert!(!checks[0].ok, "the invented field is reported");
        let bad_quote = json!({"fields": [{"name": "RequesterName", "label": "Name", "help": "", "type": "text", "required": true, "prefill_topic": "", "label_quote": "Applicant passport"}]});
        assert!(guard_map(&bad_quote, &acro(), text).0.is_empty());
        let bad_type = json!({"fields": [{"name": "ReturnMail", "label": "Return", "help": "", "type": "text", "required": false, "prefill_topic": "", "label_quote": "Return by mail"}]});
        assert!(
            guard_map(&bad_type, &acro(), text).0.is_empty(),
            "a checkbox stays a checkbox"
        );
    }

    #[test]
    fn posted_values_normalise_booleans_and_refuse_objects() {
        let raw: BTreeMap<String, Value> = serde_json::from_value(json!({
            "RequesterName": "SYNTHETIC Customer", "ReturnMail": true, "Unticked": false,
            "Copies": 2, "Blank": "  ", "Gone": null
        }))
        .unwrap();
        let v = fill_values(&raw).unwrap();
        assert_eq!(v.get("ReturnMail").map(String::as_str), Some("yes"));
        assert_eq!(v.get("Copies").map(String::as_str), Some("2"));
        assert!(!v.contains_key("Unticked") && !v.contains_key("Blank") && !v.contains_key("Gone"));
        let bad: BTreeMap<String, Value> = serde_json::from_value(json!({"X": {"a": 1}})).unwrap();
        assert!(fill_values(&bad).is_err());
        let radio = vec![forms::acro::AcroField {
            name: "Delivery".into(),
            kind: "radio".into(),
            options: vec!["Mail".into(), "Pickup".into()],
            page: Some(1),
            max_len: None,
            read_only: false,
        }];
        let raw = json!({"fields": [{"name": "Delivery", "label": "How it comes back", "help": "", "type": "checkbox", "required": true, "prefill_topic": "", "label_quote": "Return by mail"}]});
        let (kept, _) = guard_map(&raw, &radio, "Return by mail or pickup");
        assert_eq!(
            kept[0]["type"], "select",
            "a radio group is a choice of its export values"
        );
        assert_eq!(kept[0]["options"], json!(["Mail", "Pickup"]));
    }

    #[test]
    fn contract_seed_shape_matches_the_served_form() {
        let seed: Value =
            serde_json::from_str(include_str!("../../../docs/contracts/plan.json")).unwrap();
        let conn = Connection::open_in_memory().unwrap();
        migrate_tables(&conn).unwrap();
        conn.execute_batch("CREATE TABLE capture(id INTEGER PRIMARY KEY, source_id INTEGER); CREATE TABLE source(id INTEGER PRIMARY KEY, uri TEXT);").unwrap();
        conn.execute(
            "INSERT INTO form_map(authority_slug, capture_id, status, title, fields_json, guards_json, acroform_fields, created_at, decided_at)
             VALUES ('usa-wa-sos', 1, 'accepted', 'SYNTHETIC form', '[]', '[]', 2, 1, 2)",
            [],
        )
        .unwrap();
        // As `/plan` serves it: `plan::read_body` adds `fill_supported`.
        let served = accepted_for_slug(&conn, "usa-wa-sos").unwrap();
        let served = crate::plan::read_body("usa-wa-sos", None, None, vec![], vec![], served);
        let served = &served["forms"];
        for key in seed["forms"][0].as_object().unwrap().keys() {
            assert!(served[0].get(key).is_some(), "form key {key}");
        }
        assert_eq!(served[0]["status"], "active");
    }
}
