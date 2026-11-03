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

    /// POST with extra request headers (USPS reads its payment token
    /// from `X-Payment-Authorization-Token`). A transport that cannot
    /// set headers refuses rather than silently dropping one.
    fn post_json_with(
        &self,
        url: &str,
        bearer: &str,
        accept: Option<&str>,
        headers: &[(&str, &str)],
        body: &Value,
    ) -> Result<HttpResponse, TransportError> {
        if headers.is_empty() {
            self.post_json(url, bearer, accept, body)
        } else {
            Err(TransportError::new(
                "this transport cannot send extra headers",
            ))
        }
    }

    /// DELETE with extra request headers (see [`Self::post_json_with`]).
    fn delete_with(
        &self,
        url: &str,
        bearer: &str,
        headers: &[(&str, &str)],
    ) -> Result<HttpResponse, TransportError> {
        if headers.is_empty() {
            self.delete(url, bearer)
        } else {
            Err(TransportError::new(
                "this transport cannot send extra headers",
            ))
        }
    }

    /// PUT a JSON body (FedEx cancels a shipment with PUT). A
    /// transport that cannot PUT refuses rather than sending a POST.
    fn put_json(
        &self,
        _url: &str,
        _bearer: &str,
        _body: &Value,
    ) -> Result<HttpResponse, TransportError> {
        Err(TransportError::new("this transport cannot send PUT"))
    }
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
        self.delete_with(url, bearer, &[])
    }

    fn post_json_with(
        &self,
        url: &str,
        bearer: &str,
        accept: Option<&str>,
        headers: &[(&str, &str)],
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
        for (name, value) in headers {
            request = request.header(*name, value.to_string());
        }
        let response = request.send_json(body).map_err(TransportError::from_ureq)?;
        finish(response)
    }

    fn delete_with(
        &self,
        url: &str,
        bearer: &str,
        headers: &[(&str, &str)],
    ) -> Result<HttpResponse, TransportError> {
        let mut request = self
            .agent
            .delete(url)
            .header("Authorization", format!("Bearer {bearer}"));
        for (name, value) in headers {
            request = request.header(*name, value.to_string());
        }
        let response = request.call().map_err(TransportError::from_ureq)?;
        finish(response)
    }

    fn put_json(
        &self,
        url: &str,
        bearer: &str,
        body: &Value,
    ) -> Result<HttpResponse, TransportError> {
        let response = self
            .agent
            .put(url)
            .header("Authorization", format!("Bearer {bearer}"))
            .header("Content-Type", "application/json")
            .send_json(body)
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
}

impl<T: HttpTransport> UspsClient<T> {
    /// USPS always images a 4x6 PDF (`imageInfo.labelType`); the paper a
    /// customer prints it on is recorded on our label row only.
    pub fn new(base_url: &str, transport: T, certified: bool) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            transport,
            certified,
        }
    }

    /// Cancel a label bought with `payment_token` (the label API reads
    /// the same payment header on DELETE; TEM: confirm).
    pub fn cancel_paid(
        &self,
        bearer: &str,
        payment_token: &str,
        tracking_no: &str,
    ) -> Result<(), LabelError> {
        check_tracking(tracking_no)?;
        let encoded = tracking_no.trim().replace(' ', "%20");
        let url = format!("{}/labels/v3/label/{encoded}", self.base_url);
        let headers = [(crate::usps::PAYMENT_HEADER, payment_token)];
        let response = self.transport.delete_with(&url, bearer, &headers)?;
        check_ok(&response)?;
        Ok(())
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
        let body = usps_label_body(request);
        let headers = [(crate::usps::PAYMENT_HEADER, request.payment_ref)];
        let response = self.transport.post_json_with(
            &self.label_url(request.direction),
            bearer,
            Some(USPS_LABELS_ACCEPT),
            &headers,
            &body,
        )?;
        check_ok(&response)?;
        let json: Value = serde_json::from_slice(&response.body)
            .map_err(|e| LabelError::BadResponse(e.to_string()))?;
        let (tracking, postage) = crate::usps::label_metadata(&json);
        let tracking_no = tracking.or_else(|| find_tracking(&json)).ok_or_else(|| {
            LabelError::BadResponse("no tracking number in usps answer".to_string())
        })?;
        let label_pdf = label_bytes(&json, &self.transport, bearer)?;
        let postage_cost_cents = postage.or_else(|| find_cents(&json));
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

/// FedEx Ship v1 client: `POST /ship/v1/shipments` plus
/// `PUT /ship/v1/shipments/cancel`.
///
/// Request shapes follow FedEx's published Ship API JSON
/// (<https://developer.fedex.com/api/en-us/catalog/ship/v1/docs.html>).
/// Returns use `PRINT_RETURN_LABEL` (we print both PDFs); pickup is
/// drop-off (no pickup scheduling in P2).
pub struct FedExClient<T: HttpTransport> {
    base_url: String,
    transport: T,
    certified: bool,
    /// Shipper account for cancel (a buy carries its own in
    /// [`BuyRequest::payment_ref`]). Server-side only, never logged.
    account: Option<String>,
}

impl<T: HttpTransport> FedExClient<T> {
    pub fn new(base_url: &str, transport: T, certified: bool) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            transport,
            certified,
            account: None,
        }
    }

    /// The account FedEx's cancel body names (`accountNumber.value`).
    pub fn with_account(mut self, account: &str) -> Self {
        self.account = Some(account.trim().to_string());
        self
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
        let postage_cost_cents = fedex_net_charge_cents(&json);
        Ok(BuyOutcome {
            tracking_no,
            label_pdf,
            postage_cost_cents,
            carrier_service: fedex_service_type(request.payload.service).to_string(),
        })
    }

    /// Ship API "Cancel Shipment": `PUT /ship/v1/shipments/cancel`
    /// (<https://developer.fedex.com/api/en-us/catalog/ship/v1/docs.html>,
    /// section Cancel Shipment). The account is required in the body.
    fn cancel(&self, bearer: &str, tracking_no: &str) -> Result<(), LabelError> {
        check_tracking(tracking_no)?;
        let account = self.account.as_deref().unwrap_or_default();
        check_payment_ref(account)?;
        let url = format!("{}/ship/v1/shipments/cancel", self.base_url);
        let body = fedex_cancel_body(account, tracking_no);
        let response = self.transport.put_json(&url, bearer, &body)?;
        check_ok(&response)?;
        // A 200 can still say no: `output.cancelledShipment` is false
        // when FedEx kept the shipment (already tendered, say).
        let json: Value = serde_json::from_slice(&response.body).unwrap_or(Value::Null);
        if walk(&json, "output/cancelledShipment") == Some(&Value::Bool(false)) {
            return Err(LabelError::Api {
                status: response.status,
                message: "fedex did not cancel the shipment".to_string(),
            });
        }
        Ok(())
    }
}

/// Cancel Shipment body (Ship API): account, tracking number, and
/// `DELETE_ALL_PACKAGES` so a multi-piece shipment voids whole.
pub fn fedex_cancel_body(account: &str, tracking_no: &str) -> Value {
    json!({
        "accountNumber": { "value": account.trim() },
        "emailShipment": false,
        "senderCountryCode": "US",
        "deletionControl": "DELETE_ALL_PACKAGES",
        "trackingNumber": tracking_no.trim(),
    })
}

/// USPS Labels v3 label request body ([`crate::usps::label_body`]),
/// mailed today. The payment token travels as a header, never here.
pub fn usps_label_body(request: &BuyRequest) -> Value {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    crate::usps::label_body(request.payload, request.direction, now)
}

/// FedEx Ship v1 "Create Shipment" body, field by field against the
/// published Ship API JSON
/// (<https://developer.fedex.com/api/en-us/catalog/ship/v1/docs.html>):
///
/// - top level: `accountNumber.value` (the shipper account billed) and
///   `labelResponseOptions: "LABEL"` (the PDF comes back base64 in
///   `pieceResponses[].packageDocuments[].encodedLabel`, no second
///   fetch);
/// - `requestedShipment.recipients` is an array (Rate uses a single
///   `recipient`); both contacts carry `phoneNumber`, which FedEx
///   requires;
/// - `shippingChargesPayment.payor.responsibleParty.accountNumber` is
///   an object `{ "value": … }`, never a bare string;
/// - `labelSpecification`: PDF on `PAPER_4X6` (the `STOCK_*` stocks
///   are for thermal printers).
pub fn fedex_ship_body(request: &BuyRequest) -> Value {
    let payload = request.payload;
    let account = request.payment_ref.trim();
    let packaging = fedex_packaging_type(payload);
    let mut shipment: serde_json::Map<String, Value> = serde_json::Map::from_iter([
        (
            "shipper".to_string(),
            json!({
                "contact": fedex_contact(&payload.from),
                "address": fedex_address(&payload.from),
            }),
        ),
        (
            "recipients".to_string(),
            json!([{
                "contact": fedex_contact(&payload.to),
                "address": fedex_address(&payload.to),
            }]),
        ),
        (
            "serviceType".to_string(),
            Value::String(fedex_service_type(payload.service).to_string()),
        ),
        (
            "packagingType".to_string(),
            Value::String(packaging.to_string()),
        ),
        (
            "pickupType".to_string(),
            Value::String("DROPOFF_AT_FEDEX_LOCATION".to_string()),
        ),
        (
            "shippingChargesPayment".to_string(),
            json!({
                "paymentType": "SENDER",
                "payor": { "responsibleParty": { "accountNumber": { "value": account } } },
            }),
        ),
        (
            "labelSpecification".to_string(),
            json!({ "imageType": "PDF", "labelStockType": "PAPER_4X6" }),
        ),
        (
            "requestedPackageLineItems".to_string(),
            json!([fedex_package_line(payload)]),
        ),
    ]);
    if request.direction == Direction::Return {
        shipment.insert(
            "returnShipmentDetail".to_string(),
            json!({ "returnType": "PRINT_RETURN_LABEL" }),
        );
    }
    json!({
        "labelResponseOptions": "LABEL",
        "accountNumber": { "value": account },
        "requestedShipment": Value::Object(shipment),
    })
}

/// FedEx `packagingType`: envelope presets ship in `FEDEX_ENVELOPE`,
/// except on FedEx Ground, which takes only the customer's own
/// packaging (`YOUR_PACKAGING`).
pub(crate) fn fedex_packaging_type(payload: &ShipmentPayload) -> &'static str {
    if payload.parcel.is_envelope() && payload.service != crate::ServiceClass::Ground {
        "FEDEX_ENVELOPE"
    } else {
        "YOUR_PACKAGING"
    }
}

/// One `requestedPackageLineItems` entry. FedEx bills in whole pounds
/// here (grams round up); dimensions are required only for
/// `YOUR_PACKAGING` and must then carry all three sides.
pub(crate) fn fedex_package_line(payload: &ShipmentPayload) -> Value {
    let dims = payload.parcel.dims();
    let weight = json!({ "units": "LB", "value": dims.weight_g.div_ceil(454).max(1) });
    if fedex_packaging_type(payload) == "FEDEX_ENVELOPE" {
        return json!({ "weight": weight });
    }
    json!({
        "weight": weight,
        "dimensions": {
            "length": (dims.length_cm / 2.54).ceil() as u32,
            "width": (dims.width_cm / 2.54).ceil() as u32,
            "height": (dims.height_cm / 2.54).ceil() as u32,
            "units": "IN",
        },
    })
}

fn fedex_contact(address: &crate::PostalAddress) -> Value {
    json!({
        "personName": address.name.trim(),
        "phoneNumber": address
            .phone
            .as_deref()
            .map(|p| fedex_phone(p, &address.country))
            .unwrap_or_default(),
    })
}

/// FedEx wants bare digits; US and CA numbers are exactly 10, so a
/// leading `+1`/`1` country code is dropped there.
pub fn fedex_phone(raw: &str, country: &str) -> String {
    let digits: String = raw.chars().filter(char::is_ascii_digit).collect();
    let nanp = matches!(country.trim(), "US" | "CA");
    match digits.strip_prefix('1') {
        Some(rest) if nanp && digits.len() == 11 => rest.to_string(),
        _ => digits,
    }
}

fn fedex_address(address: &crate::PostalAddress) -> Value {
    let mut lines = vec![address.street1.trim().to_string()];
    if let Some(street2) = address.street2.as_deref().map(str::trim)
        && !street2.is_empty()
    {
        lines.push(street2.to_string());
    }
    json!({
        "streetLines": lines,
        "city": address.city.trim(),
        "stateOrProvinceCode": address.state_province.trim(),
        "postalCode": address.postal.trim(),
        "countryCode": address.country.trim(),
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
        "output/transactionShipments/0/pieceResponses/0/trackingNumber",
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
        // FedEx Ship API with `labelResponseOptions: LABEL`.
        "output/transactionShipments/0/pieceResponses/0/packageDocuments/0/encodedLabel",
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
            // The purchase already answered 2xx: a failed download must
            // never read as the carrier refusing the shipment.
            if !(200..300).contains(&response.status) {
                return Err(LabelError::BadResponse(format!(
                    "label download answered http {} after the purchase",
                    response.status
                )));
            }
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
    for path in ["postageCents", "postage_cents", "totalAmountCents"] {
        if let Some(Value::Number(n)) = walk(json, path)
            && let Some(cents) = n.as_i64()
        {
            return Some(cents);
        }
    }
    None
}

/// FedEx Ship charge in cents. FedEx reports decimal dollars as JSON
/// numbers (`"totalNetCharge": 9.45`), never cents.
fn fedex_net_charge_cents(json: &Value) -> Option<i64> {
    [
        concat!(
            "output/transactionShipments/0/completedShipmentDetail/",
            "shipmentRating/shipmentRateDetails/0/totalNetCharge"
        ),
        "output/transactionShipments/0/pieceResponses/0/netChargeAmount",
    ]
    .into_iter()
    .find_map(|path| walk(json, path).and_then(dollars_to_cents))
}

/// Decimal dollars (JSON number or numeric string) to whole cents,
/// rounded. Negative or non-numeric amounts are no amount.
pub fn dollars_to_cents(value: &Value) -> Option<i64> {
    let dollars = match value {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => s.trim().parse::<f64>().ok()?,
        _ => return None,
    };
    if !dollars.is_finite() || dollars < 0.0 {
        return None;
    }
    Some((dollars * 100.0).round() as i64)
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
pub(crate) mod tests {
    use super::*;
    use std::cell::RefCell;

    pub(crate) struct StubTransport {
        pub(crate) response: RefCell<HttpResponse>,
        pub(crate) last_url: RefCell<String>,
        pub(crate) last_bearer: RefCell<String>,
        pub(crate) last_accept: RefCell<Option<String>>,
        pub(crate) last_body: RefCell<Value>,
        pub(crate) last_headers: RefCell<Vec<(String, String)>>,
        pub(crate) last_method: RefCell<String>,
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
                last_headers: RefCell::new(Vec::new()),
                last_method: RefCell::new(String::new()),
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
            *self.last_method.borrow_mut() = "POST".to_string();
            *self.last_url.borrow_mut() = url.to_string();
            *self.last_bearer.borrow_mut() = bearer.to_string();
            *self.last_accept.borrow_mut() = accept.map(str::to_string);
            *self.last_body.borrow_mut() = body.clone();
            Ok(self.response.borrow().clone())
        }

        fn put_json(
            &self,
            url: &str,
            bearer: &str,
            body: &Value,
        ) -> Result<HttpResponse, TransportError> {
            *self.last_method.borrow_mut() = "PUT".to_string();
            *self.last_url.borrow_mut() = url.to_string();
            *self.last_bearer.borrow_mut() = bearer.to_string();
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

        fn post_json_with(
            &self,
            url: &str,
            bearer: &str,
            accept: Option<&str>,
            headers: &[(&str, &str)],
            body: &Value,
        ) -> Result<HttpResponse, TransportError> {
            *self.last_headers.borrow_mut() = headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            self.post_json(url, bearer, accept, body)
        }

        fn delete_with(
            &self,
            url: &str,
            bearer: &str,
            headers: &[(&str, &str)],
        ) -> Result<HttpResponse, TransportError> {
            *self.last_headers.borrow_mut() = headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            self.delete(url, bearer)
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
        assert_eq!(
            body["packageDescription"]["mailClass"],
            json!("PRIORITY_MAIL")
        );
        assert_eq!(body["imageInfo"]["labelType"], json!("4X6LABEL"));
        // The payment token is a header, never in the body (shipping-design §5.1).
        assert!(body.get("paymentAuthorizationToken").is_none());
        assert_eq!(
            *client.transport.last_headers.borrow(),
            vec![(
                "X-Payment-Authorization-Token".to_string(),
                "pay-synthetic".to_string()
            )]
        );
    }

    #[test]
    fn usps_v3_label_metadata_answers_parse() {
        use base64::Engine as _;
        let encoded = base64::engine::general_purpose::STANDARD.encode(b"%PDF-1.4 synthetic v3");
        let answer = serde_json::to_vec(&json!({
            "labelMetadata": {"trackingNumber": "9405 5111 0000 0000 0000 00", "postage": 9.85},
            "labelImage": encoded,
        }))
        .unwrap();
        let client = UspsClient::new(USPS_BASE_TEST, StubTransport::canned(200, &answer), true);
        let payload = crate::payload::tests::sample_payload();
        let request = BuyRequest {
            payload: &payload,
            direction: Direction::Outbound,
            payment_ref: "pay-synthetic",
        };
        let outcome = client.buy("bearer-synthetic", &request).unwrap();
        assert_eq!(outcome.tracking_no, "9405 5111 0000 0000 0000 00");
        assert_eq!(outcome.postage_cost_cents, Some(985));
        client
            .cancel_paid("bearer-synthetic", "pay-synthetic", "9405 5111")
            .unwrap();
        assert_eq!(
            *client.transport.last_url.borrow(),
            "https://apis-tem.usps.com/labels/v3/label/9405%205111"
        );
        assert_eq!(client.transport.last_headers.borrow().len(), 1);
    }

    #[test]
    fn a_failed_download_after_purchase_is_never_a_refusal() {
        // 2xx purchase that points at a label URL; the download answers 422.
        struct Split;
        impl HttpTransport for Split {
            fn post_json(
                &self,
                _: &str,
                _: &str,
                _: Option<&str>,
                _: &Value,
            ) -> Result<HttpResponse, TransportError> {
                Ok(HttpResponse {
                    status: 200,
                    body: br#"{"trackingNumber":"9405","labelUrl":"https://example.test/l.pdf"}"#
                        .to_vec(),
                })
            }
            fn get(&self, _: &str, _: &str) -> Result<HttpResponse, TransportError> {
                Ok(HttpResponse {
                    status: 422,
                    body: b"{}".to_vec(),
                })
            }
            fn delete(&self, _: &str, _: &str) -> Result<HttpResponse, TransportError> {
                Ok(HttpResponse {
                    status: 200,
                    body: Vec::new(),
                })
            }
        }
        let client = FedExClient::new(FEDEX_BASE_SANDBOX, Split, true);
        let payload = crate::payload::tests::sample_payload();
        let request = BuyRequest {
            payload: &payload,
            direction: Direction::Outbound,
            payment_ref: "acct-synthetic",
        };
        assert!(matches!(
            client.buy("b", &request),
            Err(LabelError::BadResponse(_))
        ));
    }

    #[test]
    fn a_transport_without_header_support_refuses_instead_of_dropping() {
        struct Plain;
        impl HttpTransport for Plain {
            fn post_json(
                &self,
                _: &str,
                _: &str,
                _: Option<&str>,
                _: &Value,
            ) -> Result<HttpResponse, TransportError> {
                Ok(HttpResponse {
                    status: 200,
                    body: Vec::new(),
                })
            }
            fn get(&self, _: &str, _: &str) -> Result<HttpResponse, TransportError> {
                Ok(HttpResponse {
                    status: 200,
                    body: Vec::new(),
                })
            }
            fn delete(&self, _: &str, _: &str) -> Result<HttpResponse, TransportError> {
                Ok(HttpResponse {
                    status: 200,
                    body: Vec::new(),
                })
            }
        }
        assert!(
            Plain
                .post_json_with("u", "b", None, &[("X", "y")], &Value::Null)
                .is_err()
        );
        assert!(
            Plain
                .post_json_with("u", "b", None, &[], &Value::Null)
                .is_ok()
        );
        assert!(Plain.delete_with("u", "b", &[("X", "y")]).is_err());
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

    /// FedEx's documented request and answer samples (see
    /// `fixtures/fedex/README.md` for the source of each file).
    pub(crate) mod fedex_fixtures {
        pub(crate) const SHIP_REQUEST: &str =
            include_str!("../fixtures/fedex/ship_request_documented.json");
        pub(crate) const RATE_REQUEST: &str =
            include_str!("../fixtures/fedex/rate_request_documented.json");
        pub(crate) const CANCEL_REQUEST: &str =
            include_str!("../fixtures/fedex/cancel_request_documented.json");
        pub(crate) const SHIP_REPLY: &str = include_str!("../fixtures/fedex/ship_reply.json");
        pub(crate) const RATE_REPLY: &str = include_str!("../fixtures/fedex/rate_reply.json");
        pub(crate) const CANCEL_REPLY: &str = include_str!("../fixtures/fedex/cancel_reply.json");
    }

    /// Every key path in `v` (`a.b[].c`), with the JSON kind of the
    /// value there: `object`, `array` or `scalar`. Arrays collapse to
    /// `[]` and merge their elements.
    pub(crate) fn shape(v: &Value) -> std::collections::BTreeMap<String, &'static str> {
        fn walk(v: &Value, path: &str, out: &mut std::collections::BTreeMap<String, &'static str>) {
            let kind = match v {
                Value::Object(_) => "object",
                Value::Array(_) => "array",
                _ => "scalar",
            };
            if !path.is_empty() {
                out.insert(path.to_string(), kind);
            }
            match v {
                Value::Object(map) => {
                    for (k, child) in map {
                        let next = if path.is_empty() {
                            k.clone()
                        } else {
                            format!("{path}.{k}")
                        };
                        walk(child, &next, out);
                    }
                }
                Value::Array(items) => {
                    for child in items {
                        walk(child, &format!("{path}[]"), out);
                    }
                }
                _ => {}
            }
        }
        let mut out = std::collections::BTreeMap::new();
        walk(v, "", &mut out);
        out
    }

    /// Every path of the documented sample is in ours with the same
    /// kind (object/array/scalar), except the listed optional fields
    /// we leave out on purpose. Scalars may differ in type: FedEx's
    /// samples send `phoneNumber`/`postalCode` as numbers where the
    /// schema says string.
    pub(crate) fn assert_covers(documented: &Value, ours: &Value, optional: &[&str]) {
        let ours = shape(ours);
        for (path, kind) in shape(documented) {
            if optional.contains(&path.as_str()) {
                continue;
            }
            assert_eq!(
                ours.get(&path),
                Some(&kind),
                "documented field {path} ({kind}) missing or reshaped"
            );
        }
    }

    /// Synthetic shipper account (FedEx's own sample value).
    const ACCOUNT: &str = "123456789";

    fn fedex_payload() -> crate::ShipmentPayload {
        let mut payload = crate::payload::tests::sample_payload();
        payload.to.phone = Some("360-555-0199".to_string());
        payload.to.street2 = Some("Suite 2".to_string());
        payload
    }

    #[test]
    fn fedex_ship_body_matches_the_documented_request() {
        let payload = fedex_payload();
        let request = BuyRequest {
            payload: &payload,
            direction: Direction::Outbound,
            payment_ref: ACCOUNT,
        };
        let body = fedex_ship_body(&request);
        let documented: Value = serde_json::from_str(fedex_fixtures::SHIP_REQUEST).unwrap();
        assert_covers(
            &documented,
            &body,
            &[
                "requestedShipment.shipDatestamp",
                "requestedShipment.blockInsightVisibility",
                "requestedShipment.shipper.contact.companyName",
                "requestedShipment.recipients[].contact.companyName",
            ],
        );
        // The fields P393 fixed, value by value.
        assert_eq!(body["accountNumber"], json!({ "value": "123456789" }));
        assert_eq!(body["labelResponseOptions"], json!("LABEL"));
        let shipment = &body["requestedShipment"];
        assert!(
            shipment.get("recipient").is_none(),
            "ship uses recipients[]"
        );
        assert_eq!(shipment["recipients"].as_array().unwrap().len(), 1);
        assert_eq!(
            shipment["shippingChargesPayment"],
            json!({
                "paymentType": "SENDER",
                "payor": { "responsibleParty": { "accountNumber": { "value": "123456789" } } },
            })
        );
        assert_eq!(
            shipment["shipper"]["contact"]["phoneNumber"],
            json!("5551234567")
        );
        assert_eq!(
            shipment["recipients"][0]["contact"]["phoneNumber"],
            json!("3605550199")
        );
        assert_eq!(
            shipment["recipients"][0]["address"]["streetLines"],
            json!(["PO Box 40234", "Suite 2"])
        );
        assert_eq!(
            shipment["labelSpecification"],
            json!({ "imageType": "PDF", "labelStockType": "PAPER_4X6" })
        );
        assert_eq!(shipment["pickupType"], json!("DROPOFF_AT_FEDEX_LOCATION"));
        assert_eq!(shipment["packagingType"], json!("FEDEX_ENVELOPE"));
        assert!(
            shipment["requestedPackageLineItems"][0]
                .get("dimensions")
                .is_none(),
            "FedEx packaging carries no dimensions"
        );
    }

    #[test]
    fn fedex_ground_never_ships_in_a_fedex_envelope() {
        let mut payload = fedex_payload();
        payload.service = crate::ServiceClass::Ground;
        let body = fedex_ship_body(&BuyRequest {
            payload: &payload,
            direction: Direction::Outbound,
            payment_ref: ACCOUNT,
        });
        let shipment = &body["requestedShipment"];
        assert_eq!(shipment["serviceType"], json!("FEDEX_GROUND"));
        assert_eq!(shipment["packagingType"], json!("YOUR_PACKAGING"));
        let dims = &shipment["requestedPackageLineItems"][0]["dimensions"];
        assert_eq!(dims["units"], json!("IN"));
        assert!(dims["length"].as_u64().unwrap() > 0);
    }

    #[test]
    fn fedex_phone_is_bare_digits_with_ten_in_nanp() {
        assert_eq!(fedex_phone("+1 (360) 555-0100", "US"), "3605550100");
        assert_eq!(fedex_phone("360.555.0100", "US"), "3605550100");
        assert_eq!(fedex_phone("1 416 555 0100", "CA"), "4165550100");
        assert_eq!(fedex_phone("+44 20 7946 0000", "GB"), "442079460000");
    }

    #[test]
    fn fedex_buy_reads_the_documented_answer() {
        let transport = StubTransport::canned(200, fedex_fixtures::SHIP_REPLY.as_bytes());
        let client = FedExClient::new(FEDEX_BASE_SANDBOX, transport, true);
        let payload = fedex_payload();
        let outcome = client
            .buy(
                "bearer-synthetic",
                &BuyRequest {
                    payload: &payload,
                    direction: Direction::Outbound,
                    payment_ref: ACCOUNT,
                },
            )
            .unwrap();
        assert_eq!(outcome.tracking_no, "794953535000");
        assert!(outcome.label_pdf.starts_with(b"%PDF-"));
        // `totalNetCharge` is decimal dollars, never cents.
        assert_eq!(outcome.postage_cost_cents, Some(1_908));
        assert_eq!(*client.transport.last_method.borrow(), "POST");
    }

    #[test]
    fn fedex_cancel_puts_the_documented_body() {
        let transport = StubTransport::canned(200, fedex_fixtures::CANCEL_REPLY.as_bytes());
        let client =
            FedExClient::new(FEDEX_BASE_SANDBOX, transport, true).with_account(" 123456789 ");
        client.cancel("bearer-synthetic", "794953535000").unwrap();
        assert_eq!(*client.transport.last_method.borrow(), "PUT");
        assert_eq!(
            *client.transport.last_url.borrow(),
            "https://apis-sandbox.fedex.com/ship/v1/shipments/cancel"
        );
        let body = client.transport.last_body.borrow().clone();
        let documented: Value = serde_json::from_str(fedex_fixtures::CANCEL_REQUEST).unwrap();
        assert_covers(&documented, &body, &[]);
        assert_eq!(body["accountNumber"], json!({ "value": "123456789" }));
        assert_eq!(body["trackingNumber"], json!("794953535000"));

        // No account: refused before any request.
        let bare = FedExClient::new(FEDEX_BASE_SANDBOX, StubTransport::canned(200, b"{}"), true);
        assert!(matches!(
            bare.cancel("b", "794953535000"),
            Err(LabelError::EmptyPaymentRef)
        ));
        assert_eq!(*bare.transport.last_method.borrow(), "");

        // A 200 that says it did not cancel is a refusal.
        let kept = FedExClient::new(
            FEDEX_BASE_SANDBOX,
            StubTransport::canned(200, br#"{"output":{"cancelledShipment":false}}"#),
            true,
        )
        .with_account("123456789");
        assert!(matches!(
            kept.cancel("b", "794953535000"),
            Err(LabelError::Api { status: 200, .. })
        ));
    }

    #[test]
    fn dollars_to_cents_rounds_numbers_and_strings() {
        assert_eq!(dollars_to_cents(&json!(26.6)), Some(2_660));
        assert_eq!(dollars_to_cents(&json!(19.08)), Some(1_908));
        assert_eq!(dollars_to_cents(&json!("12.50")), Some(1_250));
        assert_eq!(dollars_to_cents(&json!(12)), Some(1_200));
        assert_eq!(dollars_to_cents(&json!(-1.0)), None);
        assert_eq!(dollars_to_cents(&json!("n/a")), None);
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
            serde_json::json!("PRIORITY_MAIL"),
            "{seen}"
        );
        // The payment token reaches the wire as its header.
        assert!(
            head_lower.contains("x-payment-authorization-token: pay-synthetic\r\n"),
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
