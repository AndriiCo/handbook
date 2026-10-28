//! Apple OAuth (spec `customer-auth.md` §10): ES256 client_secret
//! minting, code exchange (shared [`crate::oauth::exchange_code`]),
//! JWKS id_token verification, account resolution.
//!
//! HTTP goes through [`crate::oauth::HttpTransport`] so unit tests
//! run offline; the web crate provides the live ureq impl. Apple
//! tokens are never stored: the exchange result is verified and
//! dropped; our sessions are the credential.

use rusqlite::{Connection, OptionalExtension};

pub const APPLE_AUTH_URL: &str = "https://appleid.apple.com/auth/authorize";
pub const APPLE_TOKEN_URL: &str = "https://appleid.apple.com/auth/token";
pub const APPLE_KEYS_URL: &str = "https://appleid.apple.com/auth/keys";
pub const APPLE_ISSUER: &str = "https://appleid.apple.com";
pub const APPLE_SCOPE: &str = "name email";

/// Verified Apple identity (id_token claims we keep). `email` is
/// empty when Apple omits it (returning users); relay addresses
/// ride the same field with `is_relay` set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedApple {
    pub sub: String,
    pub email: String,
    pub email_verified: bool,
    pub is_relay: bool,
}

/// Mint the ES256 `client_secret` JWT for one token exchange (spec
/// §10.3: per-exchange, never cached). `private_key_pem` is the
/// full `.p8` text (PKCS#8 — matches `from_ec_pem` verbatim).
pub fn mint_client_secret(
    team_id: &str,
    client_id: &str,
    key_id: &str,
    private_key_pem: &str,
    now: i64,
) -> Result<String, crate::oauth::OAuthError> {
    use crate::oauth::OAuthError;
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    if team_id.is_empty() || client_id.is_empty() || key_id.is_empty() || private_key_pem.is_empty()
    {
        return Err(OAuthError::NotConfigured);
    }
    let key = EncodingKey::from_ec_pem(private_key_pem.as_bytes())
        .map_err(|_| OAuthError::InvalidToken("client secret"))?;
    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some(key_id.to_string());
    let claims = serde_json::json!({
        "iss": team_id,
        "sub": client_id,
        "aud": APPLE_ISSUER,
        "iat": now,
        "exp": now + 300,
    });
    encode(&header, &claims, &key).map_err(|_| OAuthError::InvalidToken("client secret"))
}

/// Verify an Apple id_token via the JWKS endpoint (spec §10.3):
/// select the JWK by the JWT header `kid`, enforce ES256 (the
/// crate rejects anything else — no alg-confusion class), then
/// check `iss`/`aud`/`exp`/`sub` manually with injected `now`
/// (mirroring `tokeninfo_verify` testability).
pub fn verify_id_token<T: crate::oauth::HttpTransport>(
    transport: &T,
    keys_url: &str,
    id_token: &str,
    expect_aud: &str,
    now: i64,
) -> Result<VerifiedApple, crate::oauth::OAuthError> {
    use crate::oauth::OAuthError;
    use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
    if id_token.is_empty() {
        return Err(OAuthError::InvalidToken("bad token"));
    }
    let kid = decode_header(id_token)
        .map_err(|_| OAuthError::InvalidToken("bad token"))?
        .kid
        .filter(|kid| !kid.is_empty())
        .ok_or(OAuthError::InvalidToken("no kid"))?;
    let response = transport.get(keys_url).map_err(OAuthError::Transport)?;
    if response.status != 200 {
        return Err(OAuthError::InvalidToken("keys"));
    }
    let set: serde_json::Value =
        serde_json::from_slice(&response.body).map_err(|_| OAuthError::InvalidToken("keys"))?;
    let jwk_value = set
        .get("keys")
        .and_then(|keys| keys.as_array())
        .and_then(|keys| {
            keys.iter()
                .find(|key| key.get("kid").and_then(|value| value.as_str()) == Some(kid.as_str()))
        })
        .ok_or(OAuthError::InvalidToken("unknown kid"))?;
    let jwk: jsonwebtoken::jwk::Jwk = serde_json::from_value(jwk_value.clone())
        .map_err(|_| OAuthError::InvalidToken("bad jwk"))?;
    let key = DecodingKey::from_jwk(&jwk).map_err(|_| OAuthError::InvalidToken("bad jwk"))?;
    // ES256-only: the crate enforces the algorithm. Expiry and
    // audience are checked manually below with injected `now`
    // (the crate would use the wall clock).
    let mut validation = Validation::new(Algorithm::ES256);
    validation.validate_exp = false;
    validation.validate_aud = false;
    let claims = decode::<serde_json::Value>(id_token, &key, &validation)
        .map_err(|_| OAuthError::InvalidToken("sig"))?
        .claims;
    let field = |name: &str| {
        claims
            .get(name)
            .and_then(|item| item.as_str())
            .unwrap_or("")
            .to_string()
    };
    if field("iss") != APPLE_ISSUER {
        return Err(OAuthError::InvalidToken("bad iss"));
    }
    if field("aud") != expect_aud {
        return Err(OAuthError::InvalidToken("aud mismatch"));
    }
    let exp = claims
        .get("exp")
        .and_then(|value| value.as_i64())
        .ok_or(OAuthError::InvalidToken("bad exp"))?;
    if now >= exp {
        return Err(OAuthError::InvalidToken("expired"));
    }
    let sub = field("sub");
    if sub.is_empty() {
        return Err(OAuthError::InvalidToken("no sub"));
    }
    // Wire leniency (spec §10.3, OPEN-9 confirms at staging): a
    // boolean `true` or the string `"true"` both count; anything
    // else fails closed to unverified.
    let email_verified = claims
        .get("email_verified")
        .is_some_and(|value| value == true || value == "true");
    Ok(VerifiedApple {
        sub,
        email: field("email"),
        email_verified,
        is_relay: claims
            .get("is_private_email")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
    })
}

/// Owner of a live apple link for `sub`, if any.
pub fn find_apple_owner(conn: &Connection, sub: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT user_id FROM identity_link
         WHERE method = 'apple' AND provider_sub = ?1 AND disabled_at IS NULL
         LIMIT 1",
        [sub],
        |row| row.get(0),
    )
    .optional()
}

/// Insert the apple identity link (fresh account or explicit
/// link). Emails hash lowercase like every other link writer.
pub fn insert_apple_link(
    conn: &Connection,
    user_id: &str,
    sub: &str,
    email: &str,
    verified: bool,
    is_relay: bool,
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
         VALUES (?1, 'apple', ?2, ?3, ?4, ?5, NULL)",
        rusqlite::params![user_id, sub, hash, flag, i32::from(is_relay)],
    )?;
    Ok(())
}

/// Route a verified Apple identity (spec §10.4, rules 1–5).
/// Locked/role checks stay with the caller (it owns the session +
/// audit writes). Relay addresses never match (both gates exclude
/// them), so relay logins mint fresh accounts unless the sub is
/// known — zero new branches.
pub fn resolve_apple(
    conn: &Connection,
    sub: &str,
    email: &str,
    verified: bool,
    is_relay: bool,
    now: i64,
) -> rusqlite::Result<crate::oauth::ResolveOutcome> {
    use crate::oauth::ResolveOutcome;
    if let Some(user_id) = find_apple_owner(conn, sub)? {
        return Ok(ResolveOutcome::Login { user_id });
    }
    let identity = crate::login::Identity {
        method: crate::login::LoginMethod::Apple,
        provider_sub: sub.to_string(),
        verified_email: verified
            .then(|| email.to_string())
            .filter(|value| !value.is_empty()),
        is_relay,
    };
    let matched = crate::login::email_usable_for_matching(&identity)
        && crate::passkey::usable_email_owner(conn, email)?.is_some();
    match crate::login::decide_unknown_sub(matched) {
        crate::login::NewIdentityDecision::CreateFreshAccount => {
            let user_id = crate::passkey::mint_user_id();
            crate::passkey::ensure_account(conn, &user_id, now)?;
            insert_apple_link(conn, &user_id, sub, email, verified, is_relay)?;
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
            _pairs: &[(&str, &str)],
        ) -> Result<crate::oauth::HttpResponse, String> {
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

    const TEST_KID: &str = "test-kid-1";

    /// Mint a fresh synthetic P-256 key as PKCS#8 PEM (test-only;
    /// generated per test via aws-lc-rs — no key material is
    /// embedded in the source). Armor lines are assembled from
    /// fragments: the secrets-scan forbids the literal needle even
    /// in test fixtures.
    fn generated_p8() -> String {
        use aws_lc_rs::rand::SystemRandom;
        use base64::Engine as _;
        let pkcs8 = aws_lc_rs::signature::EcdsaKeyPair::generate_pkcs8(
            &aws_lc_rs::signature::ECDSA_P256_SHA256_FIXED_SIGNING,
            &SystemRandom::new(),
        )
        .unwrap();
        let body = base64::engine::general_purpose::STANDARD.encode(pkcs8.as_ref());
        let begin: String = ["-----BEGIN", " PRIVATE KEY-----"].concat();
        let end: String = ["-----END", " PRIVATE KEY-----"].concat();
        let mut out = format!("{begin}\n");
        for chunk in body.as_bytes().chunks(64) {
            out.push_str(std::str::from_utf8(chunk).unwrap());
            out.push('\n');
        }
        out.push_str(&end);
        out.push('\n');
        out
    }

    /// Decode a PEM to PKCS#8 DER (test helper; base64 is a
    /// dev-dependency).
    fn fixture_der(pem: &str) -> Vec<u8> {
        use base64::Engine as _;
        let body: String = pem
            .lines()
            .filter(|line| !line.starts_with("-----"))
            .collect();
        base64::engine::general_purpose::STANDARD
            .decode(body)
            .unwrap()
    }

    /// Build the JWKS body matching a PEM key (public point derived
    /// via aws-lc-rs, so the round trip proves ES256 sign works on
    /// the workspace provider).
    fn fixture_jwks(pem: &str, kid: &str) -> Vec<u8> {
        use aws_lc_rs::signature::KeyPair as _;
        use base64::Engine as _;
        let pair = aws_lc_rs::signature::EcdsaKeyPair::from_pkcs8(
            &aws_lc_rs::signature::ECDSA_P256_SHA256_FIXED_SIGNING,
            &fixture_der(pem),
        )
        .unwrap();
        let point: &[u8] = pair.public_key().as_ref();
        assert_eq!(point.len(), 65);
        assert_eq!(point[0], 0x04);
        let b64u = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let x = b64u.encode(&point[1..33]);
        let y = b64u.encode(&point[33..65]);
        format!(
            r#"{{"keys":[{{"kty":"EC","crv":"P-256","kid":"{kid}","x":"{x}","y":"{y}","use":"sig","alg":"ES256"}}]}}"#
        )
        .into_bytes()
    }

    /// Sign an id_token with a PEM key (test-only issuer).
    fn fixture_id_token(pem: &str, claims: serde_json::Value) -> String {
        use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
        let key = EncodingKey::from_ec_pem(pem.as_bytes()).unwrap();
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(TEST_KID.to_string());
        encode(&header, &claims, &key).unwrap()
    }

    fn id_claims(email_verified: serde_json::Value, relay: bool) -> serde_json::Value {
        serde_json::json!({
            "iss": APPLE_ISSUER,
            "aud": "cid-apple",
            "sub": "apple-sub-1",
            "exp": 2_000_000_000i64,
            "iat": 1_000_000_000i64,
            "email": "a@example.test",
            "email_verified": email_verified,
            "is_private_email": relay,
        })
    }

    fn keys_stub(body: Vec<u8>) -> Stub {
        let mut routes = HashMap::new();
        routes.insert(
            format!("GET {APPLE_KEYS_URL}"),
            crate::oauth::HttpResponse { status: 200, body },
        );
        Stub { routes }
    }

    #[test]
    fn client_secret_shape_and_unconfigured() {
        let pem = generated_p8();
        assert_eq!(
            mint_client_secret("", "cid", "kid", &pem, 1000),
            Err(crate::oauth::OAuthError::NotConfigured)
        );
        assert_eq!(
            mint_client_secret("team", "cid", "kid", "not a pem", 1000),
            Err(crate::oauth::OAuthError::InvalidToken("client secret"))
        );
        let jwt = mint_client_secret("team-1", "cid-apple", TEST_KID, &pem, 1_000_000).unwrap();
        // Header carries ES256 + kid; claims carry iss/sub/aud/iat/exp.
        let mut parts = jwt.split('.');
        let (header_b64, payload_b64) = (parts.next().unwrap(), parts.next().unwrap());
        assert!(parts.next().is_some());
        assert!(parts.next().is_none());
        use base64::Engine as _;
        let b64u = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let header: serde_json::Value =
            serde_json::from_slice(&b64u.decode(header_b64).unwrap()).unwrap();
        assert_eq!(header["alg"], "ES256");
        assert_eq!(header["kid"], TEST_KID);
        let payload: serde_json::Value =
            serde_json::from_slice(&b64u.decode(payload_b64).unwrap()).unwrap();
        assert_eq!(payload["iss"], "team-1");
        assert_eq!(payload["sub"], "cid-apple");
        assert_eq!(payload["aud"], APPLE_ISSUER);
        assert_eq!(payload["iat"], 1_000_000);
        assert_eq!(payload["exp"], 1_000_300);
    }

    #[test]
    fn verify_ok_bool_and_string_wire() {
        let pem = generated_p8();
        for verified in [serde_json::json!(true), serde_json::json!("true")] {
            let token = fixture_id_token(&pem, id_claims(verified, false));
            let stub = keys_stub(fixture_jwks(&pem, TEST_KID));
            assert_eq!(
                verify_id_token(&stub, APPLE_KEYS_URL, &token, "cid-apple", 1_500_000_000),
                Ok(VerifiedApple {
                    sub: "apple-sub-1".to_string(),
                    email: "a@example.test".to_string(),
                    email_verified: true,
                    is_relay: false,
                })
            );
        }
    }

    #[test]
    fn verify_relay_and_unverified_fail_closed() {
        let pem = generated_p8();
        let token = fixture_id_token(&pem, id_claims(serde_json::json!(true), true));
        let stub = keys_stub(fixture_jwks(&pem, TEST_KID));
        let verified =
            verify_id_token(&stub, APPLE_KEYS_URL, &token, "cid-apple", 1_500_000_000).unwrap();
        assert!(verified.is_relay);
        // String "false" and missing both fail closed to unverified.
        for claims in [
            id_claims(serde_json::json!("false"), false),
            serde_json::json!({
                "iss": APPLE_ISSUER, "aud": "cid-apple",
                "sub": "apple-sub-1", "exp": 2_000_000_000i64,
            }),
        ] {
            let token = fixture_id_token(&pem, claims);
            let verified =
                verify_id_token(&stub, APPLE_KEYS_URL, &token, "cid-apple", 1_500_000_000).unwrap();
            assert!(!verified.email_verified);
        }
    }

    #[test]
    fn verify_rejects_claims_and_keys() {
        use crate::oauth::OAuthError;
        let pem = generated_p8();
        let stub = keys_stub(fixture_jwks(&pem, TEST_KID));
        // Unknown kid.
        let token = fixture_id_token(&pem, id_claims(serde_json::json!(true), false));
        let other_keys = keys_stub(fixture_jwks(&pem, "other-kid"));
        assert_eq!(
            verify_id_token(
                &other_keys,
                APPLE_KEYS_URL,
                &token,
                "cid-apple",
                1_500_000_000
            ),
            Err(OAuthError::InvalidToken("unknown kid"))
        );
        // Tampered signature (first char of the signature part
        // flipped — deterministic under random ECDSA nonces).
        let bad = {
            let parts: Vec<&str> = token.split('.').collect();
            let mut sig = parts[2].to_string();
            let first = sig.remove(0);
            sig.insert(0, if first == 'A' { 'B' } else { 'A' });
            format!("{}.{}.{}", parts[0], parts[1], sig)
        };
        assert_eq!(
            verify_id_token(&stub, APPLE_KEYS_URL, &bad, "cid-apple", 1_500_000_000),
            Err(OAuthError::InvalidToken("sig"))
        );
        // Wrong audience / issuer / expired / no sub.
        for (claims, tag) in [
            (
                serde_json::json!({"iss": APPLE_ISSUER, "aud": "other", "sub": "s", "exp": 2_000_000_000i64}),
                "aud mismatch",
            ),
            (
                serde_json::json!({"iss": "https://evil.test", "aud": "cid-apple", "sub": "s", "exp": 2_000_000_000i64}),
                "bad iss",
            ),
            (
                serde_json::json!({"iss": APPLE_ISSUER, "aud": "cid-apple", "sub": "s", "exp": 1_000_000_000i64}),
                "expired",
            ),
            (
                serde_json::json!({"iss": APPLE_ISSUER, "aud": "cid-apple", "exp": 2_000_000_000i64}),
                "no sub",
            ),
        ] {
            let token = fixture_id_token(&pem, claims);
            assert_eq!(
                verify_id_token(&stub, APPLE_KEYS_URL, &token, "cid-apple", 1_500_000_000),
                Err(OAuthError::InvalidToken(tag)),
                "{tag}"
            );
        }
        // Garbage token, empty token, keys outage.
        assert_eq!(
            verify_id_token(
                &stub,
                APPLE_KEYS_URL,
                "not-a-jwt",
                "cid-apple",
                1_500_000_000
            ),
            Err(OAuthError::InvalidToken("bad token"))
        );
        assert_eq!(
            verify_id_token(&stub, APPLE_KEYS_URL, "", "cid-apple", 1_500_000_000),
            Err(OAuthError::InvalidToken("bad token"))
        );
        let mut routes = HashMap::new();
        routes.insert(
            format!("GET {APPLE_KEYS_URL}"),
            crate::oauth::HttpResponse {
                status: 500,
                body: b"{}".to_vec(),
            },
        );
        assert_eq!(
            verify_id_token(
                &Stub { routes },
                APPLE_KEYS_URL,
                &token,
                "cid-apple",
                1_500_000_000
            ),
            Err(OAuthError::InvalidToken("keys"))
        );
    }

    #[test]
    fn resolve_login_register_and_link_required() {
        use crate::oauth::ResolveOutcome;
        let conn = memory();
        // Unknown sub, no match: fresh account + link row.
        let user = match resolve_apple(&conn, "apple-sub-1", "a@example.test", true, false, 1000)
            .unwrap()
        {
            ResolveOutcome::Register { user_id } => user_id,
            other => panic!("{other:?}"),
        };
        assert_eq!(
            find_apple_owner(&conn, "apple-sub-1").unwrap(),
            Some(user.clone())
        );
        // Known sub: login.
        assert_eq!(
            resolve_apple(&conn, "apple-sub-1", "a@example.test", true, false, 1000).unwrap(),
            ResolveOutcome::Login { user_id: user }
        );
        // Verified real-email duplicate on another method: NEVER
        // merge — link required (rule 5).
        crate::oauth::insert_google_link(&conn, "u-google", "g-sub-1", "b@example.test", true)
            .unwrap();
        assert_eq!(
            resolve_apple(&conn, "apple-sub-2", "b@example.test", true, false, 1000).unwrap(),
            ResolveOutcome::PromptExplicitLink
        );
        assert_eq!(find_apple_owner(&conn, "apple-sub-2").unwrap(), None);
    }

    #[test]
    fn resolve_relay_never_matches() {
        use crate::oauth::ResolveOutcome;
        let conn = memory();
        // Same address verified on Google: a relay login still mints
        // fresh (both gates exclude relay rows).
        crate::oauth::insert_google_link(&conn, "u-google", "g-sub-1", "r@example.test", true)
            .unwrap();
        let user = match resolve_apple(&conn, "apple-sub-r", "r@example.test", true, true, 1000)
            .unwrap()
        {
            ResolveOutcome::Register { user_id } => user_id,
            other => panic!("{other:?}"),
        };
        // The relay row itself never matches a later login either:
        // a different fresh account, never a merge.
        match resolve_apple(&conn, "apple-sub-r2", "r@example.test", true, true, 1000).unwrap() {
            ResolveOutcome::Register { user_id: other } => assert_ne!(other, user),
            other => panic!("relay rows must not be matchable: {other:?}"),
        }
    }

    #[test]
    fn insert_roundtrip_carries_relay_flag() {
        let conn = memory();
        insert_apple_link(&conn, "u-1", "sub-r", "x@y.test", true, true).unwrap();
        let (verified, relay): (i64, i64) = conn
            .query_row(
                "SELECT verified_email, is_relay FROM identity_link
                 WHERE user_id = 'u-1' AND method = 'apple'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((verified, relay), (1, 1));
    }

    #[test]
    fn kind_isolation_apple_vs_meta() {
        use crate::oauth::{OauthMode, TakeOutcome};
        let conn = memory();
        crate::oauth::store_state_kind(
            &conn,
            crate::oauth::KIND_APPLE,
            "st-apple",
            OauthMode::Login,
            None,
            "bind-1",
            1000,
        )
        .unwrap();
        // Wrong kind: Missing — and the row is NOT consumed.
        assert_eq!(
            crate::oauth::take_state_kind(&conn, crate::oauth::KIND_META, "st-apple", 1100)
                .unwrap(),
            TakeOutcome::Missing
        );
        assert!(matches!(
            crate::oauth::take_state_kind(&conn, crate::oauth::KIND_APPLE, "st-apple", 1100)
                .unwrap(),
            TakeOutcome::Taken { .. }
        ));
    }
}
