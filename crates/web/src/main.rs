//! Apostille web API (S3 lane, roadmap Day 1 slice).
//!
//! All routes live under `/apostille`. Day 1 ships health/version,
//! static files, and a WS echo probe only.
//!
//! NOTE: auth, demo session, and real WS topics (`authority:{slug}`,
//! `call:{id}`, `submission:{id}`, `shipment:{id}`, `billing:{id}`)
//! come later per `docs/specs/roadmap.md` (Day 2+).

use axum::{
    Router,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::get,
};
use serde_json::json;
use std::net::SocketAddr;
use tower_http::services::{ServeDir, ServeFile};

/// Build the full app router. `static_dir` roots `/apostille/static`;
/// `spa_dir` (the SvelteKit `build/` output) serves the app shell at
/// `/apostille/` and every other `/apostille/*` path via SPA fallback.
fn app(static_dir: &str, spa_dir: &str) -> Router {
    let spa_index = std::path::Path::new(spa_dir).join("index.html");
    let api = Router::new()
        .route("/healthz", get(healthz))
        .route("/version", get(version))
        .route("/ws/echo", get(ws_echo_upgrade))
        .nest_service(
            "/static",
            ServeDir::new(static_dir)
                .precompressed_gzip()
                .precompressed_br(),
        )
        .fallback_service(ServeDir::new(spa_dir).fallback(ServeFile::new(spa_index.clone())));
    // axum 0.8 `nest` does not route the bare `/apostille/` (trailing
    // slash) into the nest, so the shell gets explicit outer routes.
    Router::new()
        .route("/apostille", get(serve_spa_index))
        .route("/apostille/", get(serve_spa_index))
        .nest("/apostille", api)
        .with_state(spa_index)
}

/// Serve the SPA shell for `/apostille/` exactly (the nested `/` never
/// reaches `ServeDir`'s directory-index handling, so it needs its own
/// route; deep links fall through to the `ServeDir` fallback above).
async fn serve_spa_index(State(index): State<std::path::PathBuf>) -> impl IntoResponse {
    match tokio::fs::read(&index).await {
        Ok(bytes) => (
            [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
            bytes,
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn healthz() -> impl IntoResponse {
    Json(json!({"status": "ok", "version": env!("CARGO_PKG_VERSION")}))
}

async fn version() -> impl IntoResponse {
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "build": "dev",
    }))
}

async fn ws_echo_upgrade(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(ws_echo)
}

async fn ws_echo(mut socket: WebSocket) {
    while let Some(msg) = socket.recv().await {
        match msg {
            Ok(msg) => {
                let reply = match msg {
                    Message::Text(t) => Message::Text(t),
                    Message::Binary(b) => Message::Binary(b),
                    Message::Ping(p) => Message::Pong(p),
                    Message::Pong(_) | Message::Close(_) => break,
                };
                if socket.send(reply).await.is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

#[tokio::main]
async fn main() {
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3000);
    let static_dir =
        std::env::var("STATIC_DIR").unwrap_or_else(|_| "crates/web/static".to_string());
    let spa_dir = std::env::var("SPA_DIR").unwrap_or_else(|_| "web/build".to_string());
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("bind 127.0.0.1:PORT");
    eprintln!("listening on http://{addr}/apostille");
    axum::serve(listener, app(&static_dir, &spa_dir))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("serve");
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("install ctrl-c handler");
    eprintln!("shutting down");
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    fn test_app() -> Router {
        // Tests run with cwd = crates/web, so the workspace-relative
        // defaults would miss; point at local fixture dirs.
        let dir = std::env::var("STATIC_DIR").unwrap_or_else(|_| "static".to_string());
        let spa = std::env::var("SPA_DIR").unwrap_or_else(|_| "test-spa".to_string());
        app(&dir, &spa)
    }

    #[tokio::test]
    async fn healthz_returns_ok_json() {
        let res = test_app()
            .oneshot(
                Request::builder()
                    .uri("/apostille/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["status"], "ok");
        assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
    }

    #[tokio::test]
    async fn deep_route_serves_spa_index() {
        for uri in [
            "/apostille",
            "/apostille/",
            "/apostille/nope",
            "/apostille/deep/link",
        ] {
            let res = test_app()
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::OK, "uri: {uri}");
            let body = axum::body::to_bytes(res.into_body(), 4096).await.unwrap();
            assert!(
                body.windows(b"spa-fixture".len())
                    .any(|w| w == b"spa-fixture"),
                "uri: {uri}"
            );
        }
    }

    #[tokio::test]
    async fn static_robots_txt_serves() {
        let res = test_app()
            .oneshot(
                Request::builder()
                    .uri("/apostille/static/robots.txt")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), 4096).await.unwrap();
        assert!(body.starts_with(b"User-agent:"));
    }
}
