//! Meta OAuth (spec `customer-auth.md` §11, "Login with Meta" =
//! Facebook Login): GET code exchange, `debug_token` validation,
//! sub-only account resolution.
//!
//! No email scope is requested and no email is ever fetched: Meta
//! email never matches (both gates exclude it), so the flow is
//! sub-only by design. HTTP goes through
//! [`crate::oauth::HttpTransport`] so unit tests run offline; the
//! web crate provides the live ureq impl. Meta tokens are never
//! stored: verified and dropped; our sessions are the credential.

use rusqlite::{Connection, OptionalExtension};

/// Graph API pin (spec §11.1, OPEN-12 watches sunset): highest
/// valid version probed live on 2026-10-28.
pub const META_GRAPH_VERSION: &str = "v26.0";
pub const META_AUTH_URL: &str = "https://www.facebook.com/v26.0/dialog/oauth";
pub const META_TOKEN_URL: &str = "https://graph.facebook.com/v26.0/oauth/access_token";
pub const META_DEBUG_URL: &str = "https://graph.facebook.com/v26.0/debug_token";

/// Verified Meta identity: the app-scoped user id only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedMeta {
    pub sub: String,
}

/// `appsecret_proof`: lowercase hex HMAC-SHA256 of the user token
/// under the app secret (aws-lc-rs, the workspace provider — no
/// new crypto). Always sent: required when the app's "Require App
/// Secret Proof" is on, ignored otherwise.
pub fn appsecret_proof(app_secret: &str, input_token: &str) -> String {
    use aws_lc_rs::hmac;
    let key = hmac::Key::new(hmac::HMAC_SHA256, app_secret.as_bytes());
    crate::hex_of(hmac::sign(&key, input_token.as_bytes()).as_ref())
}

/// Exchange `code` for a user access token (Meta's documented GET
/// call; pairs go pre-split so the transport encodes them —
/// secrets travel in this URL). Errors carry statuses, never
/// bodies (they can echo secrets back).
pub fn exchange_code<T: crate::oauth::HttpTransport>(
    transport: &T,
    token_url: &str,
    code: &str,
    client_id: &str,
    client_secret: &str,
    redirect_uri: &str,
) -> Result<String, crate::oauth::OAuthError> {
    use crate::oauth::OAuthError;
    if client_id.is_empty() || client_secret.is_empty() {
        return Err(OAuthError::NotConfigured);
    }
    let response = transport
        .get_query(
            token_url,
            &[
                ("client_id", client_id),
                ("redirect_uri", redirect_uri),
                ("client_secret", client_secret),
                ("code", code),
            ],
        )
        .map_err(OAuthError::Transport)?;
    if !(200..300).contains(&response.status) {
        return Err(OAuthError::Exchange(response.status));
    }
    let value: serde_json::Value = serde_json::from_slice(&response.body)
        .map_err(|_| OAuthError::InvalidToken("bad token response"))?;
    value
        .get("access_token")
        .and_then(|token| token.as_str())
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .ok_or(OAuthError::InvalidToken("bad token response"))
}

/// Validate a user token via `debug_token` (spec §11.3): require
/// `is_valid`, our app id, a non-empty app-scoped `user_id`, and a
/// future `expires_at` (checked with injected `now`).
pub fn debug_verify<T: crate::oauth::HttpTransport>(
    transport: &T,
    debug_url: &str,
    user_token: &str,
    client_id: &str,
    app_secret: &str,
    now: i64,
) -> Result<VerifiedMeta, crate::oauth::OAuthError> {
    use crate::oauth::OAuthError;
    if user_token.is_empty() || client_id.is_empty() || app_secret.is_empty() {
        return Err(OAuthError::InvalidToken("debug"));
    }
    // App access token: `client_id|client_secret` pipe-joined, built
    // in memory, never logged.
    let app_token = format!("{client_id}|{app_secret}");
    let proof = appsecret_proof(app_secret, user_token);
    let response = transport
        .get_query(
            debug_url,
            &[
                ("input_token", user_token),
                ("access_token", &app_token),
                ("appsecret_proof", &proof),
            ],
        )
        .map_err(OAuthError::Transport)?;
    if response.status != 200 {
        return Err(OAuthError::InvalidToken("debug"));
    }
    let value: serde_json::Value =
        serde_json::from_slice(&response.body).map_err(|_| OAuthError::InvalidToken("debug"))?;
    let data = value.get("data").ok_or(OAuthError::InvalidToken("debug"))?;
    let valid = data
        .get("is_valid")
        .and_then(|flag| flag.as_bool())
        .unwrap_or(false);
    let app_matches = data
        .get("app_id")
        .and_then(|id| id.as_str())
        .is_some_and(|id| id == client_id);
    let user_id = data.get("user_id").and_then(|id| id.as_str()).unwrap_or("");
    let live = data
        .get("expires_at")
        .and_then(|exp| exp.as_i64())
        .is_some_and(|exp| now < exp);
    if !valid || !app_matches || user_id.is_empty() || !live {
        return Err(OAuthError::InvalidToken("debug"));
    }
    Ok(VerifiedMeta {
        sub: user_id.to_string(),
    })
}

/// Owner of a live meta link for `sub`, if any.
pub fn find_meta_owner(conn: &Connection, sub: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT user_id FROM identity_link
         WHERE method = 'meta' AND provider_sub = ?1 AND disabled_at IS NULL
         LIMIT 1",
        [sub],
        |row| row.get(0),
    )
    .optional()
}

/// Insert the meta identity link (fresh account or explicit link):
/// email hash always NULL, verified always 0 (sub-only flow).
pub fn insert_meta_link(conn: &Connection, user_id: &str, sub: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO identity_link(user_id, method, provider_sub, email_hash, verified_email, is_relay, disabled_at)
         VALUES (?1, 'meta', ?2, NULL, 0, 0, NULL)",
        rusqlite::params![user_id, sub],
    )?;
    Ok(())
}

/// Route a verified Meta identity (spec §11.4): known sub logs in;
/// unknown subs ALWAYS mint fresh accounts (`link_required` never
/// fires for Meta — the explicit link flow is the only join path).
/// Locked/role checks stay with the caller.
pub fn resolve_meta(
    conn: &Connection,
    sub: &str,
    now: i64,
) -> rusqlite::Result<crate::oauth::ResolveOutcome> {
    use crate::oauth::ResolveOutcome;
    if let Some(user_id) = find_meta_owner(conn, sub)? {
        return Ok(ResolveOutcome::Login { user_id });
    }
    let user_id = crate::passkey::mint_user_id();
    crate::passkey::ensure_account(conn, &user_id, now)?;
    insert_meta_link(conn, &user_id, sub)?;
    Ok(ResolveOutcome::Register { user_id })
}

/// Parse a Meta data-deletion `signed_request` (dashboard "Data
/// Deletion Request Callback"): `sig_b64url.payload_b64url` where
/// sig = HMAC-SHA256 over the payload *segment* under the app
/// secret. Returns the app-scoped `user_id`; None on any
/// malformation or bad signature (single failure shape, no
/// oracle). `algorithm` must read exactly `HMAC-SHA256`.
pub fn parse_deletion_request(signed_request: &str, app_secret: &str) -> Option<String> {
    use aws_lc_rs::hmac;
    use base64::Engine as _;
    let (sig_b64, payload_b64) = signed_request.split_once('.')?;
    if sig_b64.is_empty() || payload_b64.is_empty() {
        return None;
    }
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let sig = engine.decode(sig_b64).ok()?;
    let payload_bytes = engine.decode(payload_b64).ok()?;
    let key = hmac::Key::new(hmac::HMAC_SHA256, app_secret.as_bytes());
    hmac::verify(&key, payload_b64.as_bytes(), &sig).ok()?;
    let payload: serde_json::Value = serde_json::from_slice(&payload_bytes).ok()?;
    if payload.get("algorithm")?.as_str()? != "HMAC-SHA256" {
        return None;
    }
    let sub = payload.get("user_id")?.as_str()?;
    if sub.is_empty() {
        return None;
    }
    Some(sub.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Stub {
        routes: HashMap<String, crate::oauth::HttpResponse>,
    }

    impl crate::oauth::HttpTransport for Stub {
        fn post_form(
            &self,
            url: &str,
            _pairs: &[(&str, &str)],
        ) -> Result<crate::oauth::HttpResponse, String> {
            self.routes
                .get(&format!("POST {url}"))
                .cloned()
                .ok_or_else(|| "no route".to_string())
        }

        fn get(&self, url: &str) -> Result<crate::oauth::HttpResponse, String> {
            self.routes
                .get(&format!("GET {url}"))
                .cloned()
                .ok_or_else(|| "no route".to_string())
        }

        fn get_query(
            &self,
            base: &str,
            pairs: &[(&str, &str)],
        ) -> Result<crate::oauth::HttpResponse, String> {
            // Secrets ride this URL: assert the pair shape (proving
            // pre-split encoding, no hand-built query strings).
            if base == META_TOKEN_URL {
                assert!(pairs.iter().any(|(key, _)| *key == "client_secret"));
                assert!(pairs.iter().any(|(key, _)| *key == "code"));
            }
            if base == META_DEBUG_URL {
                assert!(pairs.iter().any(|(key, _)| *key == "input_token"));
                assert!(pairs.iter().any(|(key, _)| *key == "appsecret_proof"));
            }
            self.routes
                .get(&format!("GET {base}"))
                .cloned()
                .ok_or_else(|| "no route".to_string())
        }
    }

    fn memory() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::migrate_auth_tables(&conn).unwrap();
        conn
    }

    fn ok_exchange() -> crate::oauth::HttpResponse {
        crate::oauth::HttpResponse {
            status: 200,
            body: br#"{"access_token":"user-token-1","token_type":"bearer","expires_in":5000}"#
                .to_vec(),
        }
    }

    fn debug_body(valid: bool, app_id: &str, user_id: &str, expires_at: i64) -> Vec<u8> {
        format!(
            r#"{{"data":{{"app_id":"{app_id}","is_valid":{valid},"user_id":"{user_id}","expires_at":{expires_at},"scopes":["public_profile"]}}}}"#
        )
        .into_bytes()
    }

    #[test]
    fn proof_matches_rfc_4231_vector() {
        // RFC 4231 test case 1 (HMAC-SHA-256): key = 20 × 0x0b,
        // data = "Hi There".
        let key = "\u{b}".repeat(20);
        assert_eq!(
            appsecret_proof(&key, "Hi There"),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        // Proofs differ per token (sanity: not a constant).
        assert_ne!(
            appsecret_proof("secret", "token-a"),
            appsecret_proof("secret", "token-b")
        );
    }

    #[test]
    fn exchange_ok_reject_and_unconfigured() {
        use crate::oauth::OAuthError;
        let mut routes = HashMap::new();
        routes.insert(format!("GET {META_TOKEN_URL}"), ok_exchange());
        let stub = Stub { routes };
        assert_eq!(
            exchange_code(
                &stub,
                META_TOKEN_URL,
                "code",
                "app-1",
                "sec",
                "https://x/cb"
            ),
            Ok("user-token-1".to_string())
        );
        assert_eq!(
            exchange_code(&stub, META_TOKEN_URL, "code", "app-1", "", "https://x/cb"),
            Err(OAuthError::NotConfigured)
        );
        let mut routes = HashMap::new();
        routes.insert(
            format!("GET {META_TOKEN_URL}"),
            crate::oauth::HttpResponse {
                status: 400,
                body: br#"{"error":{"message":"bad code"}}"#.to_vec(),
            },
        );
        assert_eq!(
            exchange_code(
                &Stub { routes },
                META_TOKEN_URL,
                "code",
                "app-1",
                "sec",
                "https://x/cb"
            ),
            Err(OAuthError::Exchange(400))
        );
        let mut routes = HashMap::new();
        routes.insert(
            format!("GET {META_TOKEN_URL}"),
            crate::oauth::HttpResponse {
                status: 200,
                body: b"not json".to_vec(),
            },
        );
        assert_eq!(
            exchange_code(
                &Stub { routes },
                META_TOKEN_URL,
                "code",
                "app-1",
                "sec",
                "https://x/cb"
            ),
            Err(OAuthError::InvalidToken("bad token response"))
        );
    }

    #[test]
    fn debug_ok_and_rejections() {
        use crate::oauth::OAuthError;
        let good = debug_body(true, "app-1", "scoped-1", 2_000_000_000);
        let mut routes = HashMap::new();
        routes.insert(
            format!("GET {META_DEBUG_URL}"),
            crate::oauth::HttpResponse {
                status: 200,
                body: good,
            },
        );
        let stub = Stub { routes };
        assert_eq!(
            debug_verify(
                &stub,
                META_DEBUG_URL,
                "user-token-1",
                "app-1",
                "sec",
                1_500_000_000
            ),
            Ok(VerifiedMeta {
                sub: "scoped-1".to_string()
            })
        );
        for (body, why) in [
            (
                debug_body(false, "app-1", "scoped-1", 2_000_000_000),
                "invalid",
            ),
            (
                debug_body(true, "other-app", "scoped-1", 2_000_000_000),
                "app",
            ),
            (debug_body(true, "app-1", "", 2_000_000_000), "empty sub"),
            (
                debug_body(true, "app-1", "scoped-1", 1_000_000_000),
                "expired",
            ),
            (b"{}".to_vec(), "missing data"),
        ] {
            let mut routes = HashMap::new();
            routes.insert(
                format!("GET {META_DEBUG_URL}"),
                crate::oauth::HttpResponse { status: 200, body },
            );
            assert_eq!(
                debug_verify(
                    &stub_bad(&routes),
                    META_DEBUG_URL,
                    "tok",
                    "app-1",
                    "sec",
                    1_500_000_000
                ),
                Err(OAuthError::InvalidToken("debug")),
                "{why}"
            );
        }
        // Debug outage and empty token fail closed.
        let mut routes = HashMap::new();
        routes.insert(
            format!("GET {META_DEBUG_URL}"),
            crate::oauth::HttpResponse {
                status: 500,
                body: b"{}".to_vec(),
            },
        );
        assert_eq!(
            debug_verify(
                &stub_bad(&routes),
                META_DEBUG_URL,
                "tok",
                "app-1",
                "sec",
                1_500_000_000
            ),
            Err(OAuthError::InvalidToken("debug"))
        );
        assert_eq!(
            debug_verify(&stub, META_DEBUG_URL, "", "app-1", "sec", 1_500_000_000),
            Err(OAuthError::InvalidToken("debug"))
        );

        fn stub_bad(routes: &HashMap<String, crate::oauth::HttpResponse>) -> Stub {
            Stub {
                routes: routes.clone(),
            }
        }
    }

    #[test]
    fn resolve_always_fresh_unless_known() {
        use crate::oauth::ResolveOutcome;
        let conn = memory();
        // A same-address Google account exists: Meta still mints
        // fresh (no email machinery runs at all).
        crate::oauth::insert_google_link(&conn, "u-google", "g-sub-1", "m@example.test", true)
            .unwrap();
        let user = match resolve_meta(&conn, "scoped-1", 1000).unwrap() {
            ResolveOutcome::Register { user_id } => user_id,
            other => panic!("{other:?}"),
        };
        assert_eq!(
            find_meta_owner(&conn, "scoped-1").unwrap(),
            Some(user.clone())
        );
        // Link row: NULL email hash, verified 0.
        let (hash, flag): (Option<String>, i64) = conn
            .query_row(
                "SELECT email_hash, verified_email FROM identity_link
                 WHERE user_id = ?1 AND method = 'meta'",
                [&user],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((hash, flag), (None, 0));
        // Known sub: login.
        assert_eq!(
            resolve_meta(&conn, "scoped-1", 1000).unwrap(),
            ResolveOutcome::Login { user_id: user }
        );
    }

    fn signed(payload: &str, secret: &str) -> String {
        use aws_lc_rs::hmac;
        use base64::Engine as _;
        let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let body = engine.encode(payload.as_bytes());
        let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
        let sig = engine.encode(hmac::sign(&key, body.as_bytes()).as_ref());
        format!("{sig}.{body}")
    }

    #[test]
    fn deletion_request_roundtrip_and_rejects() {
        let good = signed(
            r#"{"algorithm":"HMAC-SHA256","user_id":"218471"}"#,
            "appsecret",
        );
        assert_eq!(
            parse_deletion_request(&good, "appsecret"),
            Some("218471".to_string())
        );
        // Wrong secret.
        assert_eq!(parse_deletion_request(&good, "nope"), None);
        // Tampered payload (re-signed check fails).
        let mut bad = good.clone();
        bad.replace_range(good.len() - 4.., "9999");
        assert_eq!(parse_deletion_request(&bad, "appsecret"), None);
        // Wrong algorithm.
        let other = signed(r#"{"algorithm":"none","user_id":"218471"}"#, "appsecret");
        assert_eq!(parse_deletion_request(&other, "appsecret"), None);
        // Missing/empty user_id.
        let no_user = signed(r#"{"algorithm":"HMAC-SHA256"}"#, "appsecret");
        assert_eq!(parse_deletion_request(&no_user, "appsecret"), None);
        // Malformed envelopes.
        for broken in ["", "nodot", ".", "a.", ".b", "a.b.c"] {
            assert_eq!(
                parse_deletion_request(broken, "appsecret"),
                None,
                "{broken}"
            );
        }
    }
}
