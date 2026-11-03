//! Demo photo precheck (A2): synthetic samples + recorded live-model verdicts.
//!
//! The samples are committed geometric PNGs (`web/static/demo/` — no
//! real documents, no customer pixels). Each verdict is a replay of one
//! live run (SAM 3.1 boxes + Muse Spark judgment), recorded by the
//! operator into `<store>/demo_precheck/<sample>.json` and served
//! verbatim. Zero marginal model cost: the public surface never calls a
//! model; the only live calls happen once, at record time.
//!
//! `POST /demo/precheck` is rate-limited per IP (10/min). Unknown
//! sample ids are 422; samples with no recording yet are 503.

use std::net::SocketAddr;

use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde_json::json;

use crate::{AppState, passkey, unix_now};

/// Synthetic samples: id + label + served thumbnail.
pub const SAMPLES: [(&str, &str, &str); 3] = [
    (
        "syn-doc-clean",
        "Clean scan (synthetic)",
        "/apostille/demo/syn-doc-clean.png",
    ),
    (
        "syn-doc-dark",
        "Dark crop (synthetic)",
        "/apostille/demo/syn-doc-dark.png",
    ),
    (
        "syn-doc-wash",
        "Washed out (synthetic)",
        "/apostille/demo/syn-doc-wash.png",
    ),
];

/// `GET /api/v1/demo/precheck/samples`.
pub async fn samples() -> impl IntoResponse {
    let samples: Vec<_> = SAMPLES
        .iter()
        .map(|(id, label, thumb_url)| json!({"id": id, "label": label, "thumb_url": thumb_url}))
        .collect();
    (StatusCode::OK, Json(json!({"samples": samples}))).into_response()
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrecheckBody {
    pub sample: String,
}

/// `POST /api/v1/demo/precheck {"sample": id}`.
static PRECHECK_LIMITS: std::sync::LazyLock<passkey::RateBuckets> =
    std::sync::LazyLock::new(passkey::RateBuckets::default);

pub async fn run(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(body): Json<PrecheckBody>,
) -> impl IntoResponse {
    if !SAMPLES.iter().any(|(id, _, _)| *id == body.sample) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "unknown sample"})),
        )
            .into_response();
    }
    // A model-free read: its own map (a flood never fills the
    // demo-login map), keyed per IPv4 / IPv6 /64, admitting newcomers
    // when full (re-review of P349 #5).
    let ip = crate::admin_ip::rate_key(Some(addr.ip()));
    if !PRECHECK_LIMITS.check_read(&format!("dp:{ip}"), unix_now(), 10, 60) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "too many precheck runs — wait a minute"})),
        )
            .into_response();
    }
    let path = state
        .store_root
        .join("demo_precheck")
        .join(format!("{}.json", body.sample));
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(_) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error": "demo verdicts not recorded yet"})),
            )
                .into_response();
        }
    };
    let mut verdict: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error": "demo verdicts not recorded yet"})),
            )
                .into_response();
        }
    };
    // The recording is the verdict; the serving layer only stamps it.
    if let Some(obj) = verdict.as_object_mut() {
        obj.insert("sample".to_string(), json!(body.sample));
    }
    (StatusCode::OK, Json(verdict)).into_response()
}
