//! Google RISC receiver crypto (spec `customer-auth.md` §9.9,
//! OPEN-7): Security Event Token (SET) verification for the
//! `POST /auth/google/risc` push endpoint.
//!
//! Mirrors [`crate::apple::verify_id_token`] but RS256: select the
//! JWK by the JWT header `kid`, verify the signature, then check
//! claims manually with injected `now`. HTTP goes through
//! [`crate::oauth::HttpTransport`] so tests run offline.
//!
//! No replay table: every RISC action is idempotent (disable /
//! session-kill / lock re-apply harmlessly), so a replayed SET
//! re-applies and still 202s.

use crate::oauth::{HttpTransport, OAuthError};

/// Google's RISC signing-key set (overridden only by tests).
pub const RISC_CERTS_URL: &str = "https://www.googleapis.com/oauth2/v3/certs";

/// SET issuer: documented with a trailing slash, tokeninfo-style
/// without. Both are accepted (R1); anything else fails.
pub const RISC_ISSUER_SLASH: &str = "https://accounts.google.com/";
pub const RISC_ISSUER_BARE: &str = "https://accounts.google.com";

/// Max `|now - iat|` skew for an accepted SET (5 min).
pub const RISC_SKEW_SECS: i64 = 300;

const EVENT_SESSIONS_REVOKED: &str =
    "https://schemas.openid.net/secevent/risc/event-type/sessions-revoked";
const EVENT_CREDENTIAL_CHANGE: &str =
    "https://schemas.openid.net/secevent/risc/event-type/credential-change-required";
const EVENT_ACCOUNT_DISABLED: &str =
    "https://schemas.openid.net/secevent/risc/event-type/account-disabled";
const EVENT_ACCOUNT_ENABLED: &str =
    "https://schemas.openid.net/secevent/risc/event-type/account-enabled";
const EVENT_VERIFICATION: &str = "https://schemas.openid.net/secevent/risc/event-type/verification";

/// One parsed RISC event type. Unknown URIs stay `Unknown` (the
/// handler 202s + audits them — forward-compat, never an error).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RiscEvent {
    SessionsRevoked,
    CredentialChangeRequired,
    AccountDisabled,
    AccountEnabled,
    Verification,
    Unknown(String),
}

/// A verified SET: what happened, to whom. `sub` is set only for
/// `iss_sub` subjects (actionable); `email_subject` marks an
/// email-format subject (audit-only — acting on an unverified
/// email match is a takeover vector).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedRisc {
    pub event: RiscEvent,
    pub sub: Option<String>,
    pub email_subject: bool,
    pub state: Option<String>,
}

/// Map a full event-type URI to its variant.
pub fn parse_risc_event(uri: &str) -> RiscEvent {
    match uri {
        EVENT_SESSIONS_REVOKED => RiscEvent::SessionsRevoked,
        EVENT_CREDENTIAL_CHANGE => RiscEvent::CredentialChangeRequired,
        EVENT_ACCOUNT_DISABLED => RiscEvent::AccountDisabled,
        EVENT_ACCOUNT_ENABLED => RiscEvent::AccountEnabled,
        EVENT_VERIFICATION => RiscEvent::Verification,
        other => RiscEvent::Unknown(other.to_string()),
    }
}

/// Check SET claims with injected `now` (mirrors the Apple manual
/// checks — the crate would use the wall clock). Tags: `bad iss`,
/// `aud mismatch`, `bad iat`, `stale`, `exp`.
pub fn check_risc_claims(
    claims: &serde_json::Value,
    expect_aud: &str,
    now: i64,
) -> Result<(), OAuthError> {
    let iss = claims
        .get("iss")
        .and_then(|item| item.as_str())
        .unwrap_or("");
    if iss != RISC_ISSUER_SLASH && iss != RISC_ISSUER_BARE {
        return Err(OAuthError::InvalidToken("bad iss"));
    }
    let aud_ok = match claims.get("aud") {
        Some(serde_json::Value::String(aud)) => aud == expect_aud,
        Some(serde_json::Value::Array(list)) => {
            list.len() == 1 && list[0].as_str() == Some(expect_aud)
        }
        _ => false,
    };
    if !aud_ok {
        return Err(OAuthError::InvalidToken("aud mismatch"));
    }
    let Some(iat) = claims.get("iat").and_then(|item| item.as_i64()) else {
        return Err(OAuthError::InvalidToken("bad iat"));
    };
    if now.saturating_sub(iat).abs() > RISC_SKEW_SECS {
        return Err(OAuthError::InvalidToken("stale"));
    }
    if let Some(exp) = claims.get("exp").and_then(|item| item.as_i64())
        && exp <= now
    {
        return Err(OAuthError::InvalidToken("exp"));
    }
    Ok(())
}

/// Parse the `events` object (exactly-one expected; multi-key takes
/// the first — deterministic either way) and the `subject`.
/// Returns `(event, sub, email_subject, state)`.
fn parse_events_subject(
    claims: &serde_json::Value,
) -> Result<(RiscEvent, Option<String>, bool, Option<String>), OAuthError> {
    let events = claims
        .get("events")
        .and_then(|item| item.as_object())
        .filter(|map| !map.is_empty())
        .ok_or(OAuthError::InvalidToken("no events"))?;
    let (uri, body) = events.iter().next().expect("non-empty map has a first");
    let event = parse_risc_event(uri);
    let state = (event == RiscEvent::Verification)
        .then(|| {
            body.get("state")
                .and_then(|item| item.as_str())
                .map(str::to_string)
        })
        .flatten();
    let subject = claims
        .get("subject")
        .and_then(|item| item.as_object())
        .ok_or(OAuthError::InvalidToken("bad subject"))?;
    match subject.get("format").and_then(|item| item.as_str()) {
        Some("iss_sub") => {
            let sub = subject
                .get("sub")
                .and_then(|item| item.as_str())
                .filter(|sub| !sub.is_empty())
                .ok_or(OAuthError::InvalidToken("bad subject"))?;
            Ok((event, Some(sub.to_string()), false, state))
        }
        // Email-format subjects verify fine but are never acted on
        // (R4) — the handler audits and 202s.
        Some("email") => Ok((event, None, true, state)),
        // Unknown formats are audit-only too (forward-compat):
        // a 400 here could get a valid stream disabled.
        Some(_) => Ok((event, None, false, state)),
        None => Err(OAuthError::InvalidToken("bad subject")),
    }
}

/// Verify one pushed SET: `kid` → JWKS → RS256 → claims →
/// events/subject. Tags: `bad token`, `no kid`, `keys`,
/// `unknown kid`, `bad jwk`, `sig`, plus the claim tags.
pub fn verify_set<T: HttpTransport>(
    transport: &T,
    certs_url: &str,
    set: &str,
    expect_aud: &str,
    now: i64,
) -> Result<VerifiedRisc, OAuthError> {
    use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
    let set = set.trim();
    if set.is_empty() || set.split('.').count() != 3 {
        return Err(OAuthError::InvalidToken("bad token"));
    }
    let kid = decode_header(set)
        .map_err(|_| OAuthError::InvalidToken("bad token"))?
        .kid
        .filter(|kid| !kid.is_empty())
        .ok_or(OAuthError::InvalidToken("no kid"))?;
    let response = transport.get(certs_url).map_err(OAuthError::Transport)?;
    if response.status != 200 {
        return Err(OAuthError::InvalidToken("keys"));
    }
    let keys: serde_json::Value =
        serde_json::from_slice(&response.body).map_err(|_| OAuthError::InvalidToken("keys"))?;
    let jwk_value = keys
        .get("keys")
        .and_then(|list| list.as_array())
        .and_then(|list| {
            list.iter()
                .find(|key| key.get("kid").and_then(|value| value.as_str()) == Some(kid.as_str()))
        })
        .ok_or(OAuthError::InvalidToken("unknown kid"))?;
    let jwk: jsonwebtoken::jwk::Jwk = serde_json::from_value(jwk_value.clone())
        .map_err(|_| OAuthError::InvalidToken("bad jwk"))?;
    let key = DecodingKey::from_jwk(&jwk).map_err(|_| OAuthError::InvalidToken("bad jwk"))?;
    // RS256-only: the crate enforces the algorithm (no
    // alg-confusion class). Expiry and audience are checked
    // manually below with injected `now`.
    let mut validation = Validation::new(Algorithm::RS256);
    validation.validate_exp = false;
    validation.validate_aud = false;
    // `exp` is optional on SETs (checked manually when present),
    // so the crate's required-claims set must go too.
    validation.required_spec_claims.clear();
    let claims = decode::<serde_json::Value>(set, &key, &validation)
        .map_err(|_| OAuthError::InvalidToken("sig"))?
        .claims;
    check_risc_claims(&claims, expect_aud, now)?;
    let (event, sub, email_subject, state) = parse_events_subject(&claims)?;
    Ok(VerifiedRisc {
        event,
        sub,
        email_subject,
        state,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic RSA-2048 test vector (generated locally, test-only,
    /// never production): base64url PKCS#1 components. Stored as JWK
    /// JSON field values rather than PEM armor per the secrets-scan
    /// rule (aws-lc-rs exposes no RSA public-key export, so the
    /// P-256 generate-per-test trick cannot build the stub JWKS).
    const TEST_RSA_N: &str = "2elao6eOjvvb9zPc1Eb22YC5lIAC8bCH70k-YTZUoeH-1fhkBIMlzUxE9hSl2HWDAw7u1PL2hXVl3afWPkHFb5TWVdREcsoI2fi94VakouBufNYXBCsat_9r1iCE9O9ZQhxaXPUYPGpLNk9TFyGETmVqizegSOahHtyHR4uhq5PcVzkXukK8TLwXvgcKENZ4JnmNAyEhW_u3cUkQSCagnzwV-3FaV5391gxJvfHPGvtQrdsZmVqKuSFdl7HN7s5c-t9YGbNpIgi6SIskA8AbRT36qYtZJFqTzrAt2bULB5eZHwO_tleeX9er4vUwWjyAs4TzKW1glKvkChrHbH7MFQ";
    const TEST_RSA_E: &str = "AQAB";
    const TEST_RSA_D: &str = "FZZxFMpqLI8V7Qj1xQ_cr5J5kBUYdZIb_WAfrmi__KOTZ-YqHmpRqeN253wHLAkEOB7YGT-SnYkSVOGmL1e7tACmNS2ItGiAdPuX8AVx3PLn7zUUZaTRik3jBH0HPu6hRJPjfdcIFiAfoFs-a4m1_kA8zFrv5Zj3eEO3oSZbl1ApU8WWgrl2C1xK8NNUUXj5fcMJvR7WXBQdvldP5w9JG0sP1UTXwLI_uvd18pOKQXGLPJGBRO1iLOLXvwsadyIy5-Ft9SYxbHEbvorIhdClSR7KrtDF2i2qf5Fh7vlI0YUo2h2fc9uHhErJpW-Z-NU6BAAu2vchaz7wr02rUHPHxw";
    const TEST_RSA_P: &str = "8AZmTr1aZumIUQD1k_-h11DNaQNDDSzYaWf8WkV2CSeEMkztwYT0D1l_HulcIWNFS-hAnxvLmkrhDWXh5j_6TzxtYJyFj_tXirb7cI9HEBVWKFLMbVtAsEsxBeaKjqvXdQiCPJ142h0FQdZUI7CQMF2Esd_zhurA8a3A-QkOzbs";
    const TEST_RSA_Q: &str = "6GouJzu4iJ0_8sb-c0F56Ix4Id0vitAcvym6jbzaBrJnRQB7eTfXZbwBtNWXNAl1IRkWUK1wsTasfAucO0PyUJMoTEfD23PV_IpEAyi4j8e1W93p-NvKT9Gm4cAOUjce752D_lY0Vb7WXBUOsELKx5BKY6oKdvmcj_kXXKC1SG8";
    const TEST_RSA_DP: &str = "Kjw2DGztV2aIghh1HhHBXK3vNydHyS229vVBRnEqj_nQHTnELGVEvN62q63MdGOPcgfwNw8hCIlciIsA8Q8Y8JBlfraMGgXUJaiFHg1YJ9BZVMcdCbEg9OSGP8-NWgWK6CETKa9b8KVio28fZxhOhvQtwU7y_y6XvO8iIbAiVrk";
    const TEST_RSA_DQ: &str = "GRRcJp-rKhLN88TFcGqAXTfLPtw_ZRtP_tDh8KmQXWii_C8xI8OC2teO15S03Eio-wlZ_xq4LmdX9YrLMRoD6-31S0XzPJsjXD0PWcfrIpjKWT95IAGXs0VMXM_8u6J3FCRLOgbTXu1Y9FbRXvPWLfLEFD0lI_GLk32tp7Ev3sE";
    const TEST_RSA_QI: &str = "tAFrmAsQbcGIM_scizNNbmJaWy2B-KZFOrQV46vK8N9gEm4cxSb_ainHDMrLRGxrSWZjTkobDkAPyGnd9Zvqg0H5OnFtg2V19unjHnwJcETlnvwIeNWqGG64h5lk8mohNJS_vRo34mBSmYo3SydhalgOWcH3iTrZnboDktI401E";

    const TEST_KID: &str = "risc-test-kid";
    const TEST_AUD: &str = "test-google-client-id";

    fn b64u(bytes: &[u8]) -> String {
        use base64::Engine as _;
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    }

    fn b64u_decode(raw: &str) -> Vec<u8> {
        use base64::Engine as _;
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(raw)
            .unwrap()
    }

    /// Stub certs endpoint serving the test JWKS.
    struct StubCerts {
        body: Vec<u8>,
        status: u16,
    }

    impl HttpTransport for StubCerts {
        fn post_form(
            &self,
            _url: &str,
            _pairs: &[(&str, &str)],
        ) -> Result<crate::oauth::HttpResponse, String> {
            unreachable!("RISC never POSTs");
        }

        fn get(&self, _url: &str) -> Result<crate::oauth::HttpResponse, String> {
            Ok(crate::oauth::HttpResponse {
                status: self.status,
                body: self.body.clone(),
            })
        }

        fn get_query(
            &self,
            _base: &str,
            _pairs: &[(&str, &str)],
        ) -> Result<crate::oauth::HttpResponse, String> {
            unreachable!("RISC never GET-queries");
        }
    }

    fn stub_jwks() -> StubCerts {
        let body = serde_json::json!({
            "keys": [{
                "kty": "RSA",
                "kid": TEST_KID,
                "use": "sig",
                "alg": "RS256",
                "n": TEST_RSA_N,
                "e": TEST_RSA_E,
            }]
        });
        StubCerts {
            body: serde_json::to_vec(&body).unwrap(),
            status: 200,
        }
    }

    /// Sign a compact RS256 JWT with the test vector (hand-rolled:
    /// aws-lc-rs signs, we assemble `header.claims.sig`).
    fn sign_set(claims: &serde_json::Value, kid: &str) -> String {
        use aws_lc_rs::rsa::{KeyPair, KeyPairComponents, PublicKeyComponents};
        use aws_lc_rs::signature::RSA_PKCS1_SHA256;
        let components = KeyPairComponents {
            public_key: PublicKeyComponents {
                n: b64u_decode(TEST_RSA_N),
                e: b64u_decode(TEST_RSA_E),
            },
            d: b64u_decode(TEST_RSA_D),
            p: b64u_decode(TEST_RSA_P),
            q: b64u_decode(TEST_RSA_Q),
            dP: b64u_decode(TEST_RSA_DP),
            dQ: b64u_decode(TEST_RSA_DQ),
            qInv: b64u_decode(TEST_RSA_QI),
        };
        let key = KeyPair::from_components(&components).unwrap();
        let header = serde_json::json!({"alg": "RS256", "kid": kid, "typ": "secevent+jwt"});
        let signing_input = format!(
            "{}.{}",
            b64u(serde_json::to_vec(&header).unwrap().as_slice()),
            b64u(serde_json::to_vec(claims).unwrap().as_slice())
        );
        let rng = aws_lc_rs::rand::SystemRandom::new();
        let mut sig = vec![0u8; key.public_modulus_len()];
        key.sign(&RSA_PKCS1_SHA256, &rng, signing_input.as_bytes(), &mut sig)
            .unwrap();
        format!("{signing_input}.{}", b64u(&sig))
    }

    fn claims(event_uri: &str, now: i64) -> serde_json::Value {
        serde_json::json!({
            "iss": RISC_ISSUER_SLASH,
            "aud": TEST_AUD,
            "iat": now,
            "jti": "evt-1",
            "events": { event_uri: {} },
            "subject": {"format": "iss_sub", "iss": RISC_ISSUER_SLASH, "sub": "google-sub-1"},
        })
    }

    #[test]
    fn valid_set_verifies() {
        let now = 1_800_000_000;
        let set = sign_set(&claims(EVENT_SESSIONS_REVOKED, now), TEST_KID);
        let verified = verify_set(&stub_jwks(), "https://certs.invalid", &set, TEST_AUD, now)
            .expect("valid SET verifies");
        assert_eq!(verified.event, RiscEvent::SessionsRevoked);
        assert_eq!(verified.sub.as_deref(), Some("google-sub-1"));
        assert!(!verified.email_subject);
        assert_eq!(verified.state, None);
    }

    #[test]
    fn bare_issuer_accepted() {
        let now = 1_800_000_000;
        let mut body = claims(EVENT_ACCOUNT_ENABLED, now);
        body["iss"] = serde_json::json!(RISC_ISSUER_BARE);
        let set = sign_set(&body, TEST_KID);
        let verified = verify_set(&stub_jwks(), "https://certs.invalid", &set, TEST_AUD, now)
            .expect("bare iss verifies");
        assert_eq!(verified.event, RiscEvent::AccountEnabled);
    }

    #[test]
    fn wrong_issuer_rejected() {
        let now = 1_800_000_000;
        let mut body = claims(EVENT_ACCOUNT_ENABLED, now);
        body["iss"] = serde_json::json!("https://evil.example/");
        let set = sign_set(&body, TEST_KID);
        assert!(matches!(
            verify_set(&stub_jwks(), "https://certs.invalid", &set, TEST_AUD, now),
            Err(OAuthError::InvalidToken("bad iss"))
        ));
    }

    #[test]
    fn tampered_signature_rejected() {
        let now = 1_800_000_000;
        let set = sign_set(&claims(EVENT_SESSIONS_REVOKED, now), TEST_KID);
        let mut tampered = set.clone();
        tampered.pop();
        tampered.push(if set.ends_with('A') { 'B' } else { 'A' });
        assert!(matches!(
            verify_set(
                &stub_jwks(),
                "https://certs.invalid",
                &tampered,
                TEST_AUD,
                now
            ),
            Err(OAuthError::InvalidToken("sig"))
        ));
    }

    #[test]
    fn unknown_kid_rejected() {
        let now = 1_800_000_000;
        let set = sign_set(&claims(EVENT_SESSIONS_REVOKED, now), "other-kid");
        assert!(matches!(
            verify_set(&stub_jwks(), "https://certs.invalid", &set, TEST_AUD, now),
            Err(OAuthError::InvalidToken("unknown kid"))
        ));
    }

    #[test]
    fn garbage_body_rejected() {
        let now = 1_800_000_000;
        for bad in ["", "   ", "not-a-jwt", "a.b", "a.b.c.d"] {
            assert!(
                matches!(
                    verify_set(&stub_jwks(), "https://certs.invalid", bad, TEST_AUD, now),
                    Err(OAuthError::InvalidToken("bad token"))
                ),
                "body {bad:?} is malformed"
            );
        }
    }

    #[test]
    fn claim_checks() {
        let now = 1_800_000_000;
        // Array aud with one entry verifies.
        let mut body = claims(EVENT_VERIFICATION, now);
        body["aud"] = serde_json::json!([TEST_AUD]);
        body["events"] = serde_json::json!({ EVENT_VERIFICATION: {"state": "abc"} });
        assert!(check_risc_claims(&body, TEST_AUD, now).is_ok());
        // Wrong aud (string + array).
        body["aud"] = serde_json::json!("other");
        assert!(matches!(
            check_risc_claims(&body, TEST_AUD, now),
            Err(OAuthError::InvalidToken("aud mismatch"))
        ));
        body["aud"] = serde_json::json!([TEST_AUD, "other"]);
        assert!(matches!(
            check_risc_claims(&body, TEST_AUD, now),
            Err(OAuthError::InvalidToken("aud mismatch"))
        ));
        body["aud"] = serde_json::json!(TEST_AUD);
        // Stale iat both directions; boundary passes.
        body["iat"] = serde_json::json!(now - RISC_SKEW_SECS - 1);
        assert!(matches!(
            check_risc_claims(&body, TEST_AUD, now),
            Err(OAuthError::InvalidToken("stale"))
        ));
        body["iat"] = serde_json::json!(now + RISC_SKEW_SECS + 1);
        assert!(matches!(
            check_risc_claims(&body, TEST_AUD, now),
            Err(OAuthError::InvalidToken("stale"))
        ));
        body["iat"] = serde_json::json!(now - RISC_SKEW_SECS);
        assert!(check_risc_claims(&body, TEST_AUD, now).is_ok());
        // Missing iat, expired exp.
        body.as_object_mut().unwrap().remove("iat");
        assert!(matches!(
            check_risc_claims(&body, TEST_AUD, now),
            Err(OAuthError::InvalidToken("bad iat"))
        ));
        body["iat"] = serde_json::json!(now);
        body["exp"] = serde_json::json!(now);
        assert!(matches!(
            check_risc_claims(&body, TEST_AUD, now),
            Err(OAuthError::InvalidToken("exp"))
        ));
        body["exp"] = serde_json::json!(now + 60);
        assert!(check_risc_claims(&body, TEST_AUD, now).is_ok());
    }

    #[test]
    fn events_and_subjects() {
        let now = 1_800_000_000;
        // Unknown event URI parses (forward-compat).
        let mut body = claims("https://example.com/future-event", now);
        let (event, sub, email, _) = parse_events_subject(&body).unwrap();
        assert!(matches!(event, RiscEvent::Unknown(_)));
        assert_eq!(sub.as_deref(), Some("google-sub-1"));
        assert!(!email);
        // Email-format subject flags non-actionable.
        body["subject"] = serde_json::json!({"format": "email", "email": "u@example.com"});
        let (_, sub, email, _) = parse_events_subject(&body).unwrap();
        assert_eq!(sub, None);
        assert!(email);
        // Missing events / missing subject are malformed.
        body.as_object_mut().unwrap().remove("events");
        assert!(matches!(
            parse_events_subject(&body),
            Err(OAuthError::InvalidToken("no events"))
        ));
        body = claims(EVENT_SESSIONS_REVOKED, now);
        body.as_object_mut().unwrap().remove("subject");
        assert!(matches!(
            parse_events_subject(&body),
            Err(OAuthError::InvalidToken("bad subject"))
        ));
        // Verification extracts state.
        body["subject"] = serde_json::json!({"format": "iss_sub", "sub": "s"});
        body["events"] = serde_json::json!({ EVENT_VERIFICATION: {"state": "echo-me"} });
        let (event, _, _, state) = parse_events_subject(&body).unwrap();
        assert_eq!(event, RiscEvent::Verification);
        assert_eq!(state.as_deref(), Some("echo-me"));
    }
}
