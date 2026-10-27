//! P2 mount point (forms + labels only).
//!
//! S6 owns this module's future: template registry, printpdf
//! rendering, label payloads. The S3 slice mounts the router so
//! paths resolve; every P2 route answers 501 until S6 lands.

use axum::{
    Router,
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::get,
};
use serde_json::json;

/// P2 sub-router, nested at `/apostille/api/v1/p2`.
pub fn router() -> Router<crate::AppState> {
    Router::new().route("/status", get(status))
}

async fn status() -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({"status": "unimplemented", "owner": "S6"})),
    )
}
