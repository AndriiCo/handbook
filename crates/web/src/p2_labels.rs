//! P2 label purchase: Option A printable + Option B prepaid USPS.
//!
//! Mounted under the P2 router (`/apostille/api/v1/p2`). Every route
//! needs a DEMO session cookie and scopes shipments to the caller's
//! `customer_ref` (DEMO-grade auth, same as `/me`; the real login
//! stack replaces it per `docs/auth-playbook.md`).
//!
//! Hard rule: prepaid (Option B) buys run ONLY against the USPS test
//! environment or a loopback stub backend. [`labels::SandboxGuard`]
//! enforces it; a non-sandbox base URL fails closed with 403 plus an
//! audit row. There is no live-buy path in this module.
//!
//! Label bytes live in the store blob dir (content-addressed); the
//! `shipping_label` row carries the sha. Status changes publish
//! `label.*` WS events on `shipment:{id}`.

use axum::{
    Router,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Json},
    routing::{get, post},
};
use handbook_store::{
    db,
    store::{Store, sha256_hex},
};
use labels::{
    BuyRequest, Carrier, Direction, LabelClient, LabelStatus, ManualLabelTask, Parcel,
    PostalAddress, PurchaseOption, SandboxGuard, ServiceClass, ShipmentPayload, UreqTransport,
    UspsClient,
};
use serde::{Deserialize, Serialize};

/// Prepaid-buy configuration: USPS base URL + certification flag.
/// Built from DB-backed runtime config in `main` (`usps.base_url`,
/// `usps.certified` — goal-025 stream 4, zero-env); tests construct
/// it directly (no env races, no DB reads).
#[derive(Clone)]
pub struct LabelCfg {
    pub usps_base_url: String,
    pub usps_certified: bool,
}

impl LabelCfg {
    /// Build from loaded runtime values (no env, no DB reads).
    pub fn from_values(usps_base_url: &str, usps_certified: bool) -> Self {
        Self {
            usps_base_url: usps_base_url.to_string(),
            usps_certified,
        }
    }
}

impl Default for LabelCfg {
    fn default() -> Self {
        Self {
            usps_base_url: labels::USPS_BASE_TEST.to_string(),
            usps_certified: false,
        }
    }
}

/// Label-purchase sub-router, merged into the P2 router.
pub fn router() -> Router<crate::AppState> {
    Router::new()
        .route("/shipments", post(create_shipment))
        .route("/shipments/{id}", get(get_shipment))
        .route("/shipments/{id}/purchase", post(purchase))
        .route(
            "/shipments/{id}/labels/{label_id}/download",
            get(download_label),
        )
        .route(
            "/shipments/{id}/labels/{label_id}/paste-back",
            post(paste_back),
        )
        .route("/shipments/{id}/labels/{label_id}/void", post(void_label))
        .route(
            "/profile/label-defaults",
            get(get_defaults).put(put_defaults),
        )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateShipmentBody {
    authority_id: i64,
    from: PostalAddress,
    to: PostalAddress,
    parcel: Parcel,
    service: ServiceClass,
    #[serde(default)]
    submission_id: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PurchaseBody {
    direction: Direction,
    carrier: Carrier,
    option: PurchaseOption,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PasteBackBody {
    tracking_no: String,
    #[serde(default)]
    label_pdf_base64: Option<String>,
    #[serde(default)]
    label_sha256: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DefaultsBody {
    #[serde(default)]
    option_a_format: Option<String>,
    #[serde(default)]
    option_b_paper: Option<String>,
}

#[derive(Serialize)]
struct LabelView {
    id: i64,
    purchase_option: String,
    label_format: String,
    content_type: String,
    summary: labels::LabelSummary,
}

/// Caller identity from the DEMO session cookie (same rules as `me`).
/// The `Err` response is boxed: a bare `Response` trips
/// `clippy::result_large_err`.
fn caller(
    state: &crate::AppState,
    headers: &HeaderMap,
) -> Result<String, Box<axum::response::Response>> {
    let unauthorized = || {
        Box::new(
            (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "unauthorized"})),
            )
                .into_response(),
        )
    };
    let token = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| {
            cookies
                .split(';')
                .map(str::trim)
                .find_map(|pair| pair.strip_prefix("demo_session="))
        });
    let Some(token) = token.filter(|t| !t.is_empty()) else {
        return Err(unauthorized());
    };
    let conn = match db::open(&Store::new(&state.store_root)) {
        Ok(conn) => conn,
        Err(_) => return Err(Box::new(crate::store_unavailable())),
    };
    let Ok(Some((customer_ref, expires_at))) = db::get_demo_session(&conn, token) else {
        return Err(unauthorized());
    };
    if crate::unix_now() >= expires_at {
        return Err(unauthorized());
    }
    Ok(customer_ref)
}

fn audit(
    conn: &rusqlite::Connection,
    now: i64,
    actor: &str,
    action: &str,
    entity: &str,
    entity_id: i64,
    reason: &str,
) {
    let _ = conn.execute(
        "INSERT INTO audit(at, actor, action, entity, entity_id, reason)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![now, actor, action, entity, entity_id, reason],
    );
}

fn slug(service: ServiceClass) -> &'static str {
    match service {
        ServiceClass::Overnight => "overnight",
        ServiceClass::TwoDay => "two_day",
        ServiceClass::Ground => "ground",
        _ => "ground",
    }
}

fn unslug(service: &str) -> Option<ServiceClass> {
    match service {
        "overnight" => Some(ServiceClass::Overnight),
        "two_day" => Some(ServiceClass::TwoDay),
        "ground" => Some(ServiceClass::Ground),
        _ => None,
    }
}

fn carrier_slug(carrier: Carrier) -> &'static str {
    match carrier {
        Carrier::Usps => "usps",
        Carrier::FedEx => "fedex",
        _ => "usps",
    }
}

fn uncarrier(carrier: &str) -> Option<Carrier> {
    match carrier {
        "usps" => Some(Carrier::Usps),
        "fedex" => Some(Carrier::FedEx),
        _ => None,
    }
}

fn direction_slug(direction: Direction) -> &'static str {
    match direction {
        Direction::Outbound => "outbound",
        Direction::Return => "return",
        _ => "outbound",
    }
}

fn undirection(direction: &str) -> Option<Direction> {
    match direction {
        "outbound" => Some(Direction::Outbound),
        "return" => Some(Direction::Return),
        _ => None,
    }
}

fn status_slug(status: LabelStatus) -> &'static str {
    match status {
        LabelStatus::Pending => "pending",
        LabelStatus::Issued => "issued",
        LabelStatus::Voided => "voided",
        LabelStatus::Reissued => "reissued",
        _ => "pending",
    }
}

fn unstatus(status: &str) -> Option<LabelStatus> {
    match status {
        "pending" => Some(LabelStatus::Pending),
        "issued" => Some(LabelStatus::Issued),
        "voided" => Some(LabelStatus::Voided),
        "reissued" => Some(LabelStatus::Reissued),
        _ => None,
    }
}

/// `config::Kek` must stay `Send + Sync`: it rides inside `AppState`
/// across every `.await` in this module. This fails the build, not a
/// request, if that ever changes.
const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<config::Kek>();
};

/// Shipment row plus its owning customer.
#[derive(Clone)]
struct Shipment {
    id: i64,
    submission_id: i64,
    status: String,
    payload: ShipmentPayload,
    customer_ref: String,
}

fn load_shipment(
    p2: &rusqlite::Connection,
    shipment_id: i64,
) -> Result<Option<Shipment>, Box<axum::response::Response>> {
    let row: Option<(i64, String, String, String, String, String)> = p2
        .query_row(
            "SELECT s.submission_id, s.status, s.from_json, s.to_json, s.parcel_json,
                    f.customer_ref
             FROM shipment s JOIN form_submission f ON f.id = s.submission_id
             WHERE s.id = ?1",
            [shipment_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            _ => Err(Box::new(crate::store_unavailable())),
        })?;
    let Some((submission_id, status, from_json, to_json, parcel_json, customer_ref)) = row else {
        return Ok(None);
    };
    let from: PostalAddress = serde_json::from_str(&from_json).unwrap_or_else(|_| PostalAddress {
        name: String::new(),
        street1: String::new(),
        street2: None,
        city: String::new(),
        state_province: String::new(),
        postal: String::new(),
        country: String::new(),
        phone: None,
    });
    let to: PostalAddress = serde_json::from_str(&to_json).unwrap_or_else(|_| PostalAddress {
        name: String::new(),
        street1: String::new(),
        street2: None,
        city: String::new(),
        state_province: String::new(),
        postal: String::new(),
        country: String::new(),
        phone: None,
    });
    // Service tier travels inside parcel_json as {"parcel":..,"service":..}.
    let (parcel, service): (Parcel, ServiceClass) =
        serde_json::from_str::<serde_json::Value>(&parcel_json)
            .ok()
            .and_then(|v| {
                Some((
                    serde_json::from_value(v.get("parcel")?.clone()).ok()?,
                    serde_json::from_value(v.get("service")?.clone()).ok()?,
                ))
            })
            .unwrap_or((Parcel::LetterEnvelope, ServiceClass::Ground));
    Ok(Some(Shipment {
        id: shipment_id,
        submission_id,
        status,
        payload: ShipmentPayload {
            from,
            to,
            parcel,
            service,
            pairing_id: format!("ship-{shipment_id}"),
        },
        customer_ref,
    }))
}

/// Label row as stored.
#[derive(Clone)]
struct LabelRow {
    id: i64,
    direction: Direction,
    carrier: Carrier,
    service: ServiceClass,
    service_display: String,
    tracking_no: String,
    label_sha256: String,
    purchase_option: String,
    label_format: String,
    content_type: String,
    status: LabelStatus,
}

/// Raw `shipping_label` columns in `SELECT` order (factored out for
/// `clippy::type_complexity`).
type LabelColumns = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
);

fn load_label(
    p2: &rusqlite::Connection,
    shipment_id: i64,
    label_id: i64,
) -> Result<Option<LabelRow>, Box<axum::response::Response>> {
    let row: Option<LabelColumns> = p2
        .query_row(
            "SELECT direction, carrier, service, tracking_no, label_sha256,
                    purchase_option, label_format, content_type, status
             FROM shipping_label WHERE id = ?1 AND shipment_id = ?2",
            rusqlite::params![label_id, shipment_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                ))
            },
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            _ => Err(Box::new(crate::store_unavailable())),
        })?;
    Ok(row.and_then(
        |(
            direction,
            carrier,
            service,
            tracking_no,
            label_sha256,
            purchase_option,
            label_format,
            content_type,
            status,
        )| {
            Some(LabelRow {
                id: label_id,
                direction: undirection(&direction)?,
                carrier: uncarrier(&carrier)?,
                service_display: service.clone(),
                service: unslug(&service).unwrap_or(ServiceClass::Ground),
                tracking_no,
                label_sha256,
                purchase_option,
                label_format,
                content_type,
                status: unstatus(&status)?,
            })
        },
    ))
}

fn label_view(shipment_id: i64, row: &LabelRow) -> LabelView {
    let tracking_no = (!row.tracking_no.trim().is_empty()).then(|| row.tracking_no.clone());
    let service_name = match unslug(&row.service_display) {
        Some(service) => labels::service_name(row.carrier, service),
        None => row.service_display.clone(),
    };
    LabelView {
        id: row.id,
        purchase_option: row.purchase_option.clone(),
        label_format: row.label_format.clone(),
        content_type: row.content_type.clone(),
        summary: labels::LabelSummary {
            direction: row.direction,
            carrier: row.carrier,
            service_name,
            status: row.status,
            tracking_no: tracking_no.clone(),
            tracking_url: tracking_no
                .as_deref()
                .and_then(|no| labels::tracking_url(row.carrier, no)),
            label_url: (!row.label_sha256.is_empty()).then(|| {
                format!(
                    "/apostille/api/v1/p2/shipments/{shipment_id}/labels/{}/download",
                    row.id
                )
            }),
        },
    }
}

/// Recompute the shipment status from its labels: any pending →
/// `labels_pending`; else any issued/reissued → `ready`; else all
/// voided → `voided`; else `draft`.
fn refresh_shipment_status(p2: &rusqlite::Connection, shipment_id: i64, now: i64) -> String {
    let statuses: Vec<String> = p2
        .prepare("SELECT status FROM shipping_label WHERE shipment_id = ?1")
        .and_then(|mut stmt| {
            stmt.query_map([shipment_id], |row| row.get(0))?
                .collect::<Result<Vec<_>, _>>()
        })
        .unwrap_or_default();
    let next = if statuses.iter().any(|s| s == "pending") {
        "labels_pending"
    } else if statuses.iter().any(|s| s == "issued" || s == "reissued") {
        "ready"
    } else if !statuses.is_empty() && statuses.iter().all(|s| s == "voided") {
        "voided"
    } else {
        "draft"
    };
    let _ = p2.execute(
        "UPDATE shipment SET status = ?1, updated_at = ?2 WHERE id = ?3",
        rusqlite::params![next, now, shipment_id],
    );
    next.to_string()
}

/// Shipment events onto `shipment:{id}`: every event carries its
/// `topic` field, like the authority/run/admin publishers.
fn publish(state: &crate::AppState, shipment_id: i64, mut event: serde_json::Value) {
    if let Some(map) = event.as_object_mut() {
        map.insert(
            "topic".to_string(),
            serde_json::Value::String(format!("shipment:{shipment_id}")),
        );
    }
    state
        .bus
        .publish(&format!("shipment:{shipment_id}"), event.to_string());
}

/// Create a draft shipment. Bootstraps a minimal draft `form_submission`
/// shell (the forms stream owns real submissions; the shipment FK needs
/// a parent row).
async fn create_shipment(
    State(state): State<crate::AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateShipmentBody>,
) -> impl IntoResponse {
    let customer_ref = match caller(&state, &headers) {
        Ok(c) => c,
        Err(res) => return *res,
    };
    let provisional = ShipmentPayload {
        from: body.from.clone(),
        to: body.to.clone(),
        parcel: body.parcel,
        service: body.service,
        pairing_id: "pending".to_string(),
    };
    let defects = labels::validate_shipment(&provisional);
    if !defects.is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"errors": defects})),
        )
            .into_response();
    }
    let store = Store::new(&state.store_root);
    let (p2, main) = match (db::open_p2(&store), db::open(&store)) {
        (Ok(p2), Ok(main)) => (p2, main),
        _ => return crate::store_unavailable(),
    };
    // Optional order join: link a real caller-owned 'submitted'
    // submission; absent → legacy shell path below (unchanged).
    let linked_submission: Option<i64> = match body.submission_id {
        None => None,
        Some(wanted) => {
            let found: rusqlite::Result<Option<(String, String)>> = p2
                .query_row(
                    "SELECT customer_ref, status FROM form_submission WHERE id = ?1",
                    [wanted],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                });
            let Ok(found) = found else {
                return crate::store_unavailable();
            };
            let Some((owner, status)) = found else {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": "no such submission"})),
                )
                    .into_response();
            };
            if owner != customer_ref {
                // File convention (get/purchase/paste-back/void): owner
                // mismatch is 404, never 403 — no existence leak.
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": "no such submission"})),
                )
                    .into_response();
            }
            if status != "submitted" {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(serde_json::json!({"error": "submission is not submitted"})),
                )
                    .into_response();
            }
            Some(wanted)
        }
    };
    let now = crate::unix_now();
    let from_json = serde_json::to_string(&body.from).unwrap_or_default();
    let to_json = serde_json::to_string(&body.to).unwrap_or_default();
    let parcel_json =
        serde_json::json!({"parcel": body.parcel, "service": body.service}).to_string();
    let id = db::in_transaction(&p2, || {
        let submission_id = match linked_submission {
            Some(wanted) => wanted,
            None => {
                p2.execute(
                    "INSERT INTO form_submission(form_template_id, authority_id, customer_ref,
                                                values_json, status, created_at)
                     VALUES (0, ?1, ?2, '{}', 'draft', ?3)",
                    rusqlite::params![body.authority_id, customer_ref, now],
                )?;
                p2.last_insert_rowid()
            }
        };
        p2.execute(
            "INSERT INTO shipment(submission_id, authority_id, status, from_json, to_json,
                                 parcel_json, created_at, updated_at)
             VALUES (?1, ?2, 'draft', ?3, ?4, ?5, ?6, ?6)",
            rusqlite::params![
                submission_id,
                body.authority_id,
                from_json,
                to_json,
                parcel_json,
                now
            ],
        )?;
        Ok(p2.last_insert_rowid())
    });
    let Ok(id) = id else {
        return crate::store_unavailable();
    };
    audit(
        &main,
        now,
        &customer_ref,
        "shipment_create",
        "shipment",
        id,
        "draft",
    );
    (
        StatusCode::CREATED,
        Json(serde_json::json!({"id": id, "status": "draft"})),
    )
        .into_response()
}

async fn get_shipment(
    State(state): State<crate::AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let customer_ref = match caller(&state, &headers) {
        Ok(c) => c,
        Err(res) => return *res,
    };
    let store = Store::new(&state.store_root);
    let Ok(p2) = db::open_p2(&store) else {
        return crate::store_unavailable();
    };
    let shipment = match load_shipment(&p2, id) {
        Ok(Some(s)) => s,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "no such shipment"})),
            )
                .into_response();
        }
        Err(res) => return *res,
    };
    if shipment.customer_ref != customer_ref {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "no such shipment"})),
        )
            .into_response();
    }
    let label_ids: Vec<i64> = p2
        .prepare("SELECT id FROM shipping_label WHERE shipment_id = ?1 ORDER BY id")
        .and_then(|mut stmt| {
            stmt.query_map([id], |row| row.get(0))?
                .collect::<Result<Vec<_>, _>>()
        })
        .unwrap_or_default();
    let mut labels = Vec::new();
    for label_id in label_ids {
        match load_label(&p2, id, label_id) {
            Ok(Some(row)) => labels.push(label_view(id, &row)),
            Ok(None) => {}
            Err(res) => return *res,
        }
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "id": shipment.id,
            "submission_id": shipment.submission_id,
            "status": shipment.status,
            "labels": labels,
        })),
    )
        .into_response()
}

/// Purchase one label for a direction. Option A renders immediately;
/// Option B buys prepaid USPS postage in the sandbox (or queues the
/// staff-manual fallback when uncertified or when the buy fails).
async fn purchase(
    State(state): State<crate::AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(body): Json<PurchaseBody>,
) -> impl IntoResponse {
    let customer_ref = match caller(&state, &headers) {
        Ok(c) => c,
        Err(res) => return *res,
    };
    let store = Store::new(&state.store_root);
    // Load + validate synchronously. The connection closes at the end
    // of this block: `rusqlite::Connection` is `!Sync`, so no handle
    // may be alive across the prepaid `.await` below (each phase opens
    // its own connection instead).
    let shipment = {
        let p2 = match db::open_p2(&store) {
            Ok(p2) => p2,
            Err(_) => return crate::store_unavailable(),
        };
        match load_shipment(&p2, id) {
            Ok(Some(s)) => s,
            Ok(None) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": "no such shipment"})),
                )
                    .into_response();
            }
            Err(res) => return *res,
        }
    };
    if shipment.customer_ref != customer_ref {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "no such shipment"})),
        )
            .into_response();
    }
    let defects = labels::validate_shipment(&shipment.payload);
    if !defects.is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"errors": defects})),
        )
            .into_response();
    }
    match body.option {
        PurchaseOption::Printable { format } => purchase_printable(
            &state,
            &store,
            &shipment,
            body.carrier,
            body.direction,
            format,
        ),
        PurchaseOption::Prepaid { paper } => {
            purchase_prepaid(
                &state,
                &store,
                &shipment,
                body.carrier,
                body.direction,
                paper,
            )
            .await
        }
        _ => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"error": "unknown purchase option"})),
        )
            .into_response(),
    }
}

fn purchase_printable(
    state: &crate::AppState,
    store: &Store,
    shipment: &Shipment,
    carrier: Carrier,
    direction: Direction,
    format: labels::PrintFormat,
) -> axum::response::Response {
    let (p2, main) = match (db::open_p2(store), db::open(store)) {
        (Ok(p2), Ok(main)) => (p2, main),
        _ => return crate::store_unavailable(),
    };
    let now = crate::unix_now();
    let service_name = labels::service_name(carrier, shipment.payload.service);
    let html =
        labels::render_printable(&shipment.payload, direction, carrier, format, &service_name);
    let sha = match store.write_blob(html.as_bytes()) {
        Ok(sha) => sha,
        Err(_) => return crate::store_unavailable(),
    };
    let rowid = p2.execute(
        "INSERT INTO shipping_label(shipment_id, direction, carrier, service, tracking_no,
                                   payload_json, status, created_at, issued_at, purchase_option,
                                   label_format, label_sha256, content_type)
         VALUES (?1, ?2, ?3, ?4, '', '{}', 'issued', ?5, ?5, 'printable', ?6, ?7, 'text/html')",
        rusqlite::params![
            shipment.id,
            direction_slug(direction),
            carrier_slug(carrier),
            slug(shipment.payload.service),
            now,
            format.as_str(),
            sha,
        ],
    );
    if rowid.is_err() {
        return crate::store_unavailable();
    }
    let label_id = p2.last_insert_rowid();
    let status = refresh_shipment_status(&p2, shipment.id, now);
    audit(
        &main,
        now,
        &shipment.customer_ref,
        "label_printable_issued",
        "shipping_label",
        label_id,
        direction_slug(direction),
    );
    let Ok(Some(row)) = load_label(&p2, shipment.id, label_id) else {
        return crate::store_unavailable();
    };
    let view = label_view(shipment.id, &row);
    publish(
        state,
        shipment.id,
        serde_json::json!({
            "type": "label.ready",
            "direction": direction_slug(direction),
            "tracking_no": null,
            "url": view.summary.label_url,
        }),
    );
    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "queued": false,
            "shipment_status": status,
            "label": view,
        })),
    )
        .into_response()
}

async fn purchase_prepaid(
    state: &crate::AppState,
    store: &Store,
    shipment: &Shipment,
    carrier: Carrier,
    direction: Direction,
    paper: labels::PaperSize,
) -> axum::response::Response {
    // Prepaid is USPS-only (the fully-paid digital stamp).
    if carrier != Carrier::Usps {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"error": "prepaid labels are USPS-only"})),
        )
            .into_response();
    }
    // Hard rule: sandbox/TEM only, fail closed.
    if SandboxGuard::check_usps_base_url(&state.label_cfg.usps_base_url).is_err() {
        if let Ok(main) = db::open(store) {
            audit(
                &main,
                crate::unix_now(),
                &shipment.customer_ref,
                "label_purchase_blocked",
                "shipping_label",
                0,
                "non-sandbox usps base url",
            );
        }
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "prepaid postage is disabled outside the USPS test environment",
            })),
        )
            .into_response();
    }
    let now = crate::unix_now();
    if !state.label_cfg.usps_certified {
        let (p2, main) = match (db::open_p2(store), db::open(store)) {
            (Ok(p2), Ok(main)) => (p2, main),
            _ => return crate::store_unavailable(),
        };
        return queue_manual(
            state,
            &p2,
            &main,
            shipment,
            direction,
            paper,
            Some("carrier api not certified yet".to_string()),
            now,
        );
    }
    // Certified: buy over HTTP on a blocking thread (ureq is blocking).
    // With USPS credentials set, a real TEM bearer and payment token;
    // without them only a loopback stub answers (empty bearer). The
    // sandbox guard above keeps this path off production postage: live
    // USPS postage is bought only for paid orders (`customer_labels`).
    // No connection is alive across the await below (each phase
    // opens its own; `Connection` is `!Sync`).
    let payload = shipment.payload.clone();
    let base_url = state.label_cfg.usps_base_url.clone();
    let token_state = state.clone();
    let bought = tokio::task::spawn_blocking(move || {
        let (bearer, payment) = match crate::usps::test_tokens(&token_state) {
            Some(Ok(tokens)) => tokens,
            Some(Err(e)) => {
                return Err(labels::LabelError::Transport(labels::TransportError::new(
                    e,
                )));
            }
            None => (String::new(), "sandbox-unbilled".to_string()),
        };
        let client = UspsClient::new(&base_url, UreqTransport::new(), true);
        let request = BuyRequest {
            payload: &payload,
            direction,
            payment_ref: &payment,
        };
        client.buy(&bearer, &request)
    })
    .await;
    let bought = bought.unwrap_or_else(|e| {
        Err(labels::LabelError::Transport(labels::TransportError::new(
            e.to_string(),
        )))
    });
    // Write phase: fresh connections, opened after the await.
    let (p2, main) = match (db::open_p2(store), db::open(store)) {
        (Ok(p2), Ok(main)) => (p2, main),
        _ => return crate::store_unavailable(),
    };
    match bought {
        Ok(outcome) => {
            let sha = match store.write_blob(&outcome.label_pdf) {
                Ok(sha) => sha,
                Err(_) => return crate::store_unavailable(),
            };
            let inserted = p2.execute(
                "INSERT INTO shipping_label(shipment_id, direction, carrier, service, tracking_no,
                                           payload_json, status, created_at, issued_at, purchase_option,
                                           label_format, label_sha256, content_type)
                 VALUES (?1, ?2, 'usps', ?3, ?4, '{}', 'issued', ?5, ?5,
                         'prepaid', ?6, ?7, 'application/pdf')",
                rusqlite::params![
                    shipment.id,
                    direction_slug(direction),
                    slug(shipment.payload.service),
                    outcome.tracking_no,
                    now,
                    paper.as_str(),
                    sha,
                ],
            );
            if inserted.is_err() {
                return crate::store_unavailable();
            }
            let label_id = p2.last_insert_rowid();
            if let Some(cents) = outcome.postage_cost_cents {
                let _ = p2.execute(
                    "UPDATE shipment SET postage_cost_cents = COALESCE(postage_cost_cents, 0) + ?1
                     WHERE id = ?2",
                    rusqlite::params![cents, shipment.id],
                );
            }
            let status = refresh_shipment_status(&p2, shipment.id, now);
            audit(
                &main,
                now,
                &shipment.customer_ref,
                "label_prepaid_issued",
                "shipping_label",
                label_id,
                direction_slug(direction),
            );
            let Ok(Some(row)) = load_label(&p2, shipment.id, label_id) else {
                return crate::store_unavailable();
            };
            let view = label_view(shipment.id, &row);
            publish(
                state,
                shipment.id,
                serde_json::json!({
                    "type": "label.ready",
                    "direction": direction_slug(direction),
                    "tracking_no": view.summary.tracking_no,
                    "url": view.summary.label_url,
                }),
            );
            (
                StatusCode::CREATED,
                Json(serde_json::json!({
                    "queued": false,
                    "shipment_status": status,
                    "label": view,
                })),
            )
                .into_response()
        }
        Err(error) => queue_manual(
            state,
            &p2,
            &main,
            shipment,
            direction,
            paper,
            Some(error.to_string()),
            now,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn queue_manual(
    state: &crate::AppState,
    p2: &rusqlite::Connection,
    main: &rusqlite::Connection,
    shipment: &Shipment,
    direction: Direction,
    paper: labels::PaperSize,
    api_error: Option<String>,
    now: i64,
) -> axum::response::Response {
    let request = BuyRequest {
        payload: &shipment.payload,
        direction,
        payment_ref: "sandbox-unbilled",
    };
    let payload_json = labels::client::usps_label_body(&request).to_string();
    let inserted = p2.execute(
        "INSERT INTO shipping_label(shipment_id, direction, carrier, service, tracking_no,
                                   payload_json, status, created_at, purchase_option,
                                   label_format, label_sha256, content_type)
         VALUES (?1, ?2, 'usps', ?3, '', ?4, 'pending', ?5, 'prepaid', ?6, '', 'application/pdf')",
        rusqlite::params![
            shipment.id,
            direction_slug(direction),
            slug(shipment.payload.service),
            payload_json,
            now,
            paper.as_str(),
        ],
    );
    if inserted.is_err() {
        return crate::store_unavailable();
    }
    let label_id = p2.last_insert_rowid();
    let status = refresh_shipment_status(p2, shipment.id, now);
    let mut reason = format!("manual queue ({})", direction_slug(direction));
    if let Some(error) = &api_error {
        reason.push_str(": ");
        reason.push_str(&error.chars().take(200).collect::<String>());
    }
    audit(
        main,
        now,
        &shipment.customer_ref,
        "label_manual_queued",
        "shipping_label",
        label_id,
        &reason,
    );
    let Ok(Some(row)) = load_label(p2, shipment.id, label_id) else {
        return crate::store_unavailable();
    };
    let view = label_view(shipment.id, &row);
    publish(
        state,
        shipment.id,
        serde_json::json!({
            "type": "label.pending",
            "direction": direction_slug(direction),
        }),
    );
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "queued": true,
            "api_error": api_error,
            "shipment_status": status,
            "label": view,
        })),
    )
        .into_response()
}

/// Download a label file (printable HTML or carrier PDF). Pending
/// manual labels have no file yet (404) until staff paste back.
async fn download_label(
    State(state): State<crate::AppState>,
    headers: HeaderMap,
    Path((id, label_id)): Path<(i64, i64)>,
) -> impl IntoResponse {
    let customer_ref = match caller(&state, &headers) {
        Ok(c) => c,
        Err(res) => return *res,
    };
    let store = Store::new(&state.store_root);
    let Ok(p2) = db::open_p2(&store) else {
        return crate::store_unavailable();
    };
    let shipment = match load_shipment(&p2, id) {
        Ok(Some(s)) => s,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "no such shipment"})),
            )
                .into_response();
        }
        Err(res) => return *res,
    };
    if shipment.customer_ref != customer_ref {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "no such shipment"})),
        )
            .into_response();
    }
    let row = match load_label(&p2, id, label_id) {
        Ok(Some(row)) => row,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "no such label"})),
            )
                .into_response();
        }
        Err(res) => return *res,
    };
    if row.label_sha256.is_empty() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "label file not available yet"})),
        )
            .into_response();
    }
    let Ok(bytes) = store.read_blob(&row.label_sha256) else {
        return crate::store_unavailable();
    };
    let ext = if row.content_type == "text/html" {
        "html"
    } else {
        "pdf"
    };
    let Ok(content_type) = HeaderValue::from_str(&row.content_type) else {
        return crate::store_unavailable();
    };
    let Ok(disposition) =
        HeaderValue::from_str(&format!("attachment; filename=\"label-{label_id}.{ext}\""))
    else {
        return crate::store_unavailable();
    };
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type),
            (header::CONTENT_DISPOSITION, disposition),
        ],
        bytes,
    )
        .into_response()
}

/// Staff-manual paste-back: tracking number plus the bought label PDF
/// (base64 bytes to store, or the sha of an already-stored blob).
/// Transitions follow [`ManualLabelTask`]: pending/voided → issued/
/// reissued; anything else is a 409.
async fn paste_back(
    State(state): State<crate::AppState>,
    headers: HeaderMap,
    Path((id, label_id)): Path<(i64, i64)>,
    Json(body): Json<PasteBackBody>,
) -> impl IntoResponse {
    let customer_ref = match caller(&state, &headers) {
        Ok(c) => c,
        Err(res) => return *res,
    };
    let store = Store::new(&state.store_root);
    let (p2, main) = match (db::open_p2(&store), db::open(&store)) {
        (Ok(p2), Ok(main)) => (p2, main),
        _ => return crate::store_unavailable(),
    };
    let shipment = match load_shipment(&p2, id) {
        Ok(Some(s)) => s,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "no such shipment"})),
            )
                .into_response();
        }
        Err(res) => return *res,
    };
    if shipment.customer_ref != customer_ref {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "no such shipment"})),
        )
            .into_response();
    }
    let row = match load_label(&p2, id, label_id) {
        Ok(Some(row)) => row,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "no such label"})),
            )
                .into_response();
        }
        Err(res) => return *res,
    };
    // Resolve the label bytes before touching the state machine.
    let (sha, content_type) = match (&body.label_pdf_base64, &body.label_sha256) {
        (Some(encoded), None) => {
            use base64::Engine as _;
            let compact: String = encoded.chars().filter(|c| !c.is_whitespace()).collect();
            let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(compact.as_bytes())
            else {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(serde_json::json!({"error": "label_pdf_base64 does not decode"})),
                )
                    .into_response();
            };
            if !bytes.starts_with(b"%PDF-") {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(serde_json::json!({"error": "pasted label is not a pdf"})),
                )
                    .into_response();
            }
            match store.write_blob(&bytes) {
                Ok(sha) => (sha, "application/pdf".to_string()),
                Err(_) => return crate::store_unavailable(),
            }
        }
        (None, Some(sha)) => {
            if store.read_blob(sha).is_err() {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(serde_json::json!({"error": "unknown label_sha256"})),
                )
                    .into_response();
            }
            (sha.clone(), row.content_type.clone())
        }
        _ => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({
                    "error": "one of label_pdf_base64 or label_sha256 is required",
                })),
            )
                .into_response();
        }
    };
    // Reuse the labels-crate state machine for the transition.
    let now = crate::unix_now();
    let mut task = ManualLabelTask {
        carrier: row.carrier,
        service: row.service,
        direction: row.direction,
        pairing_id: shipment.payload.pairing_id.clone(),
        payload_json: serde_json::Value::Null,
        status: row.status,
        tracking_no: (!row.tracking_no.is_empty()).then(|| row.tracking_no.clone()),
        label_sha256: (!row.label_sha256.is_empty()).then(|| row.label_sha256.clone()),
        api_error: None,
        created_at: now,
        updated_at: now,
    };
    if task.paste_back(body.tracking_no.trim(), &sha, now).is_err() {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": "label is already issued"})),
        )
            .into_response();
    }
    let updated = p2.execute(
        "UPDATE shipping_label SET tracking_no = ?1, label_sha256 = ?2, content_type = ?3,
                                   status = ?4, issued_at = ?5 WHERE id = ?6 AND shipment_id = ?7",
        rusqlite::params![
            body.tracking_no.trim(),
            sha,
            content_type,
            status_slug(task.status),
            now,
            label_id,
            id,
        ],
    );
    if updated.is_err() {
        return crate::store_unavailable();
    }
    let status = refresh_shipment_status(&p2, id, now);
    audit(
        &main,
        now,
        &customer_ref,
        "label_paste_back",
        "shipping_label",
        label_id,
        body.tracking_no.trim(),
    );
    let Ok(Some(row)) = load_label(&p2, id, label_id) else {
        return crate::store_unavailable();
    };
    let view = label_view(id, &row);
    publish(
        &state,
        id,
        serde_json::json!({
            "type": "label.ready",
            "direction": direction_slug(row.direction),
            "tracking_no": view.summary.tracking_no,
            "url": view.summary.label_url,
        }),
    );
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "shipment_status": status,
            "label": view,
        })),
    )
        .into_response()
}

/// Void an issued label. Prepaid API labels cancel against the sandbox
/// carrier first (a failed cancel refuses the void with 502, so the
/// carrier-side label never silently stays live); manual and printable
/// labels void locally.
async fn void_label(
    State(state): State<crate::AppState>,
    headers: HeaderMap,
    Path((id, label_id)): Path<(i64, i64)>,
) -> impl IntoResponse {
    let customer_ref = match caller(&state, &headers) {
        Ok(c) => c,
        Err(res) => return *res,
    };
    let store = Store::new(&state.store_root);
    // Load phase: connections close before any `.await` below
    // (`Connection` is `!Sync`).
    let (owner, row) = {
        let p2 = match db::open_p2(&store) {
            Ok(p2) => p2,
            Err(_) => return crate::store_unavailable(),
        };
        let shipment = match load_shipment(&p2, id) {
            Ok(Some(s)) => s,
            Ok(None) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": "no such shipment"})),
                )
                    .into_response();
            }
            Err(res) => return *res,
        };
        let row = match load_label(&p2, id, label_id) {
            Ok(Some(row)) => row,
            Ok(None) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": "no such label"})),
                )
                    .into_response();
            }
            Err(res) => return *res,
        };
        (shipment.customer_ref, row)
    };
    if owner != customer_ref {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "no such shipment"})),
        )
            .into_response();
    }
    if row.status != LabelStatus::Issued {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": "only an issued label can be voided"})),
        )
            .into_response();
    }
    // API-bought prepaid labels cancel carrier-side first. Manual
    // paste-backs carry no API session, so they void locally: only a
    // label bought through this module's certified path (prepaid +
    // tracking + stored PDF bought from the carrier) cancels.
    let api_bought = row.purchase_option == "prepaid" && !row.tracking_no.trim().is_empty();
    if api_bought && state.label_cfg.usps_certified && row.carrier == Carrier::Usps {
        if SandboxGuard::check_usps_base_url(&state.label_cfg.usps_base_url).is_err() {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "error": "carrier void is disabled outside the USPS test environment",
                })),
            )
                .into_response();
        }
        let base_url = state.label_cfg.usps_base_url.clone();
        let tracking = row.tracking_no.clone();
        let token_state = state.clone();
        let cancelled = tokio::task::spawn_blocking(move || {
            let client = UspsClient::new(&base_url, UreqTransport::new(), true);
            match crate::usps::test_tokens(&token_state) {
                Some(Ok((bearer, payment))) => client.cancel_paid(&bearer, &payment, &tracking),
                Some(Err(e)) => Err(labels::LabelError::Transport(labels::TransportError::new(
                    e,
                ))),
                None => client.cancel("", &tracking),
            }
        })
        .await;
        let cancelled = cancelled.unwrap_or_else(|e| {
            Err(labels::LabelError::Transport(labels::TransportError::new(
                e.to_string(),
            )))
        });
        if let Err(error) = cancelled {
            if let Ok(main) = db::open(&store) {
                audit(
                    &main,
                    crate::unix_now(),
                    &customer_ref,
                    "label_void_refused",
                    "shipping_label",
                    label_id,
                    &error.to_string().chars().take(200).collect::<String>(),
                );
            }
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({
                    "error": format!("carrier refused the void: {error}"),
                })),
            )
                .into_response();
        }
    }
    // Write phase: fresh connections, opened after the await.
    let (p2, main) = match (db::open_p2(&store), db::open(&store)) {
        (Ok(p2), Ok(main)) => (p2, main),
        _ => return crate::store_unavailable(),
    };
    let now = crate::unix_now();
    let updated = p2.execute(
        "UPDATE shipping_label SET status = 'voided' WHERE id = ?1 AND shipment_id = ?2",
        rusqlite::params![label_id, id],
    );
    if updated.is_err() {
        return crate::store_unavailable();
    }
    let status = refresh_shipment_status(&p2, id, now);
    audit(
        &main,
        now,
        &customer_ref,
        "label_voided",
        "shipping_label",
        label_id,
        direction_slug(row.direction),
    );
    publish(
        &state,
        id,
        serde_json::json!({
            "type": "label.voided",
            "direction": direction_slug(row.direction),
        }),
    );
    // Reuse sha256_hex (blob addressing) so the void receipt cites the
    // exact file version that was voided.
    let receipt = if row.label_sha256.is_empty() {
        sha256_hex(format!("voided-label-{label_id}").as_bytes())
    } else {
        row.label_sha256.clone()
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "shipment_status": status,
            "void_receipt": receipt,
        })),
    )
        .into_response()
}

async fn get_defaults(
    State(state): State<crate::AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let customer_ref = match caller(&state, &headers) {
        Ok(c) => c,
        Err(res) => return *res,
    };
    let store = Store::new(&state.store_root);
    let Ok(p2) = db::open_p2(&store) else {
        return crate::store_unavailable();
    };
    let saved = db::get_label_defaults(&p2, &customer_ref).unwrap_or(None);
    let (option_a_format, option_b_paper) = match saved {
        Some(d) => (d.option_a_format, d.option_b_paper),
        None => ("four_by_six".to_string(), "four_by_six".to_string()),
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "option_a_format": option_a_format,
            "option_b_paper": option_b_paper,
        })),
    )
        .into_response()
}

async fn put_defaults(
    State(state): State<crate::AppState>,
    headers: HeaderMap,
    Json(body): Json<DefaultsBody>,
) -> impl IntoResponse {
    let customer_ref = match caller(&state, &headers) {
        Ok(c) => c,
        Err(res) => return *res,
    };
    if let Some(format) = &body.option_a_format
        && labels::PrintFormat::parse(format).is_none()
    {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"error": "unknown option_a_format"})),
        )
            .into_response();
    }
    if let Some(paper) = &body.option_b_paper
        && labels::PaperSize::parse(paper).is_none()
    {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"error": "unknown option_b_paper"})),
        )
            .into_response();
    }
    let store = Store::new(&state.store_root);
    let (p2, main) = match (db::open_p2(&store), db::open(&store)) {
        (Ok(p2), Ok(main)) => (p2, main),
        _ => return crate::store_unavailable(),
    };
    let now = crate::unix_now();
    let current = db::get_label_defaults(&p2, &customer_ref).unwrap_or(None);
    let option_a_format = body.option_a_format.unwrap_or_else(|| {
        current
            .as_ref()
            .map(|d| d.option_a_format.clone())
            .unwrap_or_else(|| "four_by_six".to_string())
    });
    let option_b_paper = body.option_b_paper.unwrap_or_else(|| {
        current
            .map(|d| d.option_b_paper)
            .unwrap_or_else(|| "four_by_six".to_string())
    });
    if db::upsert_label_defaults(&p2, &customer_ref, &option_a_format, &option_b_paper, now)
        .is_err()
    {
        return crate::store_unavailable();
    }
    audit(
        &main,
        now,
        &customer_ref,
        "label_defaults_saved",
        "profile_label_defaults",
        0,
        &format!("{option_a_format}/{option_b_paper}"),
    );
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "option_a_format": option_a_format,
            "option_b_paper": option_b_paper,
        })),
    )
        .into_response()
}

/// Boot DDL on the P2 DB for the customer label ledger
/// (`label_purchase`): `CREATE TABLE IF NOT EXISTS` only. B0 seam;
/// lane SA-25 fills it in.
pub fn migrate_p2_tables(_p2: &rusqlite::Connection) -> rusqlite::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[test]
    fn label_defaults_match_db_runtime_defaults() {
        // The config crate hardcodes the USPS test base as the
        // `usps.base_url` default; it must stay identical to the
        // labels crate constant (same value, two crates).
        assert_eq!(
            config::RuntimeConfig::default().usps_base_url,
            labels::USPS_BASE_TEST
        );
        assert_eq!(LabelCfg::default().usps_base_url, labels::USPS_BASE_TEST);
        assert!(!LabelCfg::default().usps_certified);
        let cfg = LabelCfg::from_values("https://stub.test", true);
        assert_eq!(cfg.usps_base_url, "https://stub.test");
        assert!(cfg.usps_certified);
    }

    struct Fixture {
        _tmp: tempfile::TempDir,
        state: crate::AppState,
    }

    fn fixture(label_cfg: LabelCfg) -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path());
        store.init().unwrap();
        let conn = db::open(&store).unwrap();
        db::migrate(&conn).unwrap();
        db::create_demo_session(&conn, "tok-1", "cust-1", 1000, 9_999_999_999).unwrap();
        db::create_demo_session(&conn, "tok-2", "cust-2", 1000, 9_999_999_999).unwrap();
        drop(conn);
        let p2 = db::open_p2(&store).unwrap();
        db::migrate_p2(&p2).unwrap();
        drop(p2);
        let state = crate::AppState {
            spa_index: std::path::PathBuf::from("test-spa/index.html"),
            static_dir: std::path::PathBuf::from("static"),
            store_root: tmp.path().to_path_buf(),
            bus: crate::ws_bus::Bus::new(),
            kek: None,
            admin_mtls_off: true,
            admin_allow_ips: config::loopback_allow_ips(),
            admin_spa_dir: tmp.path().join("admin-spa"),
            media_terms_read: false,
            label_cfg,
            voice_cfg: voice::VoiceConfig::default(), // ADMIN-VOICE
            voice_api_key: None,
            webauthn: std::sync::Arc::new(crate::passkey::test_webauthn()),
            auth_limits: std::sync::Arc::new(crate::passkey::RateBuckets::default()),
            demo_limits: std::sync::Arc::new(crate::passkey::RateBuckets::default()),
            draft_runs: std::sync::Arc::new(crate::draft::DraftRuns::default()),
            config_cache: std::sync::Arc::new(config::CachedReader::default()),
            google_cfg: None,
            apple_cfg: None,
            meta_cfg: None,
            mailer: None,
            magic_link_base: "https://example.test/apostille/login".to_string(),
            notify_poke: std::sync::Arc::new(tokio::sync::Notify::new()),
            notify_enabled: false,
            notify_daily_cap: 3,
            notify_postal: String::new(),
            notify_limits: std::sync::Arc::new(crate::passkey::RateBuckets::default()),
            notify_base: "https://example.test/apostille".to_string(),
            meta_delete_status_base: "https://example.test/auth/meta/delete-status".to_string(),
            fakes: config::Modes::default(),
            fedex_live: crate::fedex_live::FedexLiveState::default(),
        };
        Fixture { _tmp: tmp, state }
    }

    fn app(state: &crate::AppState) -> Router {
        crate::public_router(state.clone(), "static", "test-spa", "test-acme")
    }

    fn post(uri: &str, token: Option<&str>, json: serde_json::Value) -> Request<Body> {
        let mut builder = Request::builder()
            .uri(uri)
            .method("POST")
            .header("content-type", "application/json");
        if let Some(token) = token {
            builder = builder.header("cookie", format!("demo_session={token}"));
        }
        builder.body(Body::from(json.to_string())).unwrap()
    }

    fn get(uri: &str, token: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder().uri(uri);
        if let Some(token) = token {
            builder = builder.header("cookie", format!("demo_session={token}"));
        }
        builder.body(Body::empty()).unwrap()
    }

    fn put(uri: &str, token: &str, json: serde_json::Value) -> Request<Body> {
        Request::builder()
            .uri(uri)
            .method("PUT")
            .header("content-type", "application/json")
            .header("cookie", format!("demo_session={token}"))
            .body(Body::from(json.to_string()))
            .unwrap()
    }

    async fn body_json(res: axum::response::Response) -> serde_json::Value {
        let body = axum::body::to_bytes(res.into_body(), 1024 * 1024)
            .await
            .unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    fn shipment_body() -> serde_json::Value {
        serde_json::json!({
            "authority_id": 1,
            "from": {
                "name": "Jane Doe",
                "street1": "123 Main St",
                "city": "Seattle",
                "state_province": "WA",
                "postal": "98101",
                "country": "US",
                "phone": "+1 555 123 4567",
            },
            "to": {
                "name": "WA Secretary of State",
                "street1": "PO Box 40234",
                "city": "Olympia",
                "state_province": "WA",
                "postal": "98504",
                "country": "US",
            },
            "parcel": {"preset": "letter_envelope"},
            "service": "two_day",
        })
    }

    async fn create_shipment_id(fixture: &Fixture, token: &str) -> i64 {
        let res = app(&fixture.state)
            .oneshot(post(
                "/apostille/api/v1/p2/shipments",
                Some(token),
                shipment_body(),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        body_json(res).await["id"].as_i64().unwrap()
    }

    /// Stub carrier backend on an ephemeral port (port 0): routes by
    /// method (`POST` → buy answer, `DELETE` → cancel answer) and
    /// records raw requests. Handles `connections` requests, then exits.
    fn stub_backend(
        buy_answer: Vec<u8>,
        connections: usize,
    ) -> (std::net::SocketAddr, std::thread::JoinHandle<Vec<String>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for _ in 0..connections {
                let (mut stream, _) = listener.accept().unwrap();
                let request = read_http_request(&mut stream);
                let (status, answer) = if request.starts_with("DELETE") {
                    ("200 OK", b"{}".as_slice())
                } else {
                    ("200 OK", buy_answer.as_slice())
                };
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    answer.len()
                );
                use std::io::Write as _;
                stream.write_all(head.as_bytes()).unwrap();
                stream.write_all(answer).unwrap();
                stream.flush().unwrap();
                seen.push(request);
            }
            seen
        });
        (addr, handle)
    }

    fn read_http_request(stream: &mut std::net::TcpStream) -> String {
        use std::io::Read as _;
        let mut raw = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            let n = stream.read(&mut chunk).unwrap();
            raw.extend_from_slice(&chunk[..n]);
            if raw.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let head = String::from_utf8_lossy(&raw).to_string();
        let len: usize = head
            .lines()
            .find_map(|line| {
                line.split_once(':').and_then(|(k, v)| {
                    k.eq_ignore_ascii_case("content-length")
                        .then(|| v.trim().parse().ok())?
                })
            })
            .unwrap_or(0);
        let body_start = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        while raw.len() < body_start + len {
            let n = stream.read(&mut chunk).unwrap();
            raw.extend_from_slice(&chunk[..n]);
        }
        String::from_utf8_lossy(&raw).to_string()
    }

    fn usps_buy_answer() -> Vec<u8> {
        use base64::Engine as _;
        let pdf = b"%PDF-1.4 stub postage label";
        let encoded = base64::engine::general_purpose::STANDARD.encode(pdf);
        serde_json::to_vec(&serde_json::json!({
            "trackingNumber": "9405 5000 0000 0000 0000 00",
            "labelImage": encoded,
            "postageCents": 1250,
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn purchase_without_session_is_unauthorized() {
        let fixture = fixture(LabelCfg::default());
        let res = app(&fixture.state)
            .oneshot(post(
                "/apostille/api/v1/p2/shipments/1/purchase",
                None,
                serde_json::json!({
                    "direction": "outbound",
                    "carrier": "usps",
                    "option": {"kind": "printable", "format": "four_by_six"},
                }),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn create_rejects_bad_address() {
        let fixture = fixture(LabelCfg::default());
        let mut body = shipment_body();
        body["to"]["country"] = serde_json::json!("USA");
        let res = app(&fixture.state)
            .oneshot(post("/apostille/api/v1/p2/shipments", Some("tok-1"), body))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let value = body_json(res).await;
        assert_eq!(value["errors"][0]["code"], "country");
    }

    #[tokio::test]
    async fn printable_purchase_issues_and_downloads() {
        let fixture = fixture(LabelCfg::default());
        let id = create_shipment_id(&fixture, "tok-1").await;
        let mut rx = fixture.state.bus.subscribe(&format!("shipment:{id}"));
        let res = app(&fixture.state)
            .oneshot(post(
                &format!("/apostille/api/v1/p2/shipments/{id}/purchase"),
                Some("tok-1"),
                serde_json::json!({
                    "direction": "outbound",
                    "carrier": "usps",
                    "option": {"kind": "printable", "format": "window_envelope"},
                }),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        let value = body_json(res).await;
        assert_eq!(value["queued"], false);
        assert_eq!(value["shipment_status"], "ready");
        assert_eq!(value["label"]["summary"]["status"], "issued");
        assert_eq!(value["label"]["summary"]["service_name"], "USPS Priority");
        let label_id = value["label"]["id"].as_i64().unwrap();
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(event.contains("label.ready"), "{event}");
        let value: serde_json::Value = serde_json::from_str(&event).unwrap();
        assert_eq!(value["topic"], format!("shipment:{id}"), "{event}");

        let res = app(&fixture.state)
            .oneshot(get(
                &format!("/apostille/api/v1/p2/shipments/{id}/labels/{label_id}/download"),
                Some("tok-1"),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers().get("content-type").unwrap(), "text/html");
        let body = axum::body::to_bytes(res.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("NOT POSTAGE"), "{html}");
        assert!(html.contains("margin-left: 1.5in;"), "{html}");
    }

    #[tokio::test]
    async fn prepaid_uncertified_queues_then_paste_back_issues() {
        let fixture = fixture(LabelCfg::default());
        let id = create_shipment_id(&fixture, "tok-1").await;
        let res = app(&fixture.state)
            .oneshot(post(
                &format!("/apostille/api/v1/p2/shipments/{id}/purchase"),
                Some("tok-1"),
                serde_json::json!({
                    "direction": "return",
                    "carrier": "usps",
                    "option": {"kind": "prepaid", "paper": "letter"},
                }),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::ACCEPTED);
        let value = body_json(res).await;
        assert_eq!(value["queued"], true);
        assert_eq!(value["shipment_status"], "labels_pending");
        assert_eq!(value["label"]["summary"]["status"], "pending");
        let label_id = value["label"]["id"].as_i64().unwrap();

        // No file until staff paste back.
        let res = app(&fixture.state)
            .oneshot(get(
                &format!("/apostille/api/v1/p2/shipments/{id}/labels/{label_id}/download"),
                Some("tok-1"),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);

        use base64::Engine as _;
        let pdf = base64::engine::general_purpose::STANDARD.encode(b"%PDF-1.4 staff label");
        let res = app(&fixture.state)
            .oneshot(post(
                &format!("/apostille/api/v1/p2/shipments/{id}/labels/{label_id}/paste-back"),
                Some("tok-1"),
                serde_json::json!({
                    "tracking_no": "9405 5000 0000 0000 0000 00",
                    "label_pdf_base64": pdf,
                }),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let value = body_json(res).await;
        assert_eq!(value["shipment_status"], "ready");
        assert_eq!(value["label"]["summary"]["status"], "issued");
        assert_eq!(
            value["label"]["summary"]["tracking_url"],
            "https://tools.usps.com/go/TrackConfirmAction?tLabels=9405%205000%200000%200000%200000%2000"
        );
        // Second paste-back conflicts (already issued).
        let res = app(&fixture.state)
            .oneshot(post(
                &format!("/apostille/api/v1/p2/shipments/{id}/labels/{label_id}/paste-back"),
                Some("tok-1"),
                serde_json::json!({
                    "tracking_no": "9405 5000 0000 0000 0000 00",
                    "label_pdf_base64": pdf,
                }),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn prepaid_certified_buys_from_stub_backend() {
        let (addr, stub) = stub_backend(usps_buy_answer(), 1);
        let fixture = fixture(LabelCfg {
            usps_base_url: format!("http://{addr}"),
            usps_certified: true,
        });
        let id = create_shipment_id(&fixture, "tok-1").await;
        let res = app(&fixture.state)
            .oneshot(post(
                &format!("/apostille/api/v1/p2/shipments/{id}/purchase"),
                Some("tok-1"),
                serde_json::json!({
                    "direction": "outbound",
                    "carrier": "usps",
                    "option": {"kind": "prepaid", "paper": "letter"},
                }),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        let raw = axum::body::to_bytes(res.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(value["queued"], false);
        assert_eq!(
            value["label"]["summary"]["tracking_no"],
            "9405 5000 0000 0000 0000 00"
        );
        // Recorded cost never leaks into the API response.
        assert!(!raw.windows(4).any(|w| w == b"1250"), "{value}");
        assert!(!String::from_utf8_lossy(&raw).contains("postage"));
        let seen = stub.join().unwrap();
        assert_eq!(seen.len(), 1);
        assert!(
            seen[0].starts_with("POST /labels/v3/label HTTP/1.1\r\n"),
            "{}",
            seen[0]
        );
        let wire_body = seen[0].split("\r\n\r\n").nth(1).unwrap_or("");
        let wire: serde_json::Value = serde_json::from_str(wire_body).unwrap();
        // The documented v3 shape: imageInfo, PRIORITY_MAIL, and the
        // payment token as a header (no credentials here: the stub token).
        assert_eq!(wire["packageDescription"]["mailClass"], "PRIORITY_MAIL");
        assert_eq!(wire["imageInfo"]["labelType"], "4X6LABEL");
        assert!(
            seen[0]
                .to_ascii_lowercase()
                .contains("x-payment-authorization-token: sandbox-unbilled\r\n"),
            "{}",
            seen[0]
        );
        // ...but the cost IS recorded server-side.
        let store = Store::new(&fixture.state.store_root);
        let p2 = db::open_p2(&store).unwrap();
        let cents: Option<i64> = p2
            .query_row(
                "SELECT postage_cost_cents FROM shipment WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(cents, Some(1250));
    }

    #[tokio::test]
    async fn prepaid_outside_sandbox_fails_closed() {
        let fixture = fixture(LabelCfg {
            usps_base_url: "https://apis.usps.com".to_string(),
            usps_certified: true,
        });
        let id = create_shipment_id(&fixture, "tok-1").await;
        let res = app(&fixture.state)
            .oneshot(post(
                &format!("/apostille/api/v1/p2/shipments/{id}/purchase"),
                Some("tok-1"),
                serde_json::json!({
                    "direction": "outbound",
                    "carrier": "usps",
                    "option": {"kind": "prepaid", "paper": "four_by_six"},
                }),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        // No label row was created, and the block was audited.
        let store = Store::new(&fixture.state.store_root);
        let p2 = db::open_p2(&store).unwrap();
        let labels: i64 = p2
            .query_row(
                "SELECT COUNT(*) FROM shipping_label WHERE shipment_id = ?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(labels, 0);
        let main = db::open(&store).unwrap();
        let blocks: i64 = main
            .query_row(
                "SELECT COUNT(*) FROM audit WHERE action = 'label_purchase_blocked'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(blocks, 1);
    }

    #[tokio::test]
    async fn prepaid_rejects_fedex() {
        let fixture = fixture(LabelCfg::default());
        let id = create_shipment_id(&fixture, "tok-1").await;
        let res = app(&fixture.state)
            .oneshot(post(
                &format!("/apostille/api/v1/p2/shipments/{id}/purchase"),
                Some("tok-1"),
                serde_json::json!({
                    "direction": "outbound",
                    "carrier": "fed_ex",
                    "option": {"kind": "prepaid", "paper": "four_by_six"},
                }),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn void_printable_then_revoid_conflicts() {
        let fixture = fixture(LabelCfg::default());
        let id = create_shipment_id(&fixture, "tok-1").await;
        let res = app(&fixture.state)
            .oneshot(post(
                &format!("/apostille/api/v1/p2/shipments/{id}/purchase"),
                Some("tok-1"),
                serde_json::json!({
                    "direction": "outbound",
                    "carrier": "usps",
                    "option": {"kind": "printable", "format": "four_by_six"},
                }),
            ))
            .await
            .unwrap();
        let label_id = body_json(res).await["label"]["id"].as_i64().unwrap();
        let res = app(&fixture.state)
            .oneshot(post(
                &format!("/apostille/api/v1/p2/shipments/{id}/labels/{label_id}/void"),
                Some("tok-1"),
                serde_json::json!({}),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let value = body_json(res).await;
        assert_eq!(value["shipment_status"], "voided");
        assert!(value["void_receipt"].as_str().unwrap().len() == 64);
        let res = app(&fixture.state)
            .oneshot(post(
                &format!("/apostille/api/v1/p2/shipments/{id}/labels/{label_id}/void"),
                Some("tok-1"),
                serde_json::json!({}),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::CONFLICT);
        // Voided labels accept a paste-back reissue.
        use base64::Engine as _;
        let pdf = base64::engine::general_purpose::STANDARD.encode(b"%PDF-1.4 reissue");
        let res = app(&fixture.state)
            .oneshot(post(
                &format!("/apostille/api/v1/p2/shipments/{id}/labels/{label_id}/paste-back"),
                Some("tok-1"),
                serde_json::json!({
                    "tracking_no": "reissue-1",
                    "label_pdf_base64": pdf,
                }),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            body_json(res).await["label"]["summary"]["status"],
            "reissued"
        );
    }

    #[tokio::test]
    async fn void_certified_label_cancels_at_stub() {
        let (addr, stub) = stub_backend(usps_buy_answer(), 2);
        let fixture = fixture(LabelCfg {
            usps_base_url: format!("http://{addr}"),
            usps_certified: true,
        });
        let id = create_shipment_id(&fixture, "tok-1").await;
        let res = app(&fixture.state)
            .oneshot(post(
                &format!("/apostille/api/v1/p2/shipments/{id}/purchase"),
                Some("tok-1"),
                serde_json::json!({
                    "direction": "outbound",
                    "carrier": "usps",
                    "option": {"kind": "prepaid", "paper": "four_by_six"},
                }),
            ))
            .await
            .unwrap();
        let label_id = body_json(res).await["label"]["id"].as_i64().unwrap();
        let res = app(&fixture.state)
            .oneshot(post(
                &format!("/apostille/api/v1/p2/shipments/{id}/labels/{label_id}/void"),
                Some("tok-1"),
                serde_json::json!({}),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let seen = stub.join().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(
            seen[1].starts_with("DELETE /labels/v3/label/"),
            "{}",
            seen[1]
        );
    }

    #[tokio::test]
    async fn other_customer_sees_no_shipment() {
        let fixture = fixture(LabelCfg::default());
        let id = create_shipment_id(&fixture, "tok-1").await;
        let res = app(&fixture.state)
            .oneshot(get(
                &format!("/apostille/api/v1/p2/shipments/{id}"),
                Some("tok-2"),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn label_defaults_roundtrip_and_validate() {
        let fixture = fixture(LabelCfg::default());
        let res = app(&fixture.state)
            .oneshot(get(
                "/apostille/api/v1/p2/profile/label-defaults",
                Some("tok-1"),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(body_json(res).await["option_a_format"], "four_by_six");

        let res = app(&fixture.state)
            .oneshot(put(
                "/apostille/api/v1/p2/profile/label-defaults",
                "tok-1",
                serde_json::json!({
                    "option_a_format": "window_envelope",
                    "option_b_paper": "a4",
                }),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let value = body_json(res).await;
        assert_eq!(value["option_a_format"], "window_envelope");
        assert_eq!(value["option_b_paper"], "a4");

        let res = app(&fixture.state)
            .oneshot(put(
                "/apostille/api/v1/p2/profile/label-defaults",
                "tok-1",
                serde_json::json!({"option_a_format": "scroll"}),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    fn seed_submission(fixture: &Fixture, customer_ref: &str, status: &str) -> i64 {
        let store = Store::new(&fixture.state.store_root);
        let p2 = db::open_p2(&store).unwrap();
        p2.execute(
            "INSERT INTO form_submission(form_template_id, authority_id, customer_ref,
                    values_json, status, created_at)
             VALUES (1, 1, ?1, '{}', ?2, 1000)",
            rusqlite::params![customer_ref, status],
        )
        .unwrap();
        p2.last_insert_rowid()
    }

    fn shipment_body_with(submission_id: i64) -> serde_json::Value {
        let mut body = shipment_body();
        body["submission_id"] = serde_json::json!(submission_id);
        body
    }

    fn submission_count(fixture: &Fixture) -> i64 {
        let store = Store::new(&fixture.state.store_root);
        let p2 = db::open_p2(&store).unwrap();
        p2.query_row("SELECT COUNT(*) FROM form_submission", [], |r| r.get(0))
            .unwrap()
    }

    #[tokio::test]
    async fn shipment_links_owned_submitted_submission() {
        let fixture = fixture(LabelCfg::default());
        let sub = seed_submission(&fixture, "cust-1", "submitted");
        let res = app(&fixture.state)
            .oneshot(post(
                "/apostille/api/v1/p2/shipments",
                Some("tok-1"),
                shipment_body_with(sub),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        let id = body_json(res).await["id"].as_i64().unwrap();
        let res = app(&fixture.state)
            .oneshot(get(
                &format!("/apostille/api/v1/p2/shipments/{id}"),
                Some("tok-1"),
            ))
            .await
            .unwrap();
        assert_eq!(body_json(res).await["submission_id"], sub);
        assert_eq!(submission_count(&fixture), 1); // no shell minted
    }

    #[tokio::test]
    async fn shipment_unknown_submission_404s_without_side_effects() {
        let fixture = fixture(LabelCfg::default());
        let res = app(&fixture.state)
            .oneshot(post(
                "/apostille/api/v1/p2/shipments",
                Some("tok-1"),
                shipment_body_with(9999),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            body_json(res).await,
            serde_json::json!({"error": "no such submission"})
        );
        assert_eq!(submission_count(&fixture), 0);
    }

    #[tokio::test]
    async fn shipment_other_customer_submission_refused() {
        let fixture = fixture(LabelCfg::default());
        let sub = seed_submission(&fixture, "cust-2", "submitted");
        let res = app(&fixture.state)
            .oneshot(post(
                "/apostille/api/v1/p2/shipments",
                Some("tok-1"),
                shipment_body_with(sub),
            ))
            .await
            .unwrap();
        // File convention: owner mismatch is 404, never 403.
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            body_json(res).await,
            serde_json::json!({"error": "no such submission"})
        );
        assert_eq!(submission_count(&fixture), 1);
    }

    #[tokio::test]
    async fn shipment_non_submitted_status_refused() {
        let fixture = fixture(LabelCfg::default());
        for status in ["draft", "superseded", "voided"] {
            let sub = seed_submission(&fixture, "cust-1", status);
            let res = app(&fixture.state)
                .oneshot(post(
                    "/apostille/api/v1/p2/shipments",
                    Some("tok-1"),
                    shipment_body_with(sub),
                ))
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{status}");
            assert_eq!(
                body_json(res).await,
                serde_json::json!({"error": "submission is not submitted"}),
                "{status}"
            );
        }
    }

    #[tokio::test]
    async fn shipment_without_submission_keeps_shell_path() {
        let fixture = fixture(LabelCfg::default());
        let res = app(&fixture.state)
            .oneshot(post(
                "/apostille/api/v1/p2/shipments",
                Some("tok-1"),
                shipment_body(),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        let id = body_json(res).await["id"].as_i64().unwrap();
        let res = app(&fixture.state)
            .oneshot(get(
                &format!("/apostille/api/v1/p2/shipments/{id}"),
                Some("tok-1"),
            ))
            .await
            .unwrap();
        let sub = body_json(res).await["submission_id"].as_i64().unwrap();
        let store = Store::new(&fixture.state.store_root);
        let p2 = db::open_p2(&store).unwrap();
        let (template_id, status): (i64, String) = p2
            .query_row(
                "SELECT form_template_id, status FROM form_submission WHERE id = ?1",
                [sub],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((template_id, status.as_str()), (0, "draft"));
    }
}
