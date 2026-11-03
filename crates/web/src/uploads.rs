//! Staff "Upload official document" (E1): an official PDF or HTML page a
//! staff member downloaded themselves (sites like sos.wa.gov refuse
//! bots) becomes a `capture(kind='file', trust='staff_verified')` with a
//! text sidecar — the lopdf text layer for PDFs — so drafting, the
//! Wonder cross-check and the form map run on it like any fetch.
//!
//! Staff-only, admin listener, multipart, 15 MB cap. The official URL is
//! required (https) and recorded as a `document` source, so every fact
//! and form drafted from the file still cites where it came from.

use axum::{
    Router,
    extract::{DefaultBodyLimit, Multipart, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::post,
};
use handbook_store::{db, store::Store};
use rusqlite::OptionalExtension;
use serde_json::json;

use crate::admin::AdminAuth;
use crate::{AppState, store_unavailable, unix_now};

const MAX_BYTES: usize = 15 * 1024 * 1024;

fn bad(msg: &str) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({"error": msg})),
    )
        .into_response()
}

/// Text for the sidecar: the PDF text layer, else tag-stripped HTML.
pub fn sidecar_text(bytes: &[u8]) -> Result<String, String> {
    if bytes.starts_with(b"%PDF") {
        let text = forms::acro::text_layer(bytes).map_err(|e| e.to_string())?;
        if text.trim().is_empty() {
            return Err(
                "this PDF has no text layer (a scan): drafting needs selectable text".to_string(),
            );
        }
        return Ok(text);
    }
    let text = ingest::capture::extract_text(bytes);
    if text.trim().is_empty() {
        return Err("no readable text in this file".to_string());
    }
    Ok(text)
}

/// `POST /apostille/api/v1/admin/captures/upload` — multipart fields
/// `authority_slug`, `source_url` (https) and `file`.
async fn upload(
    auth: AdminAuth,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Response {
    let (mut slug, mut url, mut bytes) = (String::new(), String::new(), Vec::new());
    while let Ok(Some(field)) = multipart.next_field().await {
        match field.name().unwrap_or("") {
            "authority_slug" => slug = field.text().await.unwrap_or_default().trim().to_string(),
            "source_url" => url = field.text().await.unwrap_or_default().trim().to_string(),
            "file" => match field.bytes().await {
                Ok(b) if b.len() <= MAX_BYTES => bytes = b.to_vec(),
                _ => return bad("file too large (15 MB max)"),
            },
            _ => {}
        }
    }
    if slug.is_empty() || bytes.is_empty() {
        return bad("authority_slug and file are required");
    }
    if !url.starts_with("https://") || url.len() > 500 || url.contains(char::is_whitespace) {
        return bad("source_url must be the official https URL the file came from");
    }
    // PDF parsing is CPU-bound: keep it off the async workers (#35).
    let (bytes, parsed) = match tokio::task::spawn_blocking(move || {
        let text = sidecar_text(&bytes);
        let fillable = bytes.starts_with(b"%PDF")
            && forms::acro::fields(&bytes)
                .map(|f| !f.is_empty())
                .unwrap_or(false);
        (bytes, text.map(|t| (t, fillable)))
    })
    .await
    {
        Ok(v) => v,
        Err(_) => return store_unavailable(),
    };
    let (text, fillable) = match parsed {
        Ok(v) => v,
        Err(e) => return bad(&e),
    };
    let store = Store::new(&state.store_root);
    let Ok(conn) = db::open(&store) else {
        return store_unavailable();
    };
    let Some(authority_id) = conn
        .query_row("SELECT id FROM authority WHERE slug = ?1", [&slug], |r| {
            r.get::<_, i64>(0)
        })
        .optional()
        .unwrap_or(None)
    else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown authority"})),
        )
            .into_response();
    };
    let source_id: Option<i64> = conn
        .query_row("SELECT id FROM source WHERE authority_id = ?1 AND uri = ?2", rusqlite::params![authority_id, url], |r| r.get(0))
        .optional()
        .unwrap_or(None)
        .or_else(|| {
            conn.execute(
                "INSERT INTO source(authority_id, kind, uri, notes) VALUES (?1, 'document', ?2, 'staff upload')",
                rusqlite::params![authority_id, url],
            )
            .ok()
            .map(|_| conn.last_insert_rowid())
        });
    let input = ingest::capture::IngestInput {
        authority_id,
        source_id,
        kind: "file",
        trust: "staff_verified",
        bytes: &bytes,
        text: &text,
        captured_at: unix_now(),
        actor: &auth.user.user_id,
        origin: &url,
    };
    match ingest::capture::ingest_bytes(&conn, &store, &input) {
        Ok(outcome) => {
            let id = outcome.capture_id();
            (
                StatusCode::CREATED,
                Json(json!({"capture_id": id, "duplicate": matches!(outcome, ingest::capture::IngestOutcome::Duplicate { .. }),
                            "text_chars": text.chars().count(), "pdf": bytes.starts_with(b"%PDF"), "fillable_form": fillable})),
            )
                .into_response()
        }
        Err(_) => store_unavailable(),
    }
}

/// Staff routes merged onto the ADMIN listener only (wrapped in the
/// admin guard stack by the builder; the handler takes `AdminAuth`).
pub fn admin_router(state: AppState) -> Router {
    Router::<AppState>::new()
        .route(
            "/apostille/api/v1/admin/captures/upload",
            post(upload).layer(DefaultBodyLimit::max(MAX_BYTES + 64 * 1024)),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_text_layer_and_html_both_feed_the_sidecar() {
        assert!(
            sidecar_text(b"<html><body><p>Apostille fee: $15</p></body></html>")
                .unwrap()
                .contains("$15")
        );
        assert!(sidecar_text(b"<html></html>").is_err());
        assert!(sidecar_text(b"%PDF-1.7 not really").is_err());
    }
}
