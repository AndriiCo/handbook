//! Carrier API-buy clients (carrier-apis.md, roadmap item 9).
//!
//! [`UspsClient`] and [`FedExClient`] buy ready-to-use labels over the
//! carrier REST APIs. Both are generic over [`HttpTransport`] so tests
//! run offline against stubs; [`UreqTransport`] is the live transport.
//!
//! Secrets discipline: OAuth bearers and payment refs (USPS payment
//! authorization token, FedEx account number) are passed per call by
//! the web layer (secure-config holds them). Clients never log them
//! and never persist them.
//!
//! Exact JSON shapes marked `verify at integration` come from public
//! docs/guides (see carrier-apis.md); the sandbox accounts confirm
//! them before the first real buy. Until then every buy runs against
//! stubs or the TEM/sandbox base URLs with synthetic payloads.

use std::fmt;
use std::time::Duration;

use serde_json::{Value, json};

use crate::carrier::{Carrier, fedex_service_type, usps_mail_class};
use crate::payload::{Direction, ShipmentPayload};

/// Production + sandbox base URLs (carrier-apis §1–2).
pub const USPS_BASE_PROD: &str = "https://apis.usps.com";
pub const USPS_BASE_TEST: &str = "https://apis-tem.usps.com";
pub const FEDEX_BASE_PROD: &str = "https://apis.fedex.com";
pub const FEDEX_BASE_SANDBOX: &str = "https://apis-sandbox.fedex.com";

/// USPS Labels v3 JSON accept header: without it the API answers
/// multipart instead of JSON.
pub const USPS_LABELS_ACCEPT: &str = "application/vnd.usps.labels+json";

/// Raw HTTP response: status + bytes. Carriers answer errors as JSON
/// too, so callers always get the body.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

/// Minimal HTTP surface the clients need. Blocking on purpose: label
/// buys are rare staff-paced operations, and the web layer runs them
/// on `spawn_blocking`.
pub trait HttpTransport {
    fn post_json(
        &self,
        url: &str,
        bearer: &str,
        accept: Option<&str>,
        body: &Value,
    ) -> Result<HttpResponse, TransportError>;

    fn get(&self, url: &str, bearer: &str) -> Result<HttpResponse, TransportError>;

    fn delete(&self, url: &str, bearer: &str) -> Result<HttpResponse, TransportError>;
}

/// Live [`HttpTransport`] over ureq (rustls, pure Rust).
pub struct UreqTransport {
    agent: ureq::Agent,
}

impl UreqTransport {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
        }
    }
}

impl Default for UreqTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpTransport for UreqTransport {
    fn post_json(
        &self,
        url: &str,
        bearer: &str,
        accept: Option<&str>,
        body: &Value,
    ) -> Result<HttpResponse, TransportError> {
        let mut request = self
            .agent
            .post(url)
            .header("Authorization", format!("Bearer {bearer}"))
            .header("Content-Type", "application/json");
        if let Some(accept) = accept {
            request = request.header("Accept", accept.to_string());
        }
        let response = request.send_json(body).map_err(TransportError::from_ureq)?;
        finish(response)
    }

    fn get(&self, url: &str, bearer: &str) -> Result<HttpResponse, TransportError> {
        let response = self
            .agent
            .get(url)
            .header("Authorization", format!("Bearer {bearer}"))
            .call()
            .map_err(TransportError::from_ureq)?;
        finish(response)
    }

    fn delete(&self, url: &str, bearer: &str) -> Result<HttpResponse, TransportError> {
        let response = self
            .agent
            .delete(url)
            .header("Authorization", format!("Bearer {bearer}"))
            .call()
            .map_err(TransportError::from_ureq)?;
        finish(response)
    }
}

fn finish(response: ureq::http::Response<ureq::Body>) -> Result<HttpResponse, TransportError> {
    let status = response.status().as_u16();
    let body = response
        .into_body()
        .read_to_vec()
        .map_err(TransportError::from_ureq)?;
    Ok(HttpResponse { status, body })
}

/// Transport failure (DNS, TLS, timeouts, protocol). The message never
/// includes the bearer token or the request body.
#[derive(Debug, Clone)]
pub struct TransportError {
    message: String,
}

impl TransportError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    fn from_ureq(error: ureq::Error) -> Self {
        Self {
            message: error.to_string(),
        }
    }
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "label http transport: {}", self.message)
    }
}

impl std::error::Error for TransportError {}

/// One label buy: the neutral payload, the direction, and the
/// carrier-side money reference (USPS payment-authorization token or
/// FedEx account number — server-side only, never customer PII).
pub struct BuyRequest<'a> {
    pub payload: &'a ShipmentPayload,
    pub direction: Direction,
    pub payment_ref: &'a str,
}

/// A bought label: tracking + PDF bytes + recorded cost.
#[derive(Debug, Clone)]
pub struct BuyOutcome {
    pub tracking_no: String,
    pub label_pdf: Vec<u8>,
    /// Recorded, never charged here (P2-L4) and never shown in UI.
    pub postage_cost_cents: Option<i64>,
    /// Carrier-native service name for the audit row.
    pub carrier_service: String,
}

/// One carrier's buy/cancel surface.
pub trait LabelClient {
    fn carrier(&self) -> Carrier;

    /// True once carrier approval + certification landed (USPS Labels
    /// approval + Ship/EPA enrollment; FedEx label certification).
    /// False routes buys to the staff-manual queue automatically.
    fn ready(&self) -> bool;

    /// Buy one label. `bearer` is a fresh OAuth token from the caller.
    ///
    /// # Errors
    ///
    /// Returns [`LabelError`] on transport failure, carrier rejection,
    /// or an unparseable success response.
    fn buy(&self, bearer: &str, request: &BuyRequest) -> Result<BuyOutcome, LabelError>;

    /// Void a bought label.
    ///
    /// # Errors
    ///
    /// Returns [`LabelError`] on transport failure or carrier rejection.
    fn cancel(&self, bearer: &str, tracking_no: &str) -> Result<(), LabelError>;
}

/// [`LabelClient`] failure. Messages are safe for staff display: they
/// carry the carrier's reason, never secrets.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum LabelError {
    Transport(TransportError),
    NotReady,
    EmptyPaymentRef,
    BadTrackingNo,
    Api { status: u16, message: String },
    BadResponse(String),
}

impl fmt::Display for LabelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LabelError::Transport(e) => write!(f, "{e}"),
            LabelError::NotReady => write!(f, "carrier api not certified yet"),
            LabelError::EmptyPaymentRef => write!(f, "payment reference is required"),
            LabelError::BadTrackingNo => write!(f, "tracking number is invalid"),
            LabelError::Api { status, message } => {
                write!(f, "carrier rejected the request (http {status}): {message}")
            }
            LabelError::BadResponse(message) => {
                write!(f, "carrier answer was unparseable: {message}")
            }
        }
    }
}

impl std::error::Error for LabelError {}

impl From<TransportError> for LabelError {
    fn from(e: TransportError) -> Self {
        LabelError::Transport(e)
    }
}

/// USPS Labels v3 client: `POST /labels/v3/label`,
/// `POST /labels/v3/returnlabel`,
/// `DELETE /labels/v3/label/{trackingNumber}`.
pub struct UspsClient<T: HttpTransport> {
    base_url: String,
    transport: T,
    certified: bool,
    paper: crate::PaperSize,
}

impl<T: HttpTransport> UspsClient<T> {
    pub fn new(base_url: &str, transport: T, certified: bool) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            transport,
            certified,
            paper: crate::PaperSize::FourBySix,
        }
    }

    /// Prepaid paper size for bought labels (Option B). Defaults to 4x6.
    pub fn with_paper(mut self, paper: crate::PaperSize) -> Self {
        self.paper = paper;
        self
    }

    fn label_url(&self, direction: Direction) -> String {
        match direction {
            Direction::Outbound => format!("{}/labels/v3/label", self.base_url),
            Direction::Return => format!("{}/labels/v3/returnlabel", self.base_url),
        }
    }
}

impl<T: HttpTransport> LabelClient for UspsClient<T> {
    fn carrier(&self) -> Carrier {
        Carrier::Usps
    }

    fn ready(&self) -> bool {
        self.certified
    }

    fn buy(&self, bearer: &str, request: &BuyRequest) -> Result<BuyOutcome, LabelError> {
        if !self.certified {
            return Err(LabelError::NotReady);
        }
        check_payment_ref(request.payment_ref)?;
        let body = usps_label_body_with_paper(request, self.paper);
        let response = self.transport.post_json(
            &self.label_url(request.direction),
            bearer,
            Some(USPS_LABELS_ACCEPT),
            &body,
        )?;
        check_ok(&response)?;
        let json: Value = serde_json::from_slice(&response.body)
            .map_err(|e| LabelError::BadResponse(e.to_string()))?;
        let tracking_no = find_tracking(&json).ok_or_else(|| {
            LabelError::BadResponse("no tracking number in usps answer".to_string())
        })?;
        let label_pdf = label_bytes(&json, &self.transport, bearer)?;
        let postage_cost_cents = find_cents(&json);
        Ok(BuyOutcome {
            tracking_no,
            label_pdf,
            postage_cost_cents,
            carrier_service: usps_mail_class(request.payload.service).to_string(),
        })
    }

    fn cancel(&self, bearer: &str, tracking_no: &str) -> Result<(), LabelError> {
        check_tracking(tracking_no)?;
        // USPS tracking numbers are commonly space-separated; the
        // path segment must be encoded (`check_tracking` already
        // restricted the charset to alnum/space/dash).
        let encoded = tracking_no.trim().replace(' ', "%20");
        let url = format!("{}/labels/v3/label/{encoded}", self.base_url);
        let response = self.transport.delete(&url, bearer)?;
        check_ok(&response)?;
        Ok(())
    }
}

/// FedEx Ship v1 client: `POST /ship/v1/shipments` plus cancel.
///
/// Returns use `PRINT_RETURN_LABEL` (we print both PDFs); pickup is
/// drop-off (no pickup scheduling in P2).
pub struct FedExClient<T: HttpTransport> {
    base_url: String,
    transport: T,
    certified: bool,
}

impl<T: HttpTransport> FedExClient<T> {
    pub fn new(base_url: &str, transport: T, certified: bool) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            transport,
            certified,
        }
    }
}

impl<T: HttpTransport> LabelClient for FedExClient<T> {
    fn carrier(&self) -> Carrier {
        Carrier::FedEx
    }

    fn ready(&self) -> bool {
        self.certified
    }

    fn buy(&self, bearer: &str, request: &BuyRequest) -> Result<BuyOutcome, LabelError> {
        if !self.certified {
            return Err(LabelError::NotReady);
        }
        check_payment_ref(request.payment_ref)?;
        let body = fedex_ship_body(request);
        let url = format!("{}/ship/v1/shipments", self.base_url);
        let response = self.transport.post_json(&url, bearer, None, &body)?;
        check_ok(&response)?;
        let json: Value = serde_json::from_slice(&response.body)
            .map_err(|e| LabelError::BadResponse(e.to_string()))?;
        let tracking_no = find_tracking(&json).ok_or_else(|| {
            LabelError::BadResponse("no tracking number in fedex answer".to_string())
        })?;
        let label_pdf = label_bytes(&json, &self.transport, bearer)?;
        let postage_cost_cents = find_cents(&json);
        Ok(BuyOutcome {
            tracking_no,
            label_pdf,
            postage_cost_cents,
            carrier_service: fedex_service_type(request.payload.service).to_string(),
        })
    }

    fn cancel(&self, bearer: &str, tracking_no: &str) -> Result<(), LabelError> {
        // Cancel method (PUT vs POST) is unverified from public docs —
        // confirm in the spec before the first real cancel.
        check_tracking(tracking_no)?;
        let url = format!("{}/ship/v1/shipments/cancel", self.base_url);
        let body = json!({ "trackingNumber": tracking_no });
        let response = self.transport.post_json(&url, bearer, None, &body)?;
        check_ok(&response)?;
        Ok(())
    }
}

/// USPS Labels v3 label request body. Field names follow the public
/// migration guide + catalog; verify against the v3 spec at
/// integration (sandbox) time.
pub fn usps_label_body(request: &BuyRequest) -> Value {
    usps_label_body_with_paper(request, crate::PaperSize::FourBySix)
}

/// [`usps_label_body`] with an explicit prepaid paper size (Option B).
/// The default body keeps 4x6 so existing callers are unaffected.
pub fn usps_label_body_with_paper(request: &BuyRequest, paper: crate::PaperSize) -> Value {
    let payload = request.payload;
    let dims = payload.parcel.dims();
    json!({
        "fromAddress": usps_address(&payload.from),
        "toAddress": usps_address(&payload.to),
        "packageDescription": {
            // USPS bills in ounces/pounds; grams convert up (ceil).
            "weightLbs": 0,
            "weightOz": dims.weight_g.div_ceil(28),
            "lengthIn": (dims.length_cm / 2.54).ceil() as u32,
            "widthIn": (dims.width_cm / 2.54).ceil() as u32,
            "heightIn": (dims.height_cm / 2.54).ceil() as u32,
            "mailClass": usps_mail_class(payload.service),
        },
        "labelSpecification": {
            "imageType": "PDF",
            "paperSize": paper.usps_label_option(),
        },
        "paymentAuthorizationToken": request.payment_ref,
    })
}

fn usps_address(address: &crate::PostalAddress) -> Value {
    json!({
        "name": address.name,
        "streetAddress": address.street1,
        "secondaryAddress": address.street2,
        "city": address.city,
        "state": address.state_province,
        "zipCode": address.postal,
        "country": address.country,
        "phone": address.phone,
    })
}

/// FedEx Ship v1 shipment body. Enum spellings follow the public Ship
/// docs + guides; verify `pickupType`, `labelStockType`, and the
/// return-detail shape in the spec at integration time.
pub fn fedex_ship_body(request: &BuyRequest) -> Value {
    let payload = request.payload;
    let dims = payload.parcel.dims();
    let mut shipment: serde_json::Map<String, Value> = serde_json::Map::from_iter([
        (
            "shipper".to_string(),
            json!({
                "contact": fedex_contact(&payload.from),
                "address": fedex_address(&payload.from),
            }),
        ),
        (
            "recipient".to_string(),
            json!({
                "contact": fedex_contact(&payload.to),
                "address": fedex_address(&payload.to),
            }),
        ),
        (
            "serviceType".to_string(),
            Value::String(fedex_service_type(payload.service).to_string()),
        ),
        (
            "packagingType".to_string(),
            Value::String(
                if payload.parcel.is_envelope() {
                    "FEDEX_ENVELOPE"
                } else {
                    "YOUR_PACKAGING"
                }
                .to_string(),
            ),
        ),
        (
            "pickupType".to_string(),
            Value::String("DROPOFF_AT_FEDEX_LOCATION".to_string()),
        ),
        (
            "shippingChargesPayment".to_string(),
            json!({
                "paymentType": "SENDER",
                "payor": { "responsibleParty": { "accountNumber": request.payment_ref } },
            }),
        ),
        (
            "labelSpecification".to_string(),
            json!({ "imageType": "PDF", "labelStockType": "STOCK_4X6" }),
        ),
        (
            "requestedPackageLineItems".to_string(),
            json!([{
                // FedEx bills in lb/oz; grams convert up (ceil).
                "weight": { "units": "LB", "value": dims.weight_g.div_ceil(454).max(1) },
                "dimensions": {
                    "length": (dims.length_cm / 2.54).ceil() as u32,
                    "width": (dims.width_cm / 2.54).ceil() as u32,
                    "height": (dims.height_cm / 2.54).ceil() as u32,
                    "units": "IN",
                },
            }]),
        ),
    ]);
    if request.direction == Direction::Return {
        shipment.insert(
            "returnShipmentDetail".to_string(),
            json!({ "returnType": "PRINT_RETURN_LABEL" }),
        );
    }
    Value::Object(serde_json::Map::from_iter([(
        "requestedShipment".to_string(),
        Value::Object(shipment),
    )]))
}

fn fedex_contact(address: &crate::PostalAddress) -> Value {
    json!({ "personName": address.name, "phoneNumber": address.phone })
}

fn fedex_address(address: &crate::PostalAddress) -> Value {
    json!({
        "streetLines": [address.street1],
        "city": address.city,
        "stateOrProvinceCode": address.state_province,
        "postalCode": address.postal,
        "countryCode": address.country,
    })
}

fn check_payment_ref(payment_ref: &str) -> Result<(), LabelError> {
    if payment_ref.trim().is_empty() {
        return Err(LabelError::EmptyPaymentRef);
    }
    Ok(())
}

fn check_tracking(tracking_no: &str) -> Result<(), LabelError> {
    let trimmed = tracking_no.trim();
    if trimmed.is_empty()
        || trimmed.len() > 64
        || !trimmed
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b' ' || b == b'-')
    {
        return Err(LabelError::BadTrackingNo);
    }
    Ok(())
}

fn check_ok(response: &HttpResponse) -> Result<(), LabelError> {
    if (200..300).contains(&response.status) {
        return Ok(());
    }
    let text = String::from_utf8_lossy(&response.body);
    let clipped: String = text.chars().take(300).collect();
    Err(LabelError::Api {
        status: response.status,
        message: clipped,
    })
}

/// Tracking number from a carrier answer. Both carriers nest it
/// differently, so walk the documented candidate paths.
fn find_tracking(json: &Value) -> Option<String> {
    for path in [
        "trackingNumber",
        "tracking_number",
        "trackingNo",
        "output/transactionShipments/0/completedShipmentDetail/completedPackageDetails/0/trackingIds/0/trackingNumber",
        "output/transactionShipments/0/masterTrackingNumber",
        "label/trackingNumber",
    ] {
        if let Some(hit) = walk(json, path)
            && let Value::String(number) = hit
            && !number.trim().is_empty()
        {
            return Some(number.trim().to_string());
        }
    }
    None
}

/// Label PDF bytes: base64 embedded in the answer, else fetched from
/// the carrier's label URL with the same bearer.
fn label_bytes<T: HttpTransport>(
    json: &Value,
    transport: &T,
    bearer: &str,
) -> Result<Vec<u8>, LabelError> {
    for path in [
        "labelImage",
        "label_image",
        "label/base64",
        "output/transactionShipments/0/completedShipmentDetail/completedPackageDetails/0/packageDocuments/0/parts/0/image",
        "output/transactionShipments/0/pieceResponses/0/label",
    ] {
        if let Some(Value::String(encoded)) = walk(json, path) {
            let bytes = decode_label(encoded)?;
            if bytes.starts_with(b"%PDF-") {
                return Ok(bytes);
            }
            return Err(LabelError::BadResponse(
                "carrier label is not a pdf".to_string(),
            ));
        }
    }
    for path in ["labelUrl", "label_url", "label/url"] {
        if let Some(Value::String(url)) = walk(json, path) {
            let response = transport.get(url, bearer)?;
            check_ok(&response)?;
            if response.body.starts_with(b"%PDF-") {
                return Ok(response.body);
            }
            return Err(LabelError::BadResponse(
                "carrier label download is not a pdf".to_string(),
            ));
        }
    }
    Err(LabelError::BadResponse(
        "no label image in carrier answer".to_string(),
    ))
}

fn decode_label(encoded: &str) -> Result<Vec<u8>, LabelError> {
    use base64::Engine as _;
    // Whitespace-tolerant: some carriers wrap base64 at 76 columns.
    let compact: String = encoded.chars().filter(|c| !c.is_whitespace()).collect();
    base64::engine::general_purpose::STANDARD
        .decode(compact.as_bytes())
        .map_err(|_| LabelError::BadResponse("label base64 does not decode".to_string()))
}

/// Postage in cents when the carrier reports it (recorded, P2-L4).
fn find_cents(json: &Value) -> Option<i64> {
    for path in [
        "postageCents",
        "postage_cents",
        "totalAmountCents",
        "output/transactionShipments/0/completedShipmentDetail/shipmentRating/shipmentRateDetails/0/totalNetCharge",
    ] {
        if let Some(Value::Number(n)) = walk(json, path)
            && let Some(cents) = n.as_i64()
        {
            return Some(cents);
        }
    }
    None
}

/// Walk `a/b/0/c` through objects (by key) and arrays (by index).
fn walk<'a>(json: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = json;
    for step in path.split('/') {
        if let Ok(index) = step.parse::<usize>() {
            current = current.as_array()?.get(index)?;
        } else {
            current = current.as_object()?.get(step)?;
        }
    }
    Some(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    pub(crate) struct StubTransport {
        pub(crate) response: RefCell<HttpResponse>,
        pub(crate) last_url: RefCell<String>,
        pub(crate) last_bearer: RefCell<String>,
        pub(crate) last_accept: RefCell<Option<String>>,
        pub(crate) last_body: RefCell<Value>,
    }

    impl StubTransport {
        pub(crate) fn canned(status: u16, body: &[u8]) -> Self {
            Self {
                response: RefCell::new(HttpResponse {
                    status,
                    body: body.to_vec(),
                }),
                last_url: RefCell::new(String::new()),
                last_bearer: RefCell::new(String::new()),
                last_accept: RefCell::new(None),
                last_body: RefCell::new(Value::Null),
            }
        }
    }

    impl HttpTransport for StubTransport {
        fn post_json(
            &self,
            url: &str,
            bearer: &str,
            accept: Option<&str>,
            body: &Value,
        ) -> Result<HttpResponse, TransportError> {
            *self.last_url.borrow_mut() = url.to_string();
            *self.last_bearer.borrow_mut() = bearer.to_string();
            *self.last_accept.borrow_mut() = accept.map(str::to_string);
            *self.last_body.borrow_mut() = body.clone();
            Ok(self.response.borrow().clone())
        }

        fn get(&self, url: &str, bearer: &str) -> Result<HttpResponse, TransportError> {
            *self.last_url.borrow_mut() = url.to_string();
            *self.last_bearer.borrow_mut() = bearer.to_string();
            Ok(self.response.borrow().clone())
        }

        fn delete(&self, url: &str, bearer: &str) -> Result<HttpResponse, TransportError> {
            *self.last_url.borrow_mut() = url.to_string();
            *self.last_bearer.borrow_mut() = bearer.to_string();
            Ok(self.response.borrow().clone())
        }
    }

    fn usps_answer() -> Vec<u8> {
        use base64::Engine as _;
        let pdf = b"%PDF-1.4 synthetic label";
        let encoded = base64::engine::general_purpose::STANDARD.encode(pdf);
        serde_json::to_vec(&json!({
            "trackingNumber": "9405 5000 0000 0000 0000 00",
            "labelImage": encoded,
            "postageCents": 1250,
        }))
        .unwrap()
    }

    #[test]
    fn usps_buy_posts_label_endpoint_with_json_accept() {
        let transport = StubTransport::canned(200, &usps_answer());
        let client = UspsClient::new("https://apis-tem.usps.com/", transport, true);
        let payload = crate::payload::tests::sample_payload();
        let request = BuyRequest {
            payload: &payload,
            direction: Direction::Outbound,
            payment_ref: "pay-synthetic",
        };
        let outcome = client.buy("bearer-synthetic", &request).unwrap();
        assert_eq!(outcome.tracking_no, "9405 5000 0000 0000 0000 00");
        assert!(outcome.label_pdf.starts_with(b"%PDF-"));
        assert_eq!(outcome.postage_cost_cents, Some(1250));
        assert_eq!(outcome.carrier_service, "PRIORITY");
        assert_eq!(
            *client.transport.last_url.borrow(),
            "https://apis-tem.usps.com/labels/v3/label"
        );
        assert_eq!(
            *client.transport.last_accept.borrow(),
            Some(USPS_LABELS_ACCEPT.to_string())
        );
        let body = client.transport.last_body.borrow();
        assert_eq!(body["packageDescription"]["mailClass"], json!("PRIORITY"));
        assert_eq!(body["paymentAuthorizationToken"], json!("pay-synthetic"));
    }

    #[test]
    fn usps_return_uses_return_endpoint() {
        let transport = StubTransport::canned(200, &usps_answer());
        let client = UspsClient::new(USPS_BASE_TEST, transport, true);
        let payload = crate::payload::tests::sample_payload();
        let request = BuyRequest {
            payload: &payload,
            direction: Direction::Return,
            payment_ref: "pay-synthetic",
        };
        client.buy("bearer-synthetic", &request).unwrap();
        assert_eq!(
            *client.transport.last_url.borrow(),
            "https://apis-tem.usps.com/labels/v3/returnlabel"
        );
    }

    #[test]
    fn usps_rejection_surfaces_status() {
        let transport = StubTransport::canned(400, b"{\"error\":\"bad address\"}");
        let client = UspsClient::new(USPS_BASE_TEST, transport, true);
        let payload = crate::payload::tests::sample_payload();
        let request = BuyRequest {
            payload: &payload,
            direction: Direction::Outbound,
            payment_ref: "pay-synthetic",
        };
        let error = client.buy("bearer-synthetic", &request).unwrap_err();
        assert!(matches!(error, LabelError::Api { status: 400, .. }));
        assert!(!error.to_string().contains("pay-synthetic"));
    }

    #[test]
    fn fedex_buy_posts_shipments_with_return_detail() {
        use base64::Engine as _;
        let pdf = b"%PDF-1.4 synthetic fedex label";
        let encoded = base64::engine::general_purpose::STANDARD.encode(pdf);
        let answer = serde_json::to_vec(&json!({
            "output": { "transactionShipments": [{
                "masterTrackingNumber": "123456789012",
                "pieceResponses": [{ "label": encoded }],
            }]},
        }))
        .unwrap();
        let transport = StubTransport::canned(200, &answer);
        let client = FedExClient::new(FEDEX_BASE_SANDBOX, transport, true);
        let payload = crate::payload::tests::sample_payload();
        let request = BuyRequest {
            payload: &payload,
            direction: Direction::Return,
            payment_ref: "acct-synthetic",
        };
        let outcome = client.buy("bearer-synthetic", &request).unwrap();
        assert_eq!(outcome.tracking_no, "123456789012");
        assert!(outcome.label_pdf.starts_with(b"%PDF-"));
        assert_eq!(outcome.carrier_service, "FEDEX_2_DAY");
        assert_eq!(
            *client.transport.last_url.borrow(),
            "https://apis-sandbox.fedex.com/ship/v1/shipments"
        );
        let body = client.transport.last_body.borrow();
        let shipment = &body["requestedShipment"];
        assert_eq!(shipment["serviceType"], json!("FEDEX_2_DAY"));
        assert_eq!(shipment["packagingType"], json!("FEDEX_ENVELOPE"));
        assert_eq!(
            shipment["returnShipmentDetail"]["returnType"],
            json!("PRINT_RETURN_LABEL")
        );
    }

    #[test]
    fn uncertified_clients_refuse() {
        let client = UspsClient::new(USPS_BASE_TEST, StubTransport::canned(200, b"{}"), false);
        let payload = crate::payload::tests::sample_payload();
        let request = BuyRequest {
            payload: &payload,
            direction: Direction::Outbound,
            payment_ref: "pay",
        };
        assert!(matches!(
            client.buy("b", &request).unwrap_err(),
            LabelError::NotReady
        ));
    }

    #[test]
    fn cancel_validates_tracking() {
        let client = UspsClient::new(USPS_BASE_TEST, StubTransport::canned(200, b"{}"), true);
        assert!(matches!(
            client.cancel("b", "").unwrap_err(),
            LabelError::BadTrackingNo
        ));
        client.cancel("b", "9405 5000 0000 0000 0000 00").unwrap();
        assert!(
            client
                .transport
                .last_url
                .borrow()
                .contains("/labels/v3/label/9405%205000"),
            "spaced tracking must be encoded: {}",
            client.transport.last_url.borrow()
        );
    }

    #[test]
    fn with_paper_reaches_the_wire() {
        let transport = StubTransport::canned(200, &usps_answer());
        let client =
            UspsClient::new(USPS_BASE_TEST, transport, true).with_paper(crate::PaperSize::Letter);
        let payload = crate::payload::tests::sample_payload();
        let request = BuyRequest {
            payload: &payload,
            direction: Direction::Outbound,
            payment_ref: "pay-synthetic",
        };
        client.buy("bearer-synthetic", &request).unwrap();
        let body = client.transport.last_body.borrow();
        assert_eq!(body["labelSpecification"]["paperSize"], json!("LETTER"));
    }

    /// The live transport against a stub HTTP server on an ephemeral
    /// port (`port 0`): proves the real request path — method, auth
    /// header, accept header, JSON body — without touching carriers.
    #[test]
    fn live_transport_round_trip_on_ephemeral_port() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let answer = usps_answer();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_http_request(&mut stream);
            let body = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                answer.len()
            );
            use std::io::Write as _;
            stream.write_all(body.as_bytes()).unwrap();
            stream.write_all(&answer).unwrap();
            stream.flush().unwrap();
            request
        });
        let transport = UreqTransport::new();
        let client = UspsClient::new(&format!("http://127.0.0.1:{port}"), transport, true);
        let payload = crate::payload::tests::sample_payload();
        let request = BuyRequest {
            payload: &payload,
            direction: Direction::Outbound,
            payment_ref: "pay-synthetic",
        };
        let outcome = client.buy("bearer-synthetic", &request).unwrap();
        assert_eq!(outcome.tracking_no, "9405 5000 0000 0000 0000 00");
        let seen = server.join().unwrap();
        let head_lower = seen.to_lowercase();
        assert!(
            seen.starts_with("POST /labels/v3/label HTTP/1.1\r\n"),
            "{seen}"
        );
        assert!(
            head_lower.contains("authorization: bearer bearer-synthetic\r\n"),
            "{seen}"
        );
        assert!(
            head_lower.contains(&format!(
                "accept: {}\r\n",
                USPS_LABELS_ACCEPT.to_lowercase()
            )),
            "{seen}"
        );
        let wire_body = seen.split("\r\n\r\n").nth(1).unwrap_or("");
        let wire_json: serde_json::Value = serde_json::from_str(wire_body).unwrap();
        assert_eq!(
            wire_json["packageDescription"]["mailClass"],
            serde_json::json!("PRIORITY"),
            "{seen}"
        );
    }

    fn read_http_request(stream: &mut std::net::TcpStream) -> String {
        use std::io::Read as _;
        let mut raw = Vec::new();
        let mut chunk = [0u8; 1024];
        // Headers first, then exactly Content-Length body bytes.
        loop {
            let n = stream.read(&mut chunk).unwrap();
            raw.extend_from_slice(&chunk[..n]);
            if raw.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let head = String::from_utf8_lossy(&raw).to_string();
        let len: usize = head
            .lines()
            .find_map(|line| {
                line.strip_prefix("content-length:")
                    .or_else(|| line.strip_prefix("Content-Length:"))
                    .and_then(|v| v.trim().parse().ok())
            })
            .unwrap_or(0);
        let body_start = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        while raw.len() < body_start + len {
            let n = stream.read(&mut chunk).unwrap();
            raw.extend_from_slice(&chunk[..n]);
        }
        String::from_utf8_lossy(&raw).to_string()
    }
}
