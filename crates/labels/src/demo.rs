//! W-3 fake carrier (`labels.mode = fake`): demo labels with no carrier,
//! no credentials and no postage.
//!
//! [`DemoTransport`] is an [`HttpTransport`] that answers the FedEx Ship
//! and Rate calls and the USPS Labels and Prices calls the clients
//! already make, in each carrier's documented answer shape, so the web
//! layer's existing buy paths — caps, idempotency ledger, audit rows,
//! label storage, the customer's cabinet — run unchanged against it.
//! Nothing leaves the process.
//!
//! Every label it issues is a PDF stamped "DEMO — NOT POSTAGE" with a
//! tracking number from the reserved fake range [`DEMO_TRACKING_PREFIX`]
//! (`DEMO` + 12 digits: no carrier issues letters there), which
//! [`is_demo_tracking`] recognizes so the tracking poller never sends
//! one to a carrier. Quotes are a flat demo price; postage is zero.

use std::sync::atomic::{AtomicU64, Ordering};

use base64::Engine as _;
use serde_json::{Value, json};

use crate::client::{HttpResponse, HttpTransport, TransportError};

/// Base URL the web layer hands the clients in fake mode. `.invalid`
/// (RFC 2606): nothing could resolve it even if a call escaped.
pub const DEMO_BASE: &str = "https://demo-carrier.invalid";
/// Every demo tracking number starts with this; the rest is 12 digits.
pub const DEMO_TRACKING_PREFIX: &str = "DEMO";
/// The stamp on every demo label.
pub const DEMO_STAMP: &str = "DEMO — NOT POSTAGE";
/// Flat demo quote per leg, in cents (FedEx 12.50, USPS 9.85).
pub const DEMO_FEDEX_CENTS: i64 = 1_250;
pub const DEMO_USPS_CENTS: i64 = 985;

static SEQ: AtomicU64 = AtomicU64::new(0);

/// True for a number from the reserved demo range.
pub fn is_demo_tracking(tracking_no: &str) -> bool {
    let t = tracking_no.trim();
    t.len() == DEMO_TRACKING_PREFIX.len() + 12
        && t.starts_with(DEMO_TRACKING_PREFIX)
        && t[DEMO_TRACKING_PREFIX.len()..]
            .bytes()
            .all(|b| b.is_ascii_digit())
}

/// A fresh demo tracking number (unique within the process, and across
/// restarts by the clock).
pub fn demo_tracking_number() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    format!(
        "{DEMO_TRACKING_PREFIX}{:012}",
        (secs % 100_000_000) * 10_000 + seq % 10_000
    )
}

/// PDF literal-string text: printable ASCII only (anything else reads
/// `?`), the three specials escaped, at most `max` characters.
fn pdf_text(text: &str, max: usize) -> String {
    let mut out = String::new();
    for c in text.trim().chars().take(max) {
        match c {
            '(' | ')' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            ' '..='~' => out.push(c),
            _ => out.push('?'),
        }
    }
    out
}

/// One 4x6 in label page stamped [`DEMO_STAMP`], naming the carrier,
/// service, tracking number and destination. Deterministic for its
/// inputs; a valid PDF 1.4 file (offsets computed, Helvetica, WinAnsi).
pub fn demo_label_pdf(carrier: &str, service: &str, tracking_no: &str, to: &[String]) -> Vec<u8> {
    let mut lines = vec![
        format!("{} {}", pdf_text(carrier, 20), pdf_text(service, 30)),
        format!("Tracking {}", pdf_text(tracking_no, 40)),
        "Ship to:".to_string(),
    ];
    lines.extend(to.iter().take(4).map(|l| pdf_text(l, 40)));
    let mut content = String::new();
    // Diagonal grey watermark across the page.
    content.push_str(
        "BT /F1 36 Tf 0.85 g 0.7071 0.7071 -0.7071 0.7071 40 70 Tm (DEMO NOT POSTAGE) Tj ET\n",
    );
    // Red stamp box with the stamp text (\227 is the WinAnsi em dash).
    content.push_str("1 0 0 RG 3 w 14 352 260 54 re S\n");
    content.push_str("BT /F1 21 Tf 1 0 0 rg 24 372 Td (DEMO \\227 NOT POSTAGE) Tj ET\n");
    let mut y = 320;
    for line in &lines {
        content.push_str(&format!("BT /F2 12 Tf 0 g 24 {y} Td ({line}) Tj ET\n"));
        y -= 18;
    }
    content.push_str(
        "BT /F2 9 Tf 0 g 24 40 Td (Fake mode: no carrier was called. No postage. Do not mail.) Tj ET\n",
    );
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 288 432] \
         /Resources << /Font << /F1 4 0 R /F2 5 0 R >> >> /Contents 6 0 R >>"
            .to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>"
            .to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_string(),
        format!(
            "<< /Length {} >>\nstream\n{content}endstream",
            content.len()
        ),
    ];
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (i, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    pdf
}

/// Destination lines from a FedEx Ship or USPS Labels request body.
fn destination(body: &Value) -> Vec<String> {
    let s = |v: &Value| v.as_str().unwrap_or("").trim().to_string();
    let fedex = &body["requestedShipment"]["recipients"][0];
    if fedex.is_object() {
        let a = &fedex["address"];
        return vec![
            s(&fedex["contact"]["personName"]),
            s(&a["streetLines"][0]),
            format!(
                "{} {} {}",
                s(&a["city"]),
                s(&a["stateOrProvinceCode"]),
                s(&a["postalCode"])
            ),
        ];
    }
    let a = &body["toAddress"];
    let name = match (s(&a["firstName"]), s(&a["lastName"]), s(&a["firm"])) {
        (first, last, _) if !last.is_empty() => format!("{first} {last}"),
        (_, _, firm) => firm,
    };
    vec![
        name,
        s(&a["streetAddress"]),
        format!("{} {} {}", s(&a["city"]), s(&a["state"]), s(&a["ZIPCode"])),
    ]
}

fn ok(body: Value) -> Result<HttpResponse, TransportError> {
    Ok(HttpResponse {
        status: 200,
        body: serde_json::to_vec(&body).unwrap_or_default(),
    })
}

fn not_found() -> Result<HttpResponse, TransportError> {
    Ok(HttpResponse {
        status: 404,
        body: b"{}".to_vec(),
    })
}

/// The fake carrier. Stateless; routes by URL path.
#[derive(Debug, Clone, Copy, Default)]
pub struct DemoTransport;

impl DemoTransport {
    fn answer(&self, url: &str, body: &Value) -> Result<HttpResponse, TransportError> {
        if url.contains("/rate/v1/rates") {
            return ok(json!({ "output": { "rateReplyDetails": [{
                "serviceType": "DEMO",
                "ratedShipmentDetails": [{
                    "rateType": "ACCOUNT",
                    "totalNetCharge": DEMO_FEDEX_CENTS as f64 / 100.0,
                    "currency": "USD",
                }],
            }]}}));
        }
        if url.contains(crate::usps::PRICES_PATH) {
            return ok(
                json!({ "rateOptions": [{ "totalPrice": DEMO_USPS_CENTS as f64 / 100.0 }] }),
            );
        }
        let encoded = |carrier: &str, service: &str, tracking: &str| {
            base64::engine::general_purpose::STANDARD.encode(demo_label_pdf(
                carrier,
                service,
                tracking,
                &destination(body),
            ))
        };
        if url.contains("/ship/v1/shipments") && !url.contains("cancel") {
            let tracking = demo_tracking_number();
            let service = body["requestedShipment"]["serviceType"]
                .as_str()
                .unwrap_or("");
            return ok(json!({ "output": { "transactionShipments": [{
                "masterTrackingNumber": tracking,
                "pieceResponses": [{
                    "trackingNumber": tracking,
                    "packageDocuments": [{
                        "contentType": "LABEL",
                        "docType": "PDF",
                        "encodedLabel": encoded("FedEx", service, &tracking),
                    }],
                }],
            }]}}));
        }
        if url.contains(crate::usps::LABEL_PATH) || url.contains(crate::usps::RETURN_LABEL_PATH) {
            let tracking = demo_tracking_number();
            let service = body["packageDescription"]["mailClass"]
                .as_str()
                .unwrap_or("");
            return ok(json!({
                "labelMetadata": { "trackingNumber": tracking, "postage": 0 },
                "labelImage": encoded("USPS", service, &tracking),
            }));
        }
        not_found()
    }
}

impl HttpTransport for DemoTransport {
    fn post_json(
        &self,
        url: &str,
        _bearer: &str,
        _accept: Option<&str>,
        body: &Value,
    ) -> Result<HttpResponse, TransportError> {
        self.answer(url, body)
    }

    fn post_json_with(
        &self,
        url: &str,
        _bearer: &str,
        _accept: Option<&str>,
        _headers: &[(&str, &str)],
        body: &Value,
    ) -> Result<HttpResponse, TransportError> {
        self.answer(url, body)
    }

    fn get(&self, _url: &str, _bearer: &str) -> Result<HttpResponse, TransportError> {
        not_found()
    }

    /// USPS cancels with DELETE: a demo label always cancels.
    fn delete(&self, url: &str, _bearer: &str) -> Result<HttpResponse, TransportError> {
        if url.contains(crate::usps::LABEL_PATH) {
            ok(json!({}))
        } else {
            not_found()
        }
    }

    fn delete_with(
        &self,
        url: &str,
        bearer: &str,
        _headers: &[(&str, &str)],
    ) -> Result<HttpResponse, TransportError> {
        self.delete(url, bearer)
    }

    /// FedEx cancels with PUT.
    fn put_json(
        &self,
        url: &str,
        _bearer: &str,
        _body: &Value,
    ) -> Result<HttpResponse, TransportError> {
        if url.contains("/ship/v1/shipments/cancel") {
            ok(json!({ "output": { "cancelledShipment": true } }))
        } else {
            not_found()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{BuyRequest, FedExClient, LabelClient, UspsClient};
    use crate::payload::{Direction, Parcel, PostalAddress, ServiceClass, ShipmentPayload};

    fn party(name: &str, postal: &str) -> PostalAddress {
        PostalAddress {
            name: name.to_string(),
            street1: "1 Main St (rear)".to_string(),
            street2: None,
            city: "Olympia".to_string(),
            state_province: "WA".to_string(),
            postal: postal.to_string(),
            country: "US".to_string(),
            phone: Some("+1 360 555 0100".to_string()),
        }
    }

    fn payload() -> ShipmentPayload {
        ShipmentPayload {
            from: party("Jane Customer", "98101"),
            to: party("Office of the Secretary", "98504"),
            parcel: Parcel::FlatEnvelope,
            service: ServiceClass::TwoDay,
            pairing_id: "demo-pair".to_string(),
        }
    }

    /// Every xref offset must point at its `N 0 obj` (a reader opens it).
    fn assert_valid_pdf(pdf: &[u8]) {
        let text = String::from_utf8_lossy(pdf);
        assert!(text.starts_with("%PDF-1.4\n") && text.ends_with("%%EOF\n"));
        let start: usize = text
            .rsplit_once("startxref\n")
            .and_then(|(_, rest)| rest.lines().next())
            .and_then(|n| n.parse().ok())
            .expect("startxref");
        assert!(text[start..].starts_with("xref\n0 7\n"));
        for (i, line) in text[start..].lines().skip(3).take(6).enumerate() {
            let offset: usize = line[..10].parse().unwrap();
            assert!(
                text[offset..].starts_with(&format!("{} 0 obj", i + 1)),
                "object {} at {offset}",
                i + 1
            );
        }
    }

    #[test]
    fn the_demo_label_is_a_stamped_pdf() {
        let pdf = demo_label_pdf(
            "FedEx",
            "PRIORITY_OVERNIGHT",
            "DEMO000000000001",
            &["Office (main)".to_string(), "Café row".to_string()],
        );
        assert_valid_pdf(&pdf);
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("(DEMO \\227 NOT POSTAGE) Tj"), "stamp");
        assert!(text.contains("(Tracking DEMO000000000001) Tj"));
        assert!(
            text.contains("(Office \\(main\\)) Tj"),
            "parentheses escaped"
        );
        assert!(text.contains("(Caf? row) Tj"), "non-ASCII replaced");
        assert!(text.contains("No postage. Do not mail."));
    }

    #[test]
    fn demo_tracking_numbers_come_from_the_reserved_range() {
        let a = demo_tracking_number();
        let b = demo_tracking_number();
        assert_ne!(a, b);
        assert!(is_demo_tracking(&a) && is_demo_tracking(&b), "{a} {b}");
        for real in [
            "794600000000",
            "9400100000000000000000",
            "DEMO12",
            "DEMOX00000000000",
        ] {
            assert!(!is_demo_tracking(real), "{real}");
        }
    }

    #[test]
    fn fedex_and_usps_clients_buy_and_cancel_demo_labels() {
        let payload = payload();
        let request = BuyRequest {
            payload: &payload,
            direction: Direction::Outbound,
            payment_ref: "DEMO",
        };
        let fedex = FedExClient::new(DEMO_BASE, DemoTransport, true).with_account("DEMO");
        let bought = fedex.buy("demo", &request).unwrap();
        assert!(
            is_demo_tracking(&bought.tracking_no),
            "{}",
            bought.tracking_no
        );
        assert_valid_pdf(&bought.label_pdf);
        let text = String::from_utf8_lossy(&bought.label_pdf).into_owned();
        assert!(text.contains("DEMO \\227 NOT POSTAGE"));
        assert!(
            text.contains("Office of the Secretary"),
            "destination on the label"
        );
        fedex.cancel("demo", &bought.tracking_no).unwrap();

        let usps = UspsClient::new(DEMO_BASE, DemoTransport, true);
        let bought = usps.buy("demo", &request).unwrap();
        assert!(is_demo_tracking(&bought.tracking_no));
        assert_eq!(bought.postage_cost_cents, Some(0));
        assert!(String::from_utf8_lossy(&bought.label_pdf).contains("98504"));
        usps.cancel("demo", &bought.tracking_no).unwrap();
    }

    #[test]
    fn quotes_are_flat_demo_prices() {
        let rate = DemoTransport
            .post_json(
                &format!("{DEMO_BASE}/rate/v1/rates/quotes"),
                "demo",
                None,
                &json!({}),
            )
            .unwrap();
        let json: Value = serde_json::from_slice(&rate.body).unwrap();
        assert_eq!(
            crate::live::parse_quote_cents(&json),
            Some(DEMO_FEDEX_CENTS)
        );
        let price = DemoTransport
            .post_json(
                &format!("{DEMO_BASE}{}", crate::usps::PRICES_PATH),
                "demo",
                None,
                &json!({}),
            )
            .unwrap();
        let json: Value = serde_json::from_slice(&price.body).unwrap();
        assert_eq!(crate::usps::parse_price_cents(&json), Some(DEMO_USPS_CENTS));
        // Anything else is a 404, never a network call.
        let other = DemoTransport.get("https://apis.usps.com/x", "t").unwrap();
        assert_eq!(other.status, 404);
    }
}
