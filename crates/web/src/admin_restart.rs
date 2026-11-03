//! Staff restart from `/config` (P393):
//! `POST /apostille/api/v1/admin/restart`.
//!
//! Boot-time keys (`net.*`, `tls.*`, `auth.*`, the FedEx live
//! credentials, …) are read once by `RuntimeConfig::load`. This route
//! applies them without SSH: it first re-runs the boot config load and
//! the checks that refuse a boot (WebAuthn handle, TLS pair or ACME
//! config, bind addresses) and answers 422 with the reason when the
//! next boot would refuse to start — a refused boot under
//! `Restart=on-failure` loops into systemd's start limit and leaves
//! the service down until someone SSHes in. Otherwise it answers 202
//! with the current `started_at`, drains every listener the way
//! SIGTERM does, and exits with [`EXIT_CODE`]; systemd's
//! `Restart=on-failure` (`docs/ops/apostille.service`, unchanged)
//! starts the new process. The page polls `/admin/version` until
//! `started_at` moves.
//!
//! Guards: staff session + admin IP + admin host + CSRF (the same
//! layers as the other staff routers), one accepted restart per
//! [`MIN_INTERVAL_SECS`] (the append-only `audit` table is the clock,
//! so the limit survives the restart itself and stays far under the
//! unit's 12-starts-per-600 s limit), and a typed confirmation when
//! the pending changes touch keys that can lock staff out
//! ([`LOCKOUT_PREFIXES`]).

use std::collections::BTreeSet;
use std::net::{IpAddr, SocketAddr};
use std::sync::{LazyLock, OnceLock};

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderValue, StatusCode, header},
    response::IntoResponse,
    routing::post,
};
use handbook_store::{db, store::Store};
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::json;

use crate::{
    AppState,
    admin::{AdminAuth, admin_host_guard, csrf_guard, require_admin_ip},
    unix_now,
};

/// `EX_TEMPFAIL`: non-zero, so `Restart=on-failure` restarts us, and
/// distinct from a crash (1) or a panic abort in the journal.
pub(crate) const EXIT_CODE: i32 = 75;

/// One accepted restart per two minutes.
pub(crate) const MIN_INTERVAL_SECS: i64 = 120;

/// What staff type to confirm a restart that can lock them out.
pub(crate) const CONFIRM_WORD: &str = "restart";

/// Key families whose change can keep staff off the admin host after
/// a restart: listeners and host names, certificates, the IP
/// allowlist, passkey origin, ACME.
pub(crate) const LOCKOUT_PREFIXES: [&str; 5] = ["net.", "tls.", "admin.", "auth.", "acme."];

/// Grace before the drain starts, so the 202 reaches the browser.
const DRAIN_DELAY: std::time::Duration = std::time::Duration::from_millis(500);

/// Wakes `main`'s listener supervisor (see [`requested`]).
static RESTART: LazyLock<tokio::sync::Notify> = LazyLock::new(tokio::sync::Notify::new);

/// Listeners this process bound at boot ([`remember_boot`]). Ports in
/// this set are ours and free up in the drain, so the preflight does
/// not test-bind them.
static BOOT_LISTENERS: OnceLock<BTreeSet<Listener>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Proto {
    Tcp,
    Udp,
}

type Listener = (Proto, SocketAddr);

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/apostille/api/v1/admin/restart", post(post_restart))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            csrf_guard,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            require_admin_ip,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            admin_host_guard,
        ))
        .layer(axum::middleware::from_fn_with_state(
            "admin",
            crate::request_id_and_log,
        ))
        .with_state(state)
}

/// Record the boot config's listeners (`main`, once, after the load).
pub(crate) fn remember_boot(cfg: &config::RuntimeConfig) {
    let _ = BOOT_LISTENERS.set(listen_plan(cfg));
}

/// Resolves once staff asked for a restart. `main`'s supervisor
/// selects on it next to SIGTERM; `Notify` keeps the permit when the
/// request lands between two polls, so no request is lost.
pub(crate) async fn requested() {
    RESTART.notified().await;
}

/// Every listener the boot path binds for `cfg`: single-443 TLS
/// (pinned pair or ACME) uses [`crate::tls_listen_plan`]; plain HTTP
/// binds `net.port` and `net.admin_port` per IP (`serve_http`).
fn listen_plan(cfg: &config::RuntimeConfig) -> BTreeSet<Listener> {
    let tls = (cfg.tls_cert.is_some() && cfg.tls_key.is_some()) || cfg.acme_enabled;
    if tls {
        crate::tls_listen_plan(cfg)
            .into_iter()
            .map(|l| {
                let proto = match l.kind {
                    crate::TlsListenerKind::Quic => Proto::Udp,
                    _ => Proto::Tcp,
                };
                (proto, l.addr)
            })
            .collect()
    } else {
        cfg.bind_ips()
            .into_iter()
            .flat_map(|ip| [cfg.port, cfg.admin_port].map(|p| (Proto::Tcp, SocketAddr::new(ip, p))))
            .collect()
    }
}

/// Re-run the boot load and every check that makes `main` refuse to
/// start, against the rows as they are now. `Ok` carries the load's
/// warnings (the boot prints them); `Err` is the refusal reason.
///
/// The KEK is not re-read: under systemd it comes from the unit's
/// credential copy, which the restarted process receives again.
pub(crate) fn preflight(
    conn: &Connection,
    kek: &config::Kek,
    store_root: &std::path::Path,
    boot: &BTreeSet<Listener>,
) -> Result<Vec<String>, String> {
    let (cfg, mut warnings) = config::RuntimeConfig::load(conn, kek)
        .map_err(|e| format!("runtime config unreadable ({e})"))?;
    if crate::passkey::build_webauthn(&cfg.auth_rp_id, &cfg.auth_rp_name, &cfg.auth_origin)
        .is_none()
    {
        return Err("auth.rp_id/auth.origin do not build a WebAuthn handle".to_string());
    }
    match crate::explicit_tls_pair(cfg.tls_cert.clone(), cfg.tls_key.clone())? {
        Some((cert_pem, key_pem)) => {
            crate::load_admin_tls_from_pem(&cert_pem, &key_pem)?;
        }
        None if cfg.acme_enabled => {
            let stored = acme::StoredAcme {
                directory: cfg.acme_directory.clone(),
                domains_csv: cfg.acme_domains_csv.clone(),
                email: cfg.acme_email.clone(),
                eab_kid: cfg.acme_eab_kid.clone(),
                eab_hmac_b64: cfg
                    .acme_eab_hmac_b64
                    .as_ref()
                    .map(|secret| secret.as_str().to_string()),
            };
            acme::AcmeConfig::from_stored(
                store_root,
                cfg.acme_challenge_dir.clone().into(),
                stored,
            )
            .map_err(|e| e.to_string())?;
        }
        None => {}
    }
    check_binds(&cfg, boot)?;
    if !valid_host(&cfg.public_host) {
        warnings.push(format!(
            "net.public_host {:?} is not a host name; the boot keeps the current host",
            cfg.public_host
        ));
    }
    Ok(warnings)
}

/// Same rule as `auth::init_hosts`, without setting the process-wide
/// hosts (a preflight must not change the running server).
fn valid_host(host: &str) -> bool {
    let host = host.trim().trim_end_matches('.');
    !host.is_empty()
        && host.len() <= 200
        && host.split('.').all(|label| {
            !label.is_empty()
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

/// Every bind IP must be an address of this host, and every listener
/// the next boot binds must be free — except the ones this process
/// holds now (they free up in the drain), including a port held on
/// the wildcard address of the same family.
fn check_binds(cfg: &config::RuntimeConfig, boot: &BTreeSet<Listener>) -> Result<(), String> {
    for ip in cfg.bind_ips() {
        std::net::TcpListener::bind(SocketAddr::new(ip, 0))
            .map_err(|e| format!("bind address {ip} is not usable on this host ({e})"))?;
    }
    for (proto, addr) in listen_plan(cfg) {
        let wildcard = SocketAddr::new(unspecified(addr.ip()), addr.port());
        if boot.contains(&(proto, addr)) || boot.contains(&(proto, wildcard)) {
            continue;
        }
        let bound = match proto {
            Proto::Tcp => std::net::TcpListener::bind(addr).map(drop),
            Proto::Udp => std::net::UdpSocket::bind(addr).map(drop),
        };
        bound.map_err(|e| format!("cannot bind {proto:?} {addr} ({e})"))?;
    }
    Ok(())
}

fn unspecified(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V4(_) => IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED),
        IpAddr::V6(_) => IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED),
    }
}

/// Keys written since this process started (their new values are not
/// live yet if they are boot-time keys).
pub(crate) fn pending_keys(conn: &Connection, started_at: i64) -> Vec<String> {
    config::list_updated_at(conn)
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, at)| *at > started_at)
        .map(|(key, _)| key)
        .collect()
}

pub(crate) fn is_lockout_key(key: &str) -> bool {
    LOCKOUT_PREFIXES.iter().any(|p| key.starts_with(p))
}

/// Seconds until another restart is allowed, given the last accepted
/// one (`None`: allowed now).
fn retry_after(last: Option<i64>, now: i64) -> Option<i64> {
    let wait = last? + MIN_INTERVAL_SECS - now;
    (wait > 0).then_some(wait)
}

fn last_restart(conn: &Connection) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        "SELECT MAX(at) FROM audit WHERE action = 'restart'",
        [],
        |row| row.get(0),
    )
}

/// One `audit` row: `entity_id` is the `started_at` of the process the
/// row is about; `reason` carries key names only, never values.
fn audit(
    conn: &Connection,
    now: i64,
    actor: &str,
    action: &str,
    started_at: i64,
    reason: &str,
) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO audit(at, actor, action, entity, entity_id, before_ref, reason)
         VALUES (?1, ?2, ?3, 'service', ?4, ?5, ?6)",
        rusqlite::params![
            now,
            actor,
            action,
            started_at,
            format!("started_at:{started_at}"),
            reason
        ],
    )
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RestartBody {
    /// The typed confirmation ([`CONFIRM_WORD`]), required only when
    /// lockout keys are pending.
    #[serde(default)]
    confirm: Option<String>,
}

fn err(status: StatusCode, body: serde_json::Value) -> axum::response::Response {
    (status, Json(body)).into_response()
}

async fn post_restart(
    State(state): State<AppState>,
    auth: AdminAuth,
    Json(body): Json<RestartBody>,
) -> impl IntoResponse {
    let Some(kek) = state.kek.as_ref() else {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"error": "config unavailable"}),
        );
    };
    let Ok(mut conn) = db::open(&Store::new(&state.store_root)) else {
        return crate::store_unavailable();
    };
    let now = unix_now();
    let started_at = crate::STARTED_AT.get().copied().unwrap_or(0);
    let actor = auth.user.user_id.as_str();
    match last_restart(&conn) {
        Ok(last) => {
            if let Some(wait) = retry_after(last, now) {
                return too_soon(wait);
            }
        }
        Err(_) => return crate::store_unavailable(),
    }
    let boot = BOOT_LISTENERS.get().cloned().unwrap_or_default();
    let warnings = match preflight(&conn, kek, &state.store_root, &boot) {
        Ok(warnings) => warnings,
        Err(reason) => {
            let _ = audit(&conn, now, actor, "restart_refused", started_at, &reason);
            return err(
                StatusCode::UNPROCESSABLE_ENTITY,
                json!({
                    "error": format!("restart refused: the next boot would fail: {reason}"),
                    "reason": reason,
                }),
            );
        }
    };
    let pending = pending_keys(&conn, started_at);
    let risky: Vec<&String> = pending.iter().filter(|k| is_lockout_key(k)).collect();
    let confirmed = body
        .confirm
        .as_deref()
        .is_some_and(|c| c.trim().eq_ignore_ascii_case(CONFIRM_WORD));
    if !risky.is_empty() && !confirmed {
        return err(
            StatusCode::PRECONDITION_REQUIRED,
            json!({
                "error": format!(
                    "pending changes can lock staff out; type {CONFIRM_WORD:?} to confirm"
                ),
                "confirm_required": true,
                "confirm_word": CONFIRM_WORD,
                "risky_keys": risky,
            }),
        );
    }
    // The limit check and the audit row commit together under SQLite's
    // write lock (IMMEDIATE), so two racing requests cannot both restart.
    let accepted = (|| {
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some(wait) = retry_after(last_restart(&tx)?, now) {
            return Ok(Err(wait));
        }
        audit(&tx, now, actor, "restart", started_at, &pending.join(","))?;
        tx.commit()?;
        Ok::<_, rusqlite::Error>(Ok(()))
    })();
    match accepted {
        Ok(Ok(())) => {}
        Ok(Err(wait)) => return too_soon(wait),
        Err(_) => return crate::store_unavailable(),
    }
    tracing::warn!(
        actor = %actor,
        pending = %pending.join(","),
        "staff restart accepted: draining, exit {EXIT_CODE}"
    );
    tokio::spawn(async {
        tokio::time::sleep(DRAIN_DELAY).await;
        RESTART.notify_one();
    });
    (
        StatusCode::ACCEPTED,
        Json(json!({
            "started_at": started_at,
            "pending_keys": pending,
            "warnings": warnings,
        })),
    )
        .into_response()
}

fn too_soon(wait: i64) -> axum::response::Response {
    let mut res = err(
        StatusCode::TOO_MANY_REQUESTS,
        json!({
            "error": format!("a restart ran moments ago; try again in {wait} s"),
            "retry_after": wait,
        }),
    );
    if let Ok(value) = HeaderValue::from_str(&wait.to_string()) {
        res.headers_mut().insert(header::RETRY_AFTER, value);
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt as _;

    const CSRF: &str = "t";

    struct Fx {
        _tmp: tempfile::TempDir,
        state: AppState,
        kek: std::sync::Arc<config::Kek>,
    }

    fn fixture() -> Fx {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path());
        store.init().unwrap();
        let conn = db::open(&store).unwrap();
        db::migrate(&conn).unwrap();
        auth::migrate_auth_tables(&conn).unwrap();
        config::migrate_config_tables(&conn).unwrap();
        auth::create_session(
            &conn,
            "staff-1",
            &auth::NewSession {
                user_id: "staff-1",
                role: auth::Role::Staff,
                customer_ref: Some("cust-1"),
                now: unix_now(),
                ip: "127.0.0.1",
                user_agent: "test",
            },
        )
        .unwrap();
        let kek = std::sync::Arc::new(config::Kek::generate());
        let fx_state = AppState {
            spa_index: std::path::PathBuf::from("test-spa/index.html"),
            static_dir: std::path::PathBuf::from("static"),
            store_root: tmp.path().to_path_buf(),
            bus: crate::ws_bus::Bus::new(),
            kek: Some(kek.clone()),
            admin_mtls_off: true,
            admin_allow_ips: config::loopback_allow_ips(),
            admin_spa_dir: tmp.path().join("admin-spa"),
            media_terms_read: false,
            label_cfg: crate::p2_labels::LabelCfg::default(),
            voice_cfg: voice::VoiceConfig::default(),
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
            fedex_live: crate::fedex_live::FedexLiveState::default(),
        };
        let fx = Fx {
            _tmp: tmp,
            state: fx_state,
            kek,
        };
        // Ephemeral ports: the preflight test-binds every listener
        // this process does not hold, and the defaults (3000/3001)
        // may be taken on a dev machine. Written at process start, so
        // not pending.
        seal(&fx, config::keys::NET_PORT, "0", started_at());
        seal(&fx, config::keys::NET_ADMIN_PORT, "0", started_at());
        fx
    }

    fn started_at() -> i64 {
        crate::STARTED_AT.get().copied().unwrap_or(0)
    }

    /// Write one text row with an explicit `updated_at`.
    fn seal(fx: &Fx, key: &str, value: &str, at: i64) {
        let conn = db::open(&Store::new(&fx.state.store_root)).unwrap();
        config::set_value(
            &conn,
            &fx.kek,
            &config::SetValue {
                version: 1,
                key,
                key_type: config::ConfigType::Text,
                plaintext: value.as_bytes(),
                now: at,
                actor: "test",
            },
        )
        .unwrap();
    }

    fn post(body: serde_json::Value) -> Request<Body> {
        Request::builder()
            .uri("/apostille/api/v1/admin/restart")
            .method("POST")
            .header("host", "admin.andrii.ai")
            .header(
                "cookie",
                format!("{}=staff-1; {}={CSRF}", auth::ADM_COOKIE, auth::CSRF_COOKIE),
            )
            .header(auth::CSRF_HEADER, CSRF)
            .header("origin", "https://admin.andrii.ai")
            .header("content-type", "application/json")
            .extension(axum::extract::ConnectInfo(
                "127.0.0.1:443".parse::<SocketAddr>().unwrap(),
            ))
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    async fn send(
        fx: &Fx,
        req: Request<Body>,
    ) -> (StatusCode, axum::http::HeaderMap, serde_json::Value) {
        let res = router(fx.state.clone()).oneshot(req).await.unwrap();
        let status = res.status();
        let headers = res.headers().clone();
        let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
            .await
            .unwrap();
        (status, headers, serde_json::from_slice(&bytes).unwrap())
    }

    fn audit_actions(fx: &Fx) -> Vec<String> {
        let conn = db::open(&Store::new(&fx.state.store_root)).unwrap();
        let mut stmt = conn
            .prepare("SELECT action FROM audit WHERE entity = 'service' ORDER BY id")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<String>>>()
            .unwrap()
    }

    #[tokio::test]
    async fn accepted_restart_answers_202_then_rate_limits() {
        let fx = fixture();
        let (status, _h, v) = send(&fx, post(json!({}))).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{v}");
        assert_eq!(v["started_at"], json!(started_at()));
        assert_eq!(v["pending_keys"], json!([]));
        assert_eq!(audit_actions(&fx), vec!["restart"]);

        // A second request inside two minutes: 429 with Retry-After.
        let (status, headers, v) = send(&fx, post(json!({}))).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{v}");
        let wait: i64 = headers[header::RETRY_AFTER]
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        assert!((1..=MIN_INTERVAL_SECS).contains(&wait), "{wait}");
        assert_eq!(v["retry_after"], json!(wait));
        assert_eq!(audit_actions(&fx), vec!["restart"], "429 restarts nothing");
    }

    #[test]
    fn retry_after_counts_down_two_minutes() {
        assert_eq!(retry_after(None, 1_000), None);
        assert_eq!(retry_after(Some(1_000), 1_000), Some(120));
        assert_eq!(retry_after(Some(1_000), 1_119), Some(1));
        assert_eq!(retry_after(Some(1_000), 1_120), None);
    }

    #[tokio::test]
    async fn a_boot_that_would_fail_answers_422_with_the_reason() {
        let fx = fixture();
        // Half a TLS pair: `main` refuses this boot.
        seal(
            &fx,
            config::keys::TLS_CERT_FILE,
            "/nonexistent/cert.pem",
            started_at(),
        );
        let (status, _h, v) = send(&fx, post(json!({ "confirm": "restart" }))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
        assert_eq!(
            v["reason"],
            json!("set both tls.cert_file and tls.key_file, or neither")
        );
        assert!(
            v["error"]
                .as_str()
                .unwrap()
                .starts_with("restart refused: the next boot would fail")
        );
        // Refusals are audited and never use up the rate limit.
        assert_eq!(audit_actions(&fx), vec!["restart_refused"]);
        seal(
            &fx,
            config::keys::TLS_KEY_FILE,
            "/nonexistent/key.pem",
            started_at(),
        );
        let (status, _h, v) = send(&fx, post(json!({ "confirm": "restart" }))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
        assert!(
            v["reason"]
                .as_str()
                .unwrap()
                .starts_with("read tls.cert_file=")
        );
    }

    #[tokio::test]
    async fn a_bind_address_this_host_lacks_answers_422() {
        let fx = fixture();
        // TEST-NET-3: never assigned to a local interface.
        seal(&fx, config::keys::NET_BIND, "203.0.113.77", started_at());
        let (status, _h, v) = send(&fx, post(json!({ "confirm": "restart" }))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
        assert!(
            v["reason"]
                .as_str()
                .unwrap()
                .starts_with("bind address 203.0.113.77 is not usable"),
            "{v}"
        );
    }

    #[tokio::test]
    async fn lockout_keys_need_the_typed_confirmation() {
        let fx = fixture();
        // Written after this process started: pending.
        let later = started_at() + 1;
        seal(&fx, config::keys::ADMIN_ALLOW_IPS, "127.0.0.1/32", later);
        seal(&fx, config::keys::LABELS_FEDEX_ACCOUNT, "123456789", later);
        let (status, _h, v) = send(&fx, post(json!({}))).await;
        assert_eq!(status, StatusCode::PRECONDITION_REQUIRED, "{v}");
        assert_eq!(v["confirm_required"], json!(true));
        assert_eq!(v["risky_keys"], json!(["admin.allow_ips"]));
        let (status, _h, _v) = send(&fx, post(json!({ "confirm": "yes" }))).await;
        assert_eq!(status, StatusCode::PRECONDITION_REQUIRED);
        assert!(audit_actions(&fx).is_empty());
        let (status, _h, v) = send(&fx, post(json!({ "confirm": " Restart " }))).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{v}");
        assert_eq!(
            v["pending_keys"],
            json!(["admin.allow_ips", "labels.fedex_account"])
        );
    }

    #[tokio::test]
    async fn restart_needs_staff_and_csrf() {
        let fx = fixture();
        let mut no_csrf = post(json!({}));
        no_csrf.headers_mut().remove(auth::CSRF_HEADER);
        let (status, _h, _v) = send(&fx, no_csrf).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let mut no_session = post(json!({}));
        no_session.headers_mut().insert(
            header::COOKIE,
            HeaderValue::from_str(&format!("{}={CSRF}", auth::CSRF_COOKIE)).unwrap(),
        );
        let (status, _h, _v) = send(&fx, no_session).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(audit_actions(&fx).is_empty());
    }

    #[test]
    fn lockout_prefixes_cover_the_risky_families() {
        for key in [
            "net.bind",
            "net.public_host",
            "tls.cert_file",
            "admin.allow_ips",
            "auth.origin",
            "acme.enabled",
        ] {
            assert!(is_lockout_key(key), "{key}");
        }
        for key in ["labels.fedex_account", "labels.live", "stripe.secret_key"] {
            assert!(!is_lockout_key(key), "{key}");
        }
    }

    #[test]
    fn boot_listeners_are_skipped_including_the_wildcard() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        config::migrate_config_tables(&conn).unwrap();
        let kek = config::Kek::generate();
        let (cfg, _) = config::RuntimeConfig::load(&conn, &kek).unwrap();
        // Hold the planned ports as if this process had bound them.
        let held: Vec<std::net::TcpListener> = listen_plan(&cfg)
            .iter()
            .filter_map(|(_, addr)| std::net::TcpListener::bind(addr).ok())
            .collect();
        if held.len() == listen_plan(&cfg).len() {
            assert!(check_binds(&cfg, &listen_plan(&cfg)).is_ok());
            let wildcard: BTreeSet<Listener> = listen_plan(&cfg)
                .iter()
                .map(|(p, a)| (*p, SocketAddr::new(unspecified(a.ip()), a.port())))
                .collect();
            assert!(check_binds(&cfg, &wildcard).is_ok());
            assert!(
                check_binds(&cfg, &BTreeSet::new()).is_err(),
                "held ports are busy"
            );
        }
        drop(held);
    }
}
