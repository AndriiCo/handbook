//! USPS APIs v3 request and response shapes (OAuth, payment
//! authorization, labels, prices, tracking), as pure functions over
//! JSON so every shape is tested without the network.
//!
//! Sources: `docs/specs/carrier-apis.md` §1 and `shipping-design.md`
//! §5.1 (the official USPS/api-examples domestic-label example). Field
//! spellings marked "TEM" below are the documented ones that the first
//! run against the USPS test environment must confirm.

use serde_json::{Value, json};

use crate::payload::{Direction, PostalAddress, ServiceClass, ShipmentPayload};

/// OAuth 2.0 client-credentials token endpoint (relative to the base).
pub const OAUTH_PATH: &str = "/oauth2/v3/token";
/// Mints the payment-authorization token labels are paid with.
pub const PAYMENT_PATH: &str = "/payments/v3/payment-authorization";
/// Domestic label (returns use [`RETURN_LABEL_PATH`]).
pub const LABEL_PATH: &str = "/labels/v3/label";
pub const RETURN_LABEL_PATH: &str = "/labels/v3/returnlabel";
/// Domestic total-rates search (base price plus extras).
pub const PRICES_PATH: &str = "/prices/v3/total-rates/search";
/// The header the label API reads the payment token from.
pub const PAYMENT_HEADER: &str = "X-Payment-Authorization-Token";
/// Production and test hosts.
pub const PROD_HOST: &str = "apis.usps.com";
pub const TEM_HOST: &str = "apis-tem.usps.com";

/// v3 `mailClass` for a neutral service tier.
pub fn mail_class(service: ServiceClass) -> &'static str {
    match service {
        ServiceClass::Overnight => "PRIORITY_MAIL_EXPRESS",
        ServiceClass::TwoDay => "PRIORITY_MAIL",
        ServiceClass::Ground => "USPS_GROUND_ADVANTAGE",
    }
}

/// The client-credentials token request body.
pub fn token_body(client_id: &str, client_secret: &str) -> Value {
    json!({
        "grant_type": "client_credentials",
        "client_id": client_id,
        "client_secret": client_secret,
    })
}

/// `access_token` from a token answer.
pub fn parse_token(body: &[u8]) -> Result<String, String> {
    let json: Value =
        serde_json::from_slice(body).map_err(|_| "token answer is not json".to_string())?;
    json["access_token"]
        .as_str()
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "token answer has no access_token".to_string())
}

/// The accounts a payment token is minted for (Business Customer
/// Gateway: CRID, Mailer IDs and the Enterprise Payment Account).
pub struct PaymentAccount<'a> {
    pub crid: &'a str,
    pub mid: &'a str,
    pub manifest_mid: &'a str,
    pub epa_account: &'a str,
}

/// Payment-authorization body: we are both the payer and the label
/// owner (one EPS account pays for every label we buy).
pub fn payment_body(account: &PaymentAccount) -> Value {
    let role = |name: &str| {
        json!({
            "roleName": name,
            "CRID": account.crid,
            "MID": account.mid,
            "manifestMID": account.manifest_mid,
            "accountType": "EPS",
            "accountNumber": account.epa_account,
        })
    };
    json!({ "roles": [role("PAYER"), role("LABEL_OWNER")] })
}

/// `paymentAuthorizationToken` from a payment answer.
pub fn parse_payment_token(body: &[u8]) -> Result<String, String> {
    let json: Value =
        serde_json::from_slice(body).map_err(|_| "payment answer is not json".to_string())?;
    json["paymentAuthorizationToken"]
        .as_str()
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "payment answer has no paymentAuthorizationToken".to_string())
}

/// Weight in pounds (two decimals, at least 0.1) and inches for a parcel.
fn package(payload: &ShipmentPayload) -> (f64, u32, u32, u32) {
    let dims = payload.parcel.dims();
    let pounds = ((f64::from(dims.weight_g) / 453.592) * 100.0).ceil() / 100.0;
    (
        pounds.max(0.1),
        (dims.length_cm / 2.54).ceil() as u32,
        (dims.width_cm / 2.54).ceil() as u32,
        (dims.height_cm / 2.54).ceil() as u32,
    )
}

/// Envelopes mail as flats; anything else as a machinable parcel.
fn processing_category(payload: &ShipmentPayload) -> &'static str {
    if payload.parcel.is_envelope() {
        "FLATS"
    } else {
        "MACHINABLE"
    }
}

/// `YYYY-MM-DD` (UTC) for a unix time.
pub fn iso_date(unix: i64) -> String {
    // Civil-from-days (Howard Hinnant), valid for the whole i64 day range we use.
    let days = unix.div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// A v3 address. A person's name splits into first and last name; a
/// longer name (an office) goes in `firm` (TEM: confirm the printout).
pub fn address(a: &PostalAddress) -> Value {
    let (zip, plus4) = match a.postal.trim().split_once('-') {
        Some((zip, plus4)) => (zip.to_string(), Some(plus4.to_string())),
        None => (a.postal.trim().to_string(), None),
    };
    let words: Vec<&str> = a.name.split_whitespace().collect();
    let mut out = json!({
        "streetAddress": a.street1,
        "city": a.city,
        "state": a.state_province,
        "ZIPCode": zip,
    });
    if words.len() > 3 || words.len() < 2 {
        out["firm"] = json!(a.name.trim());
    } else {
        out["firstName"] = json!(words[..words.len() - 1].join(" "));
        out["lastName"] = json!(words[words.len() - 1]);
    }
    if let Some(line2) = a.street2.as_deref().filter(|l| !l.trim().is_empty()) {
        out["secondaryAddress"] = json!(line2);
    }
    if let Some(plus4) = plus4 {
        out["ZIPPlus4"] = json!(plus4);
    }
    if let Some(phone) = a.phone.as_deref() {
        let digits: String = phone.chars().filter(char::is_ascii_digit).collect();
        // US numbers as 10 digits (a leading country code 1 dropped).
        let digits = digits
            .strip_prefix('1')
            .filter(|d| d.len() == 10)
            .unwrap_or(&digits);
        if !digits.is_empty() {
            out["phone"] = json!(digits);
        }
    }
    out
}

/// The domestic label request (`imageInfo` + addresses +
/// `packageDescription`), per the official domestic-label example.
pub fn label_body(payload: &ShipmentPayload, direction: Direction, mailing_unix: i64) -> Value {
    let (weight, length, width, height) = package(payload);
    json!({
        "imageInfo": {
            "imageType": "PDF",
            "labelType": "4X6LABEL",
            "receiptOption": "NONE",
            "suppressPostage": false,
            "suppressMailDate": false,
            "returnLabel": direction == Direction::Return,
        },
        "toAddress": address(&payload.to),
        "fromAddress": address(&payload.from),
        "packageDescription": {
            "mailClass": mail_class(payload.service),
            "rateIndicator": "SP",
            "weightUOM": "lb",
            "weight": weight,
            "dimensionsUOM": "in",
            "length": length,
            "width": width,
            "height": height,
            "processingCategory": processing_category(payload),
            "mailingDate": iso_date(mailing_unix),
            "extraServices": [],
            "destinationEntryFacilityType": "NONE",
        },
    })
}

/// The total-rates search for one leg, priced against our EPS account.
pub fn prices_body(payload: &ShipmentPayload, epa_account: &str, mailing_unix: i64) -> Value {
    let (weight, length, width, height) = package(payload);
    let zip5 = |p: &str| p.trim().chars().take(5).collect::<String>();
    json!({
        "originZIPCode": zip5(&payload.from.postal),
        "destinationZIPCode": zip5(&payload.to.postal),
        "weight": weight,
        "length": length,
        "width": width,
        "height": height,
        "mailClass": mail_class(payload.service),
        "processingCategory": processing_category(payload),
        "rateIndicator": "SP",
        "destinationEntryFacilityType": "NONE",
        "priceType": "COMMERCIAL",
        "mailingDate": iso_date(mailing_unix),
        "accountType": "EPS",
        "accountNumber": epa_account,
    })
}

/// Dollars (a JSON number or a decimal string) to cents.
fn cents(value: &Value) -> Option<i64> {
    let dollars = match value {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => s.trim().parse::<f64>().ok()?,
        _ => return None,
    };
    (dollars.is_finite() && dollars >= 0.0).then(|| (dollars * 100.0).round() as i64)
}

/// Cheapest total price in a rates answer, in cents (`rateOptions[]`
/// carry `totalPrice`, else `totalBasePrice`).
pub fn parse_price_cents(json: &Value) -> Option<i64> {
    json["rateOptions"]
        .as_array()?
        .iter()
        .filter_map(|o| cents(&o["totalPrice"]).or_else(|| cents(&o["totalBasePrice"])))
        .filter(|c| *c > 0)
        .min()
}

/// Tracking number and postage (cents) of a label answer: v3 nests both
/// under `labelMetadata`.
pub fn label_metadata(json: &Value) -> (Option<String>, Option<i64>) {
    let meta = &json["labelMetadata"];
    let tracking = meta["trackingNumber"]
        .as_str()
        .or_else(|| json["trackingNumber"].as_str())
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string);
    (tracking, cents(&meta["postage"]))
}

/// Single-package tracking (`?expand=DETAIL`).
pub fn tracking_url(base: &str, tracking_no: &str) -> String {
    format!(
        "{}/tracking/v3/tracking/{}?expand=DETAIL",
        base.trim_end_matches('/'),
        tracking_no.trim().replace(' ', "%20")
    )
}

pub use crate::track::{TrackingScan, TrackingStatus};

/// Status bucket and scans of a v3 tracking answer.
pub fn parse_tracking(json: &Value) -> (Option<TrackingStatus>, Vec<TrackingScan>) {
    let category = json["statusCategory"]
        .as_str()
        .unwrap_or("")
        .to_ascii_lowercase();
    let status = if category.contains("delivered") && !category.contains("out for") {
        Some(TrackingStatus::Delivered)
    } else if category.contains("alert")
        || category.contains("return to sender")
        || category.contains("undeliverable")
    {
        Some(TrackingStatus::Alert)
    } else if category.contains("pre-shipment") || category.contains("label created") {
        Some(TrackingStatus::PreTransit)
    } else if category.is_empty() {
        None
    } else {
        Some(TrackingStatus::InTransit)
    };
    let scans = json["trackingEvents"]
        .as_array()
        .map(|events| {
            events
                .iter()
                .filter_map(|e| {
                    let text = e["eventType"].as_str()?.trim().to_string();
                    let at = e["eventTimestamp"]
                        .as_str()
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    let code = e["eventCode"]
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| text.clone());
                    let city = e["eventCity"].as_str().unwrap_or("").trim();
                    let state = e["eventState"].as_str().unwrap_or("").trim();
                    let place = match (city.is_empty(), state.is_empty()) {
                        (false, false) => format!("{city}, {state}"),
                        (false, true) => city.to_string(),
                        (true, false) => state.to_string(),
                        (true, true) => String::new(),
                    };
                    Some(TrackingScan {
                        code,
                        text,
                        at,
                        place,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    (status, scans)
}

/// Host of a base URL, lowercased (no port).
pub fn host(base: &str) -> Option<String> {
    let rest = base
        .trim()
        .strip_prefix("https://")
        .or_else(|| base.trim().strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = if let Some(v6) = authority.strip_prefix('[') {
        format!("[{}]", v6.split(']').next()?)
    } else {
        authority.split(':').next()?.to_string()
    };
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// True for the USPS production host.
pub fn is_production(base: &str) -> bool {
    host(base).as_deref() == Some(PROD_HOST) && base.trim().starts_with("https://")
}

/// True for the USPS test environment or a loopback stub.
pub fn is_test(base: &str) -> bool {
    match host(base).as_deref() {
        Some(TEM_HOST) => base.trim().starts_with("https://"),
        Some("127.0.0.1" | "localhost" | "[::1]") => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::Parcel;

    fn payload() -> ShipmentPayload {
        ShipmentPayload {
            from: PostalAddress {
                name: "Pat Example".into(),
                street1: "2 Elm St".into(),
                street2: Some("Apt 3".into()),
                city: "Seattle".into(),
                state_province: "WA".into(),
                postal: "98101-1234".into(),
                country: "US".into(),
                phone: Some("+1 (206) 555-0100".into()),
            },
            to: PostalAddress {
                name: "Office of the Secretary of State".into(),
                street1: "801 Capitol Way S".into(),
                street2: None,
                city: "Olympia".into(),
                state_province: "WA".into(),
                postal: "98501".into(),
                country: "US".into(),
                phone: None,
            },
            parcel: Parcel::LetterEnvelope,
            service: ServiceClass::TwoDay,
            pairing_id: "ord_test".into(),
        }
    }

    #[test]
    fn label_body_follows_the_documented_v3_shape() {
        let body = label_body(&payload(), Direction::Outbound, 1_793_750_400);
        assert_eq!(body["imageInfo"]["imageType"], "PDF");
        assert_eq!(body["imageInfo"]["labelType"], "4X6LABEL");
        assert_eq!(body["imageInfo"]["returnLabel"], false);
        let pkg = &body["packageDescription"];
        assert_eq!(pkg["mailClass"], "PRIORITY_MAIL");
        assert_eq!(pkg["processingCategory"], "FLATS");
        assert_eq!(pkg["weightUOM"], "lb");
        assert!(pkg["weight"].as_f64().unwrap() >= 0.1);
        assert_eq!(pkg["mailingDate"], "2026-11-04");
        // The payment token is a header, never part of the body.
        assert!(!body.to_string().contains("paymentAuthorizationToken"));
        assert!(body.get("labelSpecification").is_none());
        let from = &body["fromAddress"];
        assert_eq!(
            (from["firstName"].as_str(), from["lastName"].as_str()),
            (Some("Pat"), Some("Example"))
        );
        assert_eq!(
            (from["ZIPCode"].as_str(), from["ZIPPlus4"].as_str()),
            (Some("98101"), Some("1234"))
        );
        assert_eq!(from["phone"], "2065550100");
        assert_eq!(from["secondaryAddress"], "Apt 3");
        assert_eq!(
            body["toAddress"]["firm"],
            "Office of the Secretary of State"
        );
        let back = label_body(&payload(), Direction::Return, 1_793_750_400);
        assert_eq!(back["imageInfo"]["returnLabel"], true);
    }

    #[test]
    fn payment_roles_carry_every_account_number() {
        let body = payment_body(&PaymentAccount {
            crid: "11111111",
            mid: "222222",
            manifest_mid: "333333",
            epa_account: "0000000000",
        });
        let roles = body["roles"].as_array().unwrap();
        assert_eq!(roles.len(), 2);
        assert_eq!(roles[0]["roleName"], "PAYER");
        assert_eq!(roles[1]["roleName"], "LABEL_OWNER");
        for role in roles {
            assert_eq!(role["accountType"], "EPS");
            assert_eq!(role["CRID"], "11111111");
            assert_eq!(role["manifestMID"], "333333");
        }
        assert_eq!(
            parse_payment_token(br#"{"paymentAuthorizationToken":"tok"}"#).unwrap(),
            "tok"
        );
        assert!(parse_payment_token(br#"{}"#).is_err());
        assert_eq!(
            parse_token(br#"{"access_token":"a","expires_in":28800}"#).unwrap(),
            "a"
        );
        assert!(parse_token(b"<html>").is_err());
    }

    #[test]
    fn prices_parse_the_cheapest_total_in_cents() {
        let body = prices_body(&payload(), "0000000000", 1_793_750_400);
        assert_eq!(body["originZIPCode"], "98101");
        assert_eq!(body["destinationZIPCode"], "98501");
        assert_eq!(body["accountType"], "EPS");
        let answer = json!({"rateOptions": [
            {"totalBasePrice": 10.4},
            {"totalPrice": "9.85", "totalBasePrice": 9.85},
            {"totalPrice": 0}
        ]});
        assert_eq!(parse_price_cents(&answer), Some(985));
        assert_eq!(parse_price_cents(&json!({})), None);
    }

    #[test]
    fn label_answers_and_tracking_parse() {
        let (tn, cents) = label_metadata(&json!({
            "labelMetadata": {"trackingNumber": "9405 5000 0000 0000 0000 00", "postage": 9.85}
        }));
        assert_eq!(tn.as_deref(), Some("9405 5000 0000 0000 0000 00"));
        assert_eq!(cents, Some(985));
        let (status, scans) = parse_tracking(&json!({
            "statusCategory": "Delivered",
            "trackingEvents": [
                {"eventType": "Delivered, In/At Mailbox", "eventTimestamp": "2026-11-05T10:12:00",
                 "eventCity": "OLYMPIA", "eventState": "WA", "eventCode": "01"},
                {"eventType": "Arrived at USPS Facility", "eventTimestamp": "2026-11-04T08:00:00",
                 "eventCity": "SEATTLE", "eventState": "WA"}
            ]
        }));
        assert_eq!(status, Some(TrackingStatus::Delivered));
        assert_eq!(scans.len(), 2);
        assert_eq!(scans[0].code, "01");
        assert_eq!(scans[0].place, "OLYMPIA, WA");
        assert_eq!(scans[1].code, "Arrived at USPS Facility");
        assert_eq!(
            parse_tracking(&json!({"statusCategory": "In Transit"})).0,
            Some(TrackingStatus::InTransit)
        );
        assert_eq!(
            parse_tracking(&json!({"statusCategory": "Out for Delivery"})).0,
            Some(TrackingStatus::InTransit)
        );
        assert_eq!(
            parse_tracking(&json!({"statusCategory": "Alert"})).0,
            Some(TrackingStatus::Alert)
        );
        assert_eq!(
            tracking_url("https://apis-tem.usps.com/", "9405 5000"),
            "https://apis-tem.usps.com/tracking/v3/tracking/9405%205000?expand=DETAIL"
        );
    }

    #[test]
    fn hosts_split_production_from_test() {
        assert!(is_production("https://apis.usps.com"));
        assert!(!is_production("http://apis.usps.com"));
        assert!(!is_production("https://apis.usps.com.evil.test"));
        assert!(is_test("https://apis-tem.usps.com/"));
        assert!(is_test("http://127.0.0.1:9000"));
        assert!(is_test("http://[::1]:9000"));
        assert!(!is_test("https://apis.usps.com"));
        assert!(!is_test("not a url"));
        assert_eq!(iso_date(0), "1970-01-01");
        assert_eq!(iso_date(951_782_400), "2000-02-29");
    }
}
