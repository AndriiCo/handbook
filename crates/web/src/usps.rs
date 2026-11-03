//! USPS APIs v3 in the web layer: credentials and caps from sealed
//! config, the OAuth and payment-authorization tokens, and the
//! [`LabelLink`] customer label orders buy through.
//!
//! - `usps.client_id` / `usps.client_secret`: the OAuth client.
//! - `usps.crid`, `usps.mid`, `usps.manifest_mid` (defaults to
//!   `usps.mid`) and `usps.epa_account`: the payment-authorization roles.
//! - `usps.live` (off): production postage at all. While it is off only
//!   the USPS test environment (or a loopback stub) is ever called, and a
//!   paid customer order can only be a test-mode order.
//! - `usps.live_daily_cap` (10) and `usps.live_max_usd` (40): live USPS
//!   labels per UTC day and per label, on top of the customer caps.

use handbook_store::{db, store::Store};
use labels::live::LiveConfig;
use labels::usps as v3;
use labels::{
    BuyOutcome, BuyRequest, HttpTransport, LabelClient, LabelError, UreqTransport, UspsClient,
};
use rusqlite::Connection;
use serde_json::Value;

use crate::AppState;
use crate::customer_labels::{By, LabelLink};

pub const DEFAULT_LIVE_DAILY_CAP: i64 = 10;
pub const DEFAULT_LIVE_MAX_CENTS: i64 = 4_000;

/// Everything a USPS call needs, read per call (secrets never cached).
pub(crate) struct UspsCreds {
    pub(crate) base: String,
    client_id: String,
    client_secret: String,
    crid: String,
    mid: String,
    manifest_mid: String,
    epa: String,
}

impl UspsCreds {
    fn account(&self) -> v3::PaymentAccount<'_> {
        v3::PaymentAccount {
            crid: &self.crid,
            mid: &self.mid,
            manifest_mid: &self.manifest_mid,
            epa_account: &self.epa,
        }
    }
}

/// The full credential set, or `None` while any piece is missing.
pub(crate) fn creds(state: &AppState, meta: &Connection) -> Option<UspsCreds> {
    let kek = state.kek.as_deref()?;
    let secret = |key| {
        config::get_secret_for_job(meta, kek, key)
            .ok()
            .flatten()
            .and_then(|b| String::from_utf8(b).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let text = |key| {
        state
            .config_cache
            .text(meta, kek, key)
            .ok()
            .flatten()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let mid = text(config::keys::USPS_MID)?;
    Some(UspsCreds {
        base: state
            .label_cfg
            .usps_base_url
            .trim_end_matches('/')
            .to_string(),
        client_id: secret(config::keys::USPS_CLIENT_ID)?,
        client_secret: secret(config::keys::USPS_CLIENT_SECRET)?,
        crid: text(config::keys::USPS_CRID)?,
        manifest_mid: text(config::keys::USPS_MANIFEST_MID).unwrap_or_else(|| mid.clone()),
        mid,
        epa: secret(config::keys::USPS_EPA_ACCOUNT)?,
    })
}

/// `usps.live` and its caps, through the 30 s config cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveCaps {
    pub live: bool,
    pub daily_cap: i64,
    pub max_cents: i64,
}

pub fn parse_live_caps(
    live: Option<bool>,
    daily_cap: Option<&str>,
    max_usd: Option<&str>,
) -> LiveCaps {
    LiveCaps {
        live: live.unwrap_or(false),
        daily_cap: daily_cap
            .and_then(|v| v.trim().parse::<i64>().ok())
            .filter(|n| (0..=1_000).contains(n))
            .unwrap_or(DEFAULT_LIVE_DAILY_CAP),
        max_cents: max_usd
            .and_then(labels::live::decimal_usd_to_cents)
            .filter(|c| (100..=1_000_000).contains(c))
            .unwrap_or(DEFAULT_LIVE_MAX_CENTS),
    }
}

pub fn live_caps(state: &AppState, meta: &Connection) -> LiveCaps {
    let Some(kek) = state.kek.as_deref() else {
        return parse_live_caps(None, None, None);
    };
    let cache = &state.config_cache;
    let text = |key| cache.text(meta, kek, key).ok().flatten();
    parse_live_caps(
        cache
            .bool(meta, kek, config::keys::USPS_LIVE)
            .ok()
            .flatten(),
        text(config::keys::USPS_LIVE_DAILY_CAP).as_deref(),
        text(config::keys::USPS_LIVE_MAX_USD).as_deref(),
    )
}

/// Test money only buys test postage; live money only production
/// postage, and only while `usps.live` is on.
pub fn mode_ok(base: &str, stripe_live: bool, live_on: bool) -> bool {
    if stripe_live {
        live_on && v3::is_production(base)
    } else {
        v3::is_test(base)
    }
}

/// USPS customer labels can be quoted and bought now.
pub fn customer_ready(state: &AppState, meta: &Connection, stripe_live: bool) -> bool {
    state.label_cfg.usps_certified
        && creds(state, meta)
            .is_some_and(|c| mode_ok(&c.base, stripe_live, live_caps(state, meta).live))
}

/// Live USPS label buys today (anything that may have bought).
pub(crate) fn live_buys_today(p2: &Connection, now: i64) -> i64 {
    p2.query_row(
        "SELECT COUNT(*) FROM label_purchase
         WHERE carrier = 'usps' AND livemode = 1 AND created_at >= ?1
           AND status IN ('in_flight', 'bought', 'unknown')",
        [crate::telemetry::utc_day_start(now)],
        |r| r.get(0),
    )
    .unwrap_or(i64::MAX)
}

/// OAuth client-credentials token (blocking). Secrets stay in scope;
/// errors carry statuses, never the request.
pub(crate) fn fetch_token(
    base: &str,
    client_id: &str,
    client_secret: &str,
) -> Result<String, String> {
    crate::oauth::ensure_provider();
    let config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let response = agent
        .post(&format!("{base}{}", v3::OAUTH_PATH))
        .header("Content-Type", "application/json")
        .send_json(v3::token_body(client_id, client_secret))
        .map_err(|e| format!("usps oauth: {e}"))?;
    let status = response.status().as_u16();
    let body = response
        .into_body()
        .read_to_vec()
        .map_err(|e| format!("usps oauth: {e}"))?;
    if !(200..300).contains(&status) {
        return Err(format!("usps oauth: http {status}"));
    }
    v3::parse_token(&body)
}

/// A payment-authorization token for our accounts (blocking).
pub(crate) fn payment_token(
    transport: &dyn HttpTransport,
    creds: &UspsCreds,
    bearer: &str,
) -> Result<String, String> {
    let url = format!("{}{}", creds.base, v3::PAYMENT_PATH);
    let response = transport
        .post_json(&url, bearer, None, &v3::payment_body(&creds.account()))
        .map_err(|e| format!("usps payment authorization: {e}"))?;
    if !(200..300).contains(&response.status) {
        return Err(format!(
            "usps payment authorization: http {}",
            response.status
        ));
    }
    v3::parse_payment_token(&response.body)
}

/// One order's USPS link: fresh bearer and payment token.
pub(crate) struct UspsLink<T: HttpTransport> {
    pub(crate) base: String,
    pub(crate) bearer: String,
    pub(crate) payment: String,
    pub(crate) epa: String,
    /// `usps.live_max_usd` in cents, for production postage only.
    pub(crate) live_max: Option<i64>,
    pub(crate) transport: T,
}

impl<T: HttpTransport> LabelLink for UspsLink<T> {
    fn carrier(&self) -> &'static str {
        "usps"
    }

    fn quote(&self, cfg: &LiveConfig, request: &BuyRequest) -> Result<i64, String> {
        cfg.require_enabled().map_err(|e| e.to_string())?;
        cfg.check_zip(&request.payload.to.postal)
            .map_err(|e| e.to_string())?;
        let url = format!("{}{}", self.base, v3::PRICES_PATH);
        let body = v3::prices_body(request.payload, &self.epa, crate::unix_now());
        let response = self
            .transport
            .post_json(&url, &self.bearer, None, &body)
            .map_err(|e| format!("usps prices: {e}"))?;
        if !(200..300).contains(&response.status) {
            return Err(format!("usps prices: http {}", response.status));
        }
        let json: Value = serde_json::from_slice(&response.body).map_err(|e| e.to_string())?;
        let cents = v3::parse_price_cents(&json).ok_or("usps prices: no usable rate")?;
        if self.live_max.is_some_and(|max| cents > max) {
            return Err(format!("usps quote {cents}c is over usps.live_max_usd"));
        }
        Ok(cents)
    }

    fn buy(&self, request: &BuyRequest) -> Result<BuyOutcome, LabelError> {
        UspsClient::new(&self.base, By(&self.transport), true).buy(&self.bearer, request)
    }

    fn cancel(&self, tracking_no: &str) -> Result<(), LabelError> {
        UspsClient::new(&self.base, By(&self.transport), true).cancel_paid(
            &self.bearer,
            &self.payment,
            tracking_no,
        )
    }

    fn payment_ref(&self) -> &str {
        &self.payment
    }
}

/// The USPS link for one customer order (blocking: two token calls).
/// `reserve`: buys that must still fit under `usps.live_daily_cap` (the
/// quote reserves both legs; the buyer checks each leg itself).
pub(crate) fn customer_link(
    state: &AppState,
    meta: &Connection,
    stripe_live: bool,
    reserve: i64,
) -> Result<Box<dyn LabelLink>, String> {
    if !state.label_cfg.usps_certified {
        return Err("usps api not certified yet (usps.certified)".to_string());
    }
    let creds = creds(state, meta).ok_or("usps credentials are not set")?;
    let caps = live_caps(state, meta);
    if !mode_ok(&creds.base, stripe_live, caps.live) {
        return Err("usps endpoint does not match the payment mode".to_string());
    }
    if stripe_live && reserve > 0 {
        let p2 = db::open_p2(&Store::new(&state.store_root)).map_err(|e| e.to_string())?;
        if live_buys_today(&p2, crate::unix_now()).saturating_add(reserve) > caps.daily_cap {
            return Err("usps live daily cap reached (usps.live_daily_cap)".to_string());
        }
    }
    let bearer = fetch_token(&creds.base, &creds.client_id, &creds.client_secret)?;
    let transport = UreqTransport::new();
    let payment = payment_token(&transport, &creds, &bearer)?;
    Ok(Box::new(UspsLink {
        base: creds.base.clone(),
        bearer,
        payment,
        epa: creds.epa.clone(),
        live_max: stripe_live.then_some(caps.max_cents),
        transport,
    }))
}

/// Tokens for the test-environment prepaid path (`p2_labels`): `None`
/// while credentials are unset (loopback stubs then take an empty
/// bearer), else a bearer and a payment token. Never for production.
pub(crate) fn test_tokens(state: &AppState) -> Option<Result<(String, String), String>> {
    let meta = db::open(&Store::new(&state.store_root)).ok()?;
    let creds = creds(state, &meta)?;
    if !v3::is_test(&creds.base) {
        return Some(Err(
            "usps tokens are minted for the test environment only".to_string()
        ));
    }
    Some((|| {
        let bearer = fetch_token(&creds.base, &creds.client_id, &creds.client_secret)?;
        let payment = payment_token(&UreqTransport::new(), &creds, &bearer)?;
        Ok((bearer, payment))
    })())
}

/// A bearer for read-only calls (tracking), or `None` without creds.
pub(crate) fn bearer(
    state: &AppState,
    meta: &Connection,
) -> Option<Result<(String, String), String>> {
    let creds = creds(state, meta)?;
    Some(fetch_token(&creds.base, &creds.client_id, &creds.client_secret).map(|t| (creds.base, t)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_caps_parse_with_safe_defaults() {
        assert_eq!(
            parse_live_caps(None, None, None),
            LiveCaps {
                live: false,
                daily_cap: DEFAULT_LIVE_DAILY_CAP,
                max_cents: DEFAULT_LIVE_MAX_CENTS
            }
        );
        let caps = parse_live_caps(Some(true), Some("3"), Some("25"));
        assert_eq!(
            (caps.live, caps.daily_cap, caps.max_cents),
            (true, 3, 2_500)
        );
        let caps = parse_live_caps(Some(true), Some("ten"), Some("-5"));
        assert_eq!(
            (caps.daily_cap, caps.max_cents),
            (DEFAULT_LIVE_DAILY_CAP, DEFAULT_LIVE_MAX_CENTS)
        );
    }

    #[test]
    fn modes_pair_money_with_the_matching_environment() {
        let (prod, tem, stub) = (
            "https://apis.usps.com",
            "https://apis-tem.usps.com",
            "http://127.0.0.1:9100",
        );
        // Test money: the test environment or a stub, never production.
        assert!(mode_ok(tem, false, false));
        assert!(mode_ok(stub, false, true));
        assert!(!mode_ok(prod, false, true));
        // Live money: production, only with usps.live on.
        assert!(mode_ok(prod, true, true));
        assert!(!mode_ok(prod, true, false));
        assert!(!mode_ok(tem, true, true));
    }

    /// Answers by URL: prices, payment, label (the stub records headers
    /// through the default-refusing path, so it implements them).
    type Calls = Vec<(String, Vec<(String, String)>)>;

    struct Stub {
        calls: std::cell::RefCell<Calls>,
    }

    impl HttpTransport for Stub {
        fn post_json(
            &self,
            url: &str,
            _bearer: &str,
            _accept: Option<&str>,
            _body: &Value,
        ) -> Result<labels::HttpResponse, labels::TransportError> {
            self.post_json_with(url, _bearer, _accept, &[], _body)
        }
        fn get(&self, _: &str, _: &str) -> Result<labels::HttpResponse, labels::TransportError> {
            Ok(labels::HttpResponse {
                status: 404,
                body: Vec::new(),
            })
        }
        fn delete(
            &self,
            url: &str,
            b: &str,
        ) -> Result<labels::HttpResponse, labels::TransportError> {
            self.delete_with(url, b, &[])
        }
        fn post_json_with(
            &self,
            url: &str,
            _bearer: &str,
            _accept: Option<&str>,
            headers: &[(&str, &str)],
            _body: &Value,
        ) -> Result<labels::HttpResponse, labels::TransportError> {
            use base64::Engine as _;
            self.calls.borrow_mut().push((
                url.to_string(),
                headers
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            ));
            let body = if url.ends_with(v3::PRICES_PATH) {
                br#"{"rateOptions":[{"totalPrice":9.85}]}"#.to_vec()
            } else {
                let pdf =
                    base64::engine::general_purpose::STANDARD.encode(b"%PDF-1.4 synthetic usps");
                serde_json::to_vec(&serde_json::json!({
                    "labelMetadata": {"trackingNumber": "9405 5000 0000 0000 0000 01", "postage": 9.85},
                    "labelImage": pdf,
                }))
                .unwrap()
            };
            Ok(labels::HttpResponse { status: 200, body })
        }
        fn delete_with(
            &self,
            url: &str,
            _bearer: &str,
            headers: &[(&str, &str)],
        ) -> Result<labels::HttpResponse, labels::TransportError> {
            self.calls.borrow_mut().push((
                url.to_string(),
                headers
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            ));
            Ok(labels::HttpResponse {
                status: 200,
                body: b"{}".to_vec(),
            })
        }
    }

    #[test]
    fn the_link_quotes_buys_and_cancels_with_the_payment_header() {
        let link = UspsLink {
            base: "https://apis-tem.usps.com".to_string(),
            bearer: "synthetic".to_string(),
            payment: "pay-synthetic".to_string(),
            epa: "0000000000".to_string(),
            live_max: Some(900),
            transport: Stub {
                calls: Default::default(),
            },
        };
        let payload = labels::payload::ShipmentPayload {
            from: labels::payload::PostalAddress {
                name: "Pat Example".into(),
                street1: "2 Elm St".into(),
                street2: None,
                city: "Seattle".into(),
                state_province: "WA".into(),
                postal: "98101".into(),
                country: "US".into(),
                phone: None,
            },
            to: labels::payload::PostalAddress {
                name: "Office".into(),
                street1: "801 Capitol Way S".into(),
                street2: None,
                city: "Olympia".into(),
                state_province: "WA".into(),
                postal: "98501".into(),
                country: "US".into(),
                phone: None,
            },
            parcel: labels::payload::Parcel::LetterEnvelope,
            service: labels::payload::ServiceClass::TwoDay,
            pairing_id: "ord_test".into(),
        };
        let request = BuyRequest {
            payload: &payload,
            direction: labels::payload::Direction::Outbound,
            payment_ref: "pay-synthetic",
        };
        let cfg = LiveConfig {
            enabled: true,
            daily_cap: 10,
            max_usd_cents: 6_000,
            ship_to_allow: vec!["98501".to_string()],
        };
        // 985c is over the 900c live ceiling: refused at the quote.
        assert!(
            link.quote(&cfg, &request)
                .unwrap_err()
                .contains("usps.live_max_usd")
        );
        let open = UspsLink {
            live_max: None,
            ..link
        };
        assert_eq!(open.quote(&cfg, &request), Ok(985));
        let outcome = open.buy(&request).expect("buys");
        assert_eq!(outcome.postage_cost_cents, Some(985));
        open.cancel(&outcome.tracking_no).expect("cancels");
        let calls = open.transport.calls.borrow();
        let label = calls
            .iter()
            .find(|(u, _)| u.ends_with("/labels/v3/label"))
            .unwrap();
        assert_eq!(
            label.1,
            vec![(
                "X-Payment-Authorization-Token".to_string(),
                "pay-synthetic".to_string()
            )]
        );
        assert!(
            calls
                .last()
                .unwrap()
                .0
                .contains("/labels/v3/label/9405%205000")
        );
    }
}
