//! `POST /auth/guest/login`: the shared `demo@andrii.ai` customer
//! sign-in, no passkey, until `auth::guest::GUEST_ENDS_AT`
//! (see `auth::guest`). It only ever signs in the one guest account,
//! and only as a customer.

use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde_json::json;
use std::net::SocketAddr;

use super::passkey;
use super::{AppState, store_unavailable, unix_now};
use auth::guest::{GUEST_USER_ID, guest_active, is_guest_email};
use auth::passkey as store;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GuestBody {
    email: String,
}

pub async fn login(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<GuestBody>,
) -> Response {
    let ip = passkey::client_ip(addr);
    login_at(&state, &ip, &headers, &body.email, unix_now())
}

fn refuse(status: StatusCode, error: &str) -> Response {
    (status, Json(json!({"error": error}))).into_response()
}

/// The sign-in at a given time. Origin check only, like the magic-link
/// consume: a first visit has no double-submit cookie yet.
fn login_at(state: &AppState, ip: &str, headers: &HeaderMap, email: &str, now: i64) -> Response {
    if !passkey::origin_ok(headers, state.admin_mtls_off) {
        return passkey::csrf_fail();
    }
    if !state.auth_limits.check(&format!("gl:{ip}"), now, 10, 60) {
        return refuse(StatusCode::TOO_MANY_REQUESTS, "rate limited");
    }
    if !is_guest_email(email) {
        return refuse(StatusCode::UNAUTHORIZED, "not the guest login");
    }
    if !guest_active(now) {
        return refuse(StatusCode::GONE, "the guest login has ended");
    }
    let Some(conn) = passkey::open_conn(state) else {
        return store_unavailable();
    };
    if store::ensure_account(&conn, GUEST_USER_ID, now).is_err() {
        return store_unavailable();
    }
    // Locking the account in the store ends the guest login early.
    match store::get_account(&conn, GUEST_USER_ID).unwrap_or(None) {
        Some(account) if account.locked_at.is_none() && account.role == "customer" => {}
        _ => {
            passkey::audit_auth(&conn, now, GUEST_USER_ID, "account_locked", "login refused");
            return refuse(StatusCode::GONE, "the guest login has ended");
        }
    }
    let session_id = auth::mint_session_id();
    if auth::create_session(
        &conn,
        &session_id,
        &auth::NewSession {
            user_id: GUEST_USER_ID,
            role: auth::Role::Customer,
            customer_ref: Some(GUEST_USER_ID),
            now,
            ip,
            user_agent: passkey::user_agent(headers),
        },
    )
    .is_err()
    {
        return store_unavailable();
    }
    passkey::audit_auth(&conn, now, GUEST_USER_ID, "guest_login", "");
    let rotation = auth::csrf::mint_token();
    let secure = !state.admin_mtls_off;
    (
        StatusCode::OK,
        passkey::set_cookies(&[
            auth::csrf_cookie(&rotation, secure),
            auth::session_cookie(auth::PUB_COOKIE, &session_id, secure),
        ]),
        Json(json!({"user_id": GUEST_USER_ID, "guest": true})),
    )
        .into_response()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use auth::guest::{GUEST_EMAIL, GUEST_ENDS_AT};

    /// Sign in at `now`; the session id on success.
    fn sign_in(state: &AppState, email: &str, now: i64) -> (StatusCode, Option<String>) {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::ORIGIN,
            axum::http::HeaderValue::from_static("https://andrii.ai"),
        );
        let res = login_at(state, "203.0.113.9", &headers, email, now);
        let status = res.status();
        let session = res
            .headers()
            .get_all(axum::http::header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find_map(|c| auth::cookie_value(c, auth::PUB_COOKIE).map(str::to_string));
        (status, session)
    }

    pub(crate) fn guest_login_ends_with_its_sessions(state: &AppState) {
        let before = GUEST_ENDS_AT - 60;
        let (status, session) = sign_in(state, " Demo@Andrii.ai ", before);
        assert_eq!(status, StatusCode::OK);
        let id = session.expect("session cookie");
        let conn = passkey::open_conn(state).unwrap();
        let row = auth::get_session(&conn, &id).unwrap().unwrap();
        assert_eq!(row.user_id, GUEST_USER_ID);
        assert_eq!(row.role, auth::Role::Customer);
        assert_eq!(row.customer_ref.as_deref(), Some(GUEST_USER_ID));
        // Working a moment before the deadline, dead from it on: the
        // session's own 12h lifetime does not carry it past.
        assert!(auth::session_usable(&conn, &row, GUEST_ENDS_AT - 1));
        assert!(!auth::session_usable(&conn, &row, GUEST_ENDS_AT));
        // And no new sign-in from the deadline on.
        let (status, session) = sign_in(state, GUEST_EMAIL, GUEST_ENDS_AT);
        assert_eq!(status, StatusCode::GONE);
        assert!(session.is_none());
    }

    pub(crate) fn guest_login_takes_only_its_address(state: &AppState) {
        let before = GUEST_ENDS_AT - 60;
        for email in ["someone@andrii.ai", "demo@andrii.ai.evil.test", ""] {
            let (status, session) = sign_in(state, email, before);
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{email}");
            assert!(session.is_none());
        }
    }

    pub(crate) fn locking_the_guest_account_ends_it_early(state: &AppState) {
        let before = GUEST_ENDS_AT - 60;
        assert_eq!(sign_in(state, GUEST_EMAIL, before).0, StatusCode::OK);
        let conn = passkey::open_conn(state).unwrap();
        conn.execute(
            "UPDATE account SET locked_at = ?1 WHERE user_id = ?2",
            rusqlite::params![before, GUEST_USER_ID],
        )
        .unwrap();
        assert_eq!(sign_in(state, GUEST_EMAIL, before).0, StatusCode::GONE);
    }
}
