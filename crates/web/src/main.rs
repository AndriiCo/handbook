//! Apostille web API (S3 lane, roadmap Day 1–2 slice).
//!
//! All routes live under `/apostille`: health/version, static files, a
//! WS echo probe, and DEMO login/session (`/api/v1/demo/login`,
//! `/api/v1/me`). No real auth: passkey/OAuth come later per
//! `docs/specs/roadmap.md`.
//!
//! NOTE: real WS topics (`authority:{slug}`, `call:{id}`,
//! `submission:{id}`, `shipment:{id}`, `billing:{id}`) come later
//! (Day 2+).

use axum::{
    Router,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Json},
    routing::{get, post},
};
use handbook::{db, store::Store};
use serde_json::json;
use std::net::SocketAddr;
use std::path::PathBuf;
use tower_http::services::{ServeDir, ServeFile};

/// Shared router state: SPA shell path plus the store root. Handlers
/// open a NEW SQLite connection per request (`rusqlite::Connection`
/// is `!Sync`, so one shared connection cannot cross await points;
/// a pool comes later).
#[derive(Clone)]
struct AppState {
    spa_index: PathBuf,
    store_root: PathBuf,
}

/// Build the full app router. `static_dir` roots `/apostille/static`;
/// `spa_dir` (the SvelteKit `build/` output) serves the app shell at
/// `/apostille/` and every other `/apostille/*` path via SPA fallback;
/// `store_root` locates the SQLite metadata DB for session handlers.
fn app(static_dir: &str, spa_dir: &str, store_root: PathBuf) -> Router {
    let spa_index = std::path::Path::new(spa_dir).join("index.html");
    let api = Router::new()
        .route("/healthz", get(healthz))
        .route("/version", get(version))
        .route("/ws/echo", get(ws_echo_upgrade))
        .route("/api/v1/demo/login", post(demo_login))
        .route("/api/v1/me", get(me))
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
        .with_state(AppState {
            spa_index,
            store_root,
        })
}

/// Serve the SPA shell for `/apostille/` exactly (the nested `/` never
/// reaches `ServeDir`'s directory-index handling, so it needs its own
/// route; deep links fall through to the `ServeDir` fallback above).
async fn serve_spa_index(State(state): State<AppState>) -> impl IntoResponse {
    match tokio::fs::read(&state.spa_index).await {
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

/// DEMO login: no password is checked (clearly-marked demo only).
/// Creates a `demo_session` row with 24h expiry and sets a
/// `demo_session` cookie. The full token is stored server-side; prod
/// upgrades to hashes before any real auth.
async fn demo_login(
    State(state): State<AppState>,
    Json(body): Json<LoginBody>,
) -> impl IntoResponse {
    if body.customer_ref.is_empty() || body.customer_ref.len() > 64 {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "invalid customer_ref"})),
        )
            .into_response();
    }
    let bytes = rand::random::<[u8; 32]>();
    let mut token = String::with_capacity(64);
    for b in bytes {
        token.push_str(&format!("{b:02x}"));
    }
    let now = unix_now();
    let conn = match db::open(&Store::new(&state.store_root)) {
        Ok(conn) => conn,
        Err(_) => return store_unavailable(),
    };
    // M1: sweep expired rows on every login so the demo table cannot
    // grow unboundedly (a per-IP rate limit is still Day-7 work).
    let _ = db::delete_expired_demo_sessions(&conn, now);
    if db::create_demo_session(&conn, &token, &body.customer_ref, now, now + 86_400).is_err() {
        return store_unavailable();
    }
    // Built manually (no cookie crate): token is hex, so always valid.
    let cookie = format!("demo_session={token}; HttpOnly; SameSite=Lax; Path=/apostille");
    let Ok(set_cookie) = HeaderValue::from_str(&cookie) else {
        return store_unavailable();
    };
    (
        StatusCode::OK,
        [(header::SET_COOKIE, set_cookie)],
        Json(json!({"customer_ref": body.customer_ref})),
    )
        .into_response()
}

/// DEMO session check: missing/unknown/expired token → 401.
async fn me(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let unauthorized = || {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized"})),
        )
            .into_response()
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
        return unauthorized();
    };
    let conn = match db::open(&Store::new(&state.store_root)) {
        Ok(conn) => conn,
        Err(_) => return store_unavailable(),
    };
    let Ok(Some((customer_ref, expires_at))) = db::get_demo_session(&conn, token) else {
        return unauthorized();
    };
    if unix_now() >= expires_at {
        return unauthorized();
    }
    (
        StatusCode::OK,
        Json(json!({"customer_ref": customer_ref, "expires_at": expires_at})),
    )
        .into_response()
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)] // m3: mass-assignment hygiene on mutating DTOs
struct LoginBody {
    customer_ref: String,
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn store_unavailable() -> axum::response::Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": "store unavailable"})),
    )
        .into_response()
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
    let store_dir = std::env::var("STORE_DIR").unwrap_or_else(|_| "store".to_string());
    let store = Store::new(&store_dir);
    if let Err(e) = store.init() {
        panic!("STORE_DIR={store_dir}: init store: {e}");
    }
    let conn =
        db::open(&store).unwrap_or_else(|e| panic!("STORE_DIR={store_dir}: open metadata DB: {e}"));
    db::migrate(&conn)
        .unwrap_or_else(|e| panic!("STORE_DIR={store_dir}: migrate metadata DB: {e}"));
    drop(conn);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("bind 127.0.0.1:PORT");
    eprintln!("listening on http://{addr}/apostille");
    axum::serve(
        listener,
        app(&static_dir, &spa_dir, PathBuf::from(store_dir)),
    )
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
        // Static/health tests never touch the DB; the path stays unused.
        app(&dir, &spa, PathBuf::from("test-store-unused"))
    }

    /// App wired to a migrated temp store (handlers read state, not env).
    fn session_app(store_root: &std::path::Path) -> Router {
        let dir = std::env::var("STATIC_DIR").unwrap_or_else(|_| "static".to_string());
        let spa = std::env::var("SPA_DIR").unwrap_or_else(|_| "test-spa".to_string());
        let store = Store::new(store_root);
        store.init().unwrap();
        let conn = db::open(&store).unwrap();
        db::migrate(&conn).unwrap();
        app(&dir, &spa, store_root.to_path_buf())
    }

    fn login_request(customer_ref: &str) -> Request<Body> {
        Request::builder()
            .uri("/apostille/api/v1/demo/login")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"customer_ref": customer_ref}).to_string(),
            ))
            .unwrap()
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

    #[tokio::test]
    async fn demo_login_then_me_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let login = session_app(tmp.path())
            .oneshot(login_request("demo-1"))
            .await
            .unwrap();
        assert_eq!(login.status(), StatusCode::OK);
        let set_cookie = login
            .headers()
            .get(axum::http::header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert!(set_cookie.starts_with("demo_session="));
        assert!(set_cookie.contains("HttpOnly"));
        assert!(set_cookie.contains("SameSite=Lax"));
        assert!(set_cookie.contains("Path=/apostille"));
        let body = axum::body::to_bytes(login.into_body(), 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["customer_ref"], "demo-1");

        let cookie = set_cookie.split(';').next().unwrap().to_string();
        let res = session_app(tmp.path())
            .oneshot(
                Request::builder()
                    .uri("/apostille/api/v1/me")
                    .header("cookie", cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["customer_ref"], "demo-1");
        assert!(v["expires_at"].as_i64().unwrap() > unix_now());
    }

    #[tokio::test]
    async fn me_without_cookie_is_unauthorized() {
        let tmp = tempfile::tempdir().unwrap();
        let res = session_app(tmp.path())
            .oneshot(
                Request::builder()
                    .uri("/apostille/api/v1/me")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        let body = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["error"], "unauthorized");
    }

    #[tokio::test]
    async fn me_with_bogus_cookie_is_unauthorized() {
        let tmp = tempfile::tempdir().unwrap();
        let res = session_app(tmp.path())
            .oneshot(
                Request::builder()
                    .uri("/apostille/api/v1/me")
                    .header("cookie", "demo_session=bogus")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        let body = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["error"], "unauthorized");
    }

    #[tokio::test]
    async fn demo_login_rejects_bad_ref() {
        let tmp = tempfile::tempdir().unwrap();
        for customer_ref in [String::new(), "x".repeat(65)] {
            let res = session_app(tmp.path())
                .oneshot(login_request(&customer_ref))
                .await
                .unwrap();
            assert_eq!(
                res.status(),
                StatusCode::UNPROCESSABLE_ENTITY,
                "ref len {}",
                customer_ref.len()
            );
        }
    }

    #[tokio::test]
    async fn demo_login_rejects_unknown_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let res = session_app(tmp.path())
            .oneshot(
                Request::builder()
                    .uri("/apostille/api/v1/demo/login")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"customer_ref": "demo-1", "admin": true}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn me_with_expired_cookie_is_unauthorized() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path());
        store.init().unwrap();
        let conn = db::open(&store).unwrap();
        db::migrate(&conn).unwrap();
        db::create_demo_session(&conn, "tok-old", "demo-1", 1000, 1001).unwrap();
        drop(conn);
        let res = session_app(tmp.path())
            .oneshot(
                Request::builder()
                    .uri("/apostille/api/v1/me")
                    .header("cookie", "demo_session=tok-old")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        let body = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["error"], "unauthorized");
    }

    #[tokio::test]
    async fn login_with_broken_store_is_500() {
        let tmp = tempfile::tempdir().unwrap();
        // A regular FILE as store root: db::open cannot create
        // `<file>/handbook.sqlite`, so handlers must 500, not panic.
        let blocker = tmp.path().join("blocker");
        std::fs::write(&blocker, b"not a dir").unwrap();
        let dir = std::env::var("STATIC_DIR").unwrap_or_else(|_| "static".to_string());
        let spa = std::env::var("SPA_DIR").unwrap_or_else(|_| "test-spa".to_string());
        let res = app(&dir, &spa, blocker)
            .oneshot(login_request("demo-1"))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["error"], "store unavailable");
    }
}
