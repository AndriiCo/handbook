//! Google OAuth code flow (spec `customer-auth.md` §9): state,
//! code exchange, tokeninfo verify, account resolve.
//!
//! HTTP goes through [`HttpTransport`] so unit tests run offline;
//! the web crate provides the live ureq impl. This crate never
//! sees a request. Google tokens are never stored: the exchange
//! result is verified and dropped; our sessions are the credential.

use rusqlite::{Connection, OptionalExtension};

/// Challenge kind reusing `webauthn_challenge` (no migration).
pub const KIND_OAUTH: &str = "oauth";
pub const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
pub const GOOGLE_TOKENINFO_URL: &str = "https://oauth2.googleapis.com/tokeninfo";
pub const OAUTH_SCOPE: &str = "openid email profile";

/// Raw HTTP response: status + bytes.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

/// Minimal blocking HTTP surface the flow needs. Pairs go
/// pre-split so transports encode them (no hand-rolled escaping).
pub trait HttpTransport {
    fn post_form(&self, url: &str, pairs: &[(&str, &str)]) -> Result<HttpResponse, String>;
    fn get(&self, url: &str) -> Result<HttpResponse, String>;
}

/// Verified Google identity (tokeninfo claims we keep).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedGoogle {
    pub sub: String,
    pub email: String,
    pub email_verified: bool,
}

/// What went wrong. Callers map every variant to one generic
/// `oauth_failed` surface (plus `oauth_unavailable` for
/// [`OAuthError::NotConfigured`]); statuses never leave the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OAuthError {
    NotConfigured,
    Transport(String),
    /// Token endpoint rejected the code (status kept for audit).
    Exchange(u16),
    /// tokeninfo rejected the token or a claim failed (static tag).
    InvalidToken(&'static str),
}

/// Start mode, stored in the state row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OauthMode {
    Login,
    Link,
}

/// `state_json` shape for `kind='oauth'` rows.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OauthState {
    pub mode: OauthMode,
    /// Split secret: start sets it as the `hb_oauth` cookie; the
    /// callback requires cookie == row (spec §9.2).
    pub binding: String,
}

/// Outcome of taking an OAuth state row exactly once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TakeOutcome {
    /// Missing, consumed, or expired (caller → `oauth_expired`).
    Missing,
    /// Consumed but unparsable: tamper or bug (caller → `oauth_failed`).
    Corrupt,
    /// Fresh row, now consumed.
    Taken {
        link_user: Option<String>,
        mode: OauthMode,
        binding: String,
    },
}

/// Fresh 256-bit state handle, hex (same shape as session ids).
pub fn mint_state() -> String {
    crate::mint_session_id()
}

/// Store state; `link_user` pins explicit-link mode (`None` =
/// logged-out login/register). Sweeps expired rows first.
pub fn store_state(
    conn: &Connection,
    state: &str,
    mode: OauthMode,
    link_user: Option<&str>,
    binding: &str,
    now: i64,
) -> rusqlite::Result<()> {
    let _ = crate::passkey::sweep_challenges(conn, now);
    // Infallible: both fields are internally minted (enum + hex).
    let state_json = serde_json::to_string(&OauthState {
        mode,
        binding: binding.to_string(),
    })
    .unwrap_or_default();
    crate::passkey::store_challenge(
        conn,
        state,
        &crate::passkey::NewChallenge {
            kind: KIND_OAUTH,
            user_id: link_user,
            state_json: &state_json,
            now,
        },
    )
}

/// Take state exactly once (consume-before-exchange, mirroring
/// the passkey finish handlers).
pub fn take_state(conn: &Connection, state: &str, now: i64) -> rusqlite::Result<TakeOutcome> {
    let Some((link_user, state_json)) =
        crate::passkey::take_challenge(conn, state, KIND_OAUTH, now)?
    else {
        return Ok(TakeOutcome::Missing);
    };
    let parsed: OauthState = match serde_json::from_str(&state_json) {
        Ok(parsed) => parsed,
        Err(_) => return Ok(TakeOutcome::Corrupt),
    };
    Ok(TakeOutcome::Taken {
        link_user,
        mode: parsed.mode,
        binding: parsed.binding,
    })
}

/// Stamp the consumed state row's owner (login mode stored `NULL`;
/// the stamp lets `proof_fresh` double as this login's unlink
/// re-auth proof).
pub fn stamp_state_user(conn: &Connection, state: &str, user_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE webauthn_challenge SET user_id = ?1 WHERE id = ?2",
        rusqlite::params![user_id, state],
    )?;
    Ok(())
}

/// Exchange `code` for an ID token. Secrets travel only in the
/// POST body; errors carry statuses, never bodies (they can echo
/// secrets back).
pub fn exchange_code<T: HttpTransport>(
    transport: &T,
    token_url: &str,
    code: &str,
    client_id: &str,
    secret: &str,
    redirect_uri: &str,
) -> Result<String, OAuthError> {
    if client_id.is_empty() || secret.is_empty() {
        return Err(OAuthError::NotConfigured);
    }
    let response = transport
        .post_form(
            token_url,
            &[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("client_id", client_id),
                ("client_secret", secret),
                ("redirect_uri", redirect_uri),
            ],
        )
        .map_err(OAuthError::Transport)?;
    if !(200..300).contains(&response.status) {
        return Err(OAuthError::Exchange(response.status));
    }
    let value: serde_json::Value = serde_json::from_slice(&response.body)
        .map_err(|_| OAuthError::InvalidToken("bad token response"))?;
    value
        .get("id_token")
        .and_then(|token| token.as_str())
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .ok_or(OAuthError::InvalidToken("bad token response"))
}

/// Verify an ID token via Google tokeninfo (spec decision: no
/// local JWT/JWK stack — zero new crypto deps). Checks `aud` is
/// our client id, `iss`, and `exp`.
pub fn tokeninfo_verify<T: HttpTransport>(
    transport: &T,
    tokeninfo_url: &str,
    id_token: &str,
    expect_aud: &str,
    now: i64,
) -> Result<VerifiedGoogle, OAuthError> {
    // JWT alphabet is query-safe; anything else fails closed here
    // instead of warping the tokeninfo URL.
    let token_ok = !id_token.is_empty()
        && id_token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    if !token_ok {
        return Err(OAuthError::InvalidToken("bad token chars"));
    }
    let response = transport
        .get(&format!("{tokeninfo_url}?id_token={id_token}"))
        .map_err(OAuthError::Transport)?;
    if response.status != 200 {
        return Err(OAuthError::InvalidToken("tokeninfo rejected"));
    }
    let value: serde_json::Value = serde_json::from_slice(&response.body)
        .map_err(|_| OAuthError::InvalidToken("unparseable"))?;
    let field = |key: &str| {
        value
            .get(key)
            .and_then(|item| item.as_str())
            .unwrap_or("")
            .to_string()
    };
    if field("aud") != expect_aud {
        return Err(OAuthError::InvalidToken("aud mismatch"));
    }
    if !matches!(
        field("iss").as_str(),
        "https://accounts.google.com" | "accounts.google.com"
    ) {
        return Err(OAuthError::InvalidToken("bad iss"));
    }
    let exp: i64 = field("exp")
        .parse()
        .map_err(|_| OAuthError::InvalidToken("bad exp"))?;
    if now >= exp {
        return Err(OAuthError::InvalidToken("expired"));
    }
    let sub = field("sub");
    if sub.is_empty() {
        return Err(OAuthError::InvalidToken("no sub"));
    }
    Ok(VerifiedGoogle {
        sub,
        email: field("email"),
        email_verified: field("email_verified") == "true",
    })
}

/// Owner of a live google link for `sub`, if any.
pub fn find_google_owner(conn: &Connection, sub: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT user_id FROM identity_link
         WHERE method = 'google' AND provider_sub = ?1 AND disabled_at IS NULL
         LIMIT 1",
        [sub],
        |row| row.get(0),
    )
    .optional()
}

/// Insert the google identity link (fresh account or explicit
/// link). Emails hash lowercase like every other link writer.
pub fn insert_google_link(
    conn: &Connection,
    user_id: &str,
    sub: &str,
    email: &str,
    verified: bool,
) -> rusqlite::Result<()> {
    let (hash, flag) = if email.is_empty() {
        (None, 0)
    } else {
        (
            Some(crate::hash_str(&email.to_lowercase())),
            i32::from(verified),
        )
    };
    conn.execute(
        "INSERT INTO identity_link(user_id, method, provider_sub, email_hash, verified_email, is_relay, disabled_at)
         VALUES (?1, 'google', ?2, ?3, ?4, 0, NULL)",
        rusqlite::params![user_id, sub, hash, flag],
    )?;
    Ok(())
}

/// Login-mode routing for a verified Google identity. Link mode
/// stays with the caller (it owns the session proof).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveOutcome {
    /// Known sub: session for this user.
    Login { user_id: String },
    /// Unknown sub, no usable email match: fresh account minted.
    Register { user_id: String },
    /// Unknown sub WITH a usable email match: NEVER merge (rule
    /// 5) — caller surfaces the explicit-link hint. No writes.
    PromptExplicitLink,
}

/// Route a verified identity. Locked/role checks stay with the
/// caller (it owns the session + audit writes).
pub fn resolve_google(
    conn: &Connection,
    sub: &str,
    email: &str,
    verified: bool,
    now: i64,
) -> rusqlite::Result<ResolveOutcome> {
    if let Some(user_id) = find_google_owner(conn, sub)? {
        return Ok(ResolveOutcome::Login { user_id });
    }
    let identity = crate::login::Identity {
        method: crate::login::LoginMethod::Google,
        provider_sub: sub.to_string(),
        verified_email: verified
            .then(|| email.to_string())
            .filter(|value| !value.is_empty()),
        is_relay: false,
    };
    let matched = crate::login::email_usable_for_matching(&identity)
        && crate::passkey::usable_email_owner(conn, email)?.is_some();
    match crate::login::decide_unknown_sub(matched) {
        crate::login::NewIdentityDecision::CreateFreshAccount => {
            let user_id = crate::passkey::mint_user_id();
            crate::passkey::ensure_account(conn, &user_id, now)?;
            insert_google_link(conn, &user_id, sub, email, verified)?;
            Ok(ResolveOutcome::Register { user_id })
        }
        crate::login::NewIdentityDecision::PromptExplicitLink => {
            Ok(ResolveOutcome::PromptExplicitLink)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Stub {
        routes: HashMap<String, HttpResponse>,
    }

    impl HttpTransport for Stub {
        fn post_form(&self, url: &str, _pairs: &[(&str, &str)]) -> Result<HttpResponse, String> {
            self.routes
                .get(&format!("POST {url}"))
                .cloned()
                .ok_or_else(|| "no route".to_string())
        }

        fn get(&self, url: &str) -> Result<HttpResponse, String> {
            // Path shape asserted without logging tokens.
            assert!(url.contains("id_token="), "{url}");
            self.routes
                .get("GET tokeninfo")
                .cloned()
                .ok_or_else(|| "no route".to_string())
        }
    }

    fn memory() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::migrate_auth_tables(&conn).unwrap();
        conn
    }

    fn tokeninfo_body(aud: &str, exp: i64, verified: &str) -> Vec<u8> {
        format!(
            r#"{{"aud":"{aud}","iss":"https://accounts.google.com","sub":"sub-1","exp":"{exp}","email":"a@example.test","email_verified":"{verified}"}}"#
        )
        .into_bytes()
    }

    fn ok_exchange() -> HttpResponse {
        HttpResponse {
            status: 200,
            body: br#"{"access_token":"at","id_token":"id-token-1","token_type":"Bearer"}"#
                .to_vec(),
        }
    }

    #[test]
    fn state_stores_takes_once_then_missing() {
        let conn = memory();
        store_state(&conn, "st-1", OauthMode::Login, None, "bind-1", 1000).unwrap();
        assert_eq!(
            take_state(&conn, "st-1", 1100).unwrap(),
            TakeOutcome::Taken {
                link_user: None,
                mode: OauthMode::Login,
                binding: "bind-1".to_string(),
            }
        );
        assert_eq!(
            take_state(&conn, "st-1", 1100).unwrap(),
            TakeOutcome::Missing
        );
        assert_eq!(
            take_state(&conn, "absent", 1100).unwrap(),
            TakeOutcome::Missing
        );
    }

    #[test]
    fn state_carries_link_mode_and_user() {
        let conn = memory();
        store_state(&conn, "st-2", OauthMode::Link, Some("u-1"), "bind-2", 1000).unwrap();
        assert_eq!(
            take_state(&conn, "st-2", 1100).unwrap(),
            TakeOutcome::Taken {
                link_user: Some("u-1".to_string()),
                mode: OauthMode::Link,
                binding: "bind-2".to_string(),
            }
        );
    }

    #[test]
    fn state_expires_and_corrupt_is_flagged() {
        let conn = memory();
        store_state(&conn, "st-old", OauthMode::Login, None, "b", 1000).unwrap();
        assert_eq!(
            take_state(&conn, "st-old", 1000 + crate::passkey::CHALLENGE_TTL_SECS).unwrap(),
            TakeOutcome::Missing
        );
        crate::passkey::store_challenge(
            &conn,
            "st-bad",
            &crate::passkey::NewChallenge {
                kind: KIND_OAUTH,
                user_id: None,
                state_json: "not json",
                now: 1000,
            },
        )
        .unwrap();
        assert_eq!(
            take_state(&conn, "st-bad", 1100).unwrap(),
            TakeOutcome::Corrupt
        );
    }

    #[test]
    fn exchange_ok_reject_and_unconfigured() {
        let mut routes = HashMap::new();
        routes.insert("POST https://token.test".to_string(), ok_exchange());
        let stub = Stub { routes };
        assert_eq!(
            exchange_code(
                &stub,
                "https://token.test",
                "code",
                "cid",
                "sec",
                "https://x/cb"
            ),
            Ok("id-token-1".to_string())
        );
        assert_eq!(
            exchange_code(
                &stub,
                "https://token.test",
                "code",
                "cid",
                "",
                "https://x/cb"
            ),
            Err(OAuthError::NotConfigured)
        );
        let mut routes = HashMap::new();
        routes.insert(
            "POST https://token.test".to_string(),
            HttpResponse {
                status: 400,
                body: br#"{"error":"invalid_grant"}"#.to_vec(),
            },
        );
        let stub = Stub { routes };
        assert_eq!(
            exchange_code(
                &stub,
                "https://token.test",
                "code",
                "cid",
                "sec",
                "https://x/cb"
            ),
            Err(OAuthError::Exchange(400))
        );
        let mut routes = HashMap::new();
        routes.insert(
            "POST https://token.test".to_string(),
            HttpResponse {
                status: 200,
                body: b"not json".to_vec(),
            },
        );
        let stub = Stub { routes };
        assert_eq!(
            exchange_code(
                &stub,
                "https://token.test",
                "code",
                "cid",
                "sec",
                "https://x/cb"
            ),
            Err(OAuthError::InvalidToken("bad token response"))
        );
    }

    #[test]
    fn tokeninfo_accepts_good_claims() {
        let mut routes = HashMap::new();
        routes.insert(
            "GET tokeninfo".to_string(),
            HttpResponse {
                status: 200,
                body: tokeninfo_body("cid", 2000, "true"),
            },
        );
        let stub = Stub { routes };
        assert_eq!(
            tokeninfo_verify(&stub, "https://ti.test", "aaa.bbb.ccc", "cid", 1000),
            Ok(VerifiedGoogle {
                sub: "sub-1".to_string(),
                email: "a@example.test".to_string(),
                email_verified: true,
            })
        );
    }

    #[test]
    fn tokeninfo_rejects_claim_failures() {
        for (name, status, body, token) in [
            ("aud", 200, tokeninfo_body("other", 2000, "true"), "aaa.bbb.ccc"),
            (
                "iss",
                200,
                br#"{"aud":"cid","iss":"https://evil.test","sub":"s","exp":"2000","email":"a@example.test","email_verified":"true"}"#.to_vec(),
                "aaa.bbb.ccc",
            ),
            ("exp", 200, tokeninfo_body("cid", 1000, "true"), "aaa.bbb.ccc"),
            ("http", 400, b"{}".to_vec(), "aaa.bbb.ccc"),
            ("chars", 200, tokeninfo_body("cid", 2000, "true"), "has space"),
        ] {
            let mut routes = HashMap::new();
            routes.insert("GET tokeninfo".to_string(), HttpResponse { status, body });
            let stub = Stub { routes };
            assert!(
                tokeninfo_verify(&stub, "https://ti.test", token, "cid", 1000).is_err(),
                "{name} must fail"
            );
        }
        // Unverified email verifies the sub but flags it.
        let mut routes = HashMap::new();
        routes.insert(
            "GET tokeninfo".to_string(),
            HttpResponse {
                status: 200,
                body: tokeninfo_body("cid", 2000, "false"),
            },
        );
        let stub = Stub { routes };
        let verified =
            tokeninfo_verify(&stub, "https://ti.test", "aaa.bbb.ccc", "cid", 1000).unwrap();
        assert!(!verified.email_verified);
    }

    #[test]
    fn resolve_known_sub_logs_in() {
        let conn = memory();
        crate::passkey::ensure_account(&conn, "u-1", 1000).unwrap();
        insert_google_link(&conn, "u-1", "sub-1", "a@example.test", true).unwrap();
        assert_eq!(
            resolve_google(&conn, "sub-1", "a@example.test", true, 1100).unwrap(),
            ResolveOutcome::Login {
                user_id: "u-1".to_string()
            }
        );
    }

    #[test]
    fn resolve_fresh_sub_mints_account_and_link() {
        let conn = memory();
        let outcome = resolve_google(&conn, "sub-9", "new@example.test", true, 1000).unwrap();
        let ResolveOutcome::Register { user_id } = outcome else {
            panic!("must register, got {outcome:?}");
        };
        assert_eq!(find_google_owner(&conn, "sub-9").unwrap(), Some(user_id));
    }

    #[test]
    fn resolve_verified_email_match_prompts_and_writes_nothing() {
        let conn = memory();
        crate::passkey::ensure_account(&conn, "u-1", 1000).unwrap();
        insert_google_link(&conn, "u-1", "sub-old", "taken@example.test", true).unwrap();
        assert_eq!(
            resolve_google(&conn, "sub-new", "taken@example.test", true, 1100).unwrap(),
            ResolveOutcome::PromptExplicitLink
        );
        // Rule 5: the new sub gains no row anywhere.
        assert_eq!(find_google_owner(&conn, "sub-new").unwrap(), None);
    }

    #[test]
    fn resolve_unverified_email_never_matches() {
        let conn = memory();
        crate::passkey::ensure_account(&conn, "u-1", 1000).unwrap();
        insert_google_link(&conn, "u-1", "sub-old", "taken@example.test", true).unwrap();
        let outcome = resolve_google(&conn, "sub-new", "taken@example.test", false, 1100).unwrap();
        assert!(
            matches!(outcome, ResolveOutcome::Register { .. }),
            "{outcome:?}"
        );
    }

    #[test]
    fn stamp_makes_proof_fresh() {
        let conn = memory();
        store_state(&conn, "st-3", OauthMode::Login, None, "b", 1000).unwrap();
        take_state(&conn, "st-3", 1100).unwrap();
        assert!(!crate::passkey::proof_fresh(&conn, "st-3", "u-1", 1100).unwrap());
        stamp_state_user(&conn, "st-3", "u-1").unwrap();
        assert!(crate::passkey::proof_fresh(&conn, "st-3", "u-1", 1100).unwrap());
    }
}
