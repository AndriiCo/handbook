//! Carrier-neutral label payloads (P2-L2, carrier-apis §3).
//!
//! Every field a carrier buy needs — addresses, parcel weight+dims,
//! service, return pairing — in one schema both clients consume.

use std::fmt;

use serde::{Deserialize, Serialize};

/// One end of a shipment (P2-L2 data needs).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostalAddress {
    pub name: String,
    pub street1: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub street2: Option<String>,
    pub city: String,
    pub state_province: String,
    pub postal: String,
    /// ISO 3166-1 alpha-2, uppercase (`US`). P2 is domestic USA;
    /// cross-border customs paperwork is P3.
    pub country: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone: Option<String>,
}

/// Parcel size + weight in metric units.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ParcelDims {
    pub length_cm: f32,
    pub width_cm: f32,
    pub height_cm: f32,
    pub weight_g: u32,
}

/// Letter/flat envelope preset + override (P2-L2).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "preset")]
#[non_exhaustive]
pub enum Parcel {
    LetterEnvelope,
    FlatEnvelope,
    Custom(ParcelDims),
}

impl Parcel {
    /// Concrete dims: presets expand to fixed values, custom passes
    /// through. Letter = a few sheets; flat = a thin bound set.
    pub fn dims(self) -> ParcelDims {
        match self {
            Parcel::LetterEnvelope => ParcelDims {
                length_cm: 24.0,
                width_cm: 12.0,
                height_cm: 1.0,
                weight_g: 100,
            },
            Parcel::FlatEnvelope => ParcelDims {
                length_cm: 30.0,
                width_cm: 23.0,
                height_cm: 2.0,
                weight_g: 300,
            },
            Parcel::Custom(dims) => dims,
        }
    }

    /// True for envelope presets (maps to `FEDEX_ENVELOPE`).
    pub fn is_envelope(self) -> bool {
        !matches!(self, Parcel::Custom(_))
    }
}

/// Carrier-neutral service tier (each client maps it to native codes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ServiceClass {
    Overnight,
    TwoDay,
    Ground,
}

/// Outbound (customer → authority) or return (authority → customer).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Direction {
    Outbound,
    Return,
}

/// One shippable unit: the carrier-neutral buy payload (P2-D3/D4).
///
/// `pairing_id` is our own shipment id linking the outbound and return
/// labels — neither carrier pairs them server-side (carrier-apis §1–2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShipmentPayload {
    pub from: PostalAddress,
    pub to: PostalAddress,
    pub parcel: Parcel,
    pub service: ServiceClass,
    pub pairing_id: String,
}

/// One payload defect: `{field, code, message}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayloadError {
    pub field: String,
    pub code: String,
    pub message: String,
}

impl PayloadError {
    fn new(field: &str, code: &str, message: String) -> Self {
        Self {
            field: field.to_string(),
            code: code.to_string(),
            message,
        }
    }
}

/// Validate one address: required lines present, country an
/// uppercase ISO-2 code, phone (when present) dialable.
pub fn validate_address(prefix: &str, address: &PostalAddress) -> Vec<PayloadError> {
    let mut errors = Vec::new();
    for (name, value) in [
        ("name", address.name.as_str()),
        ("street1", address.street1.as_str()),
        ("city", address.city.as_str()),
        ("state_province", address.state_province.as_str()),
        ("postal", address.postal.as_str()),
    ] {
        if value.trim().is_empty() {
            errors.push(PayloadError::new(
                &format!("{prefix}.{name}"),
                "required",
                format!("{prefix} {name} is required"),
            ));
        }
    }
    let country = address.country.trim();
    if country.len() != 2
        || !country.bytes().all(|b| b.is_ascii_uppercase())
        || !country.bytes().all(|b| b.is_ascii_alphabetic())
    {
        errors.push(PayloadError::new(
            &format!("{prefix}.country"),
            "country",
            format!("{prefix} country must be an uppercase ISO-2 code like US"),
        ));
    }
    if let Some(phone) = &address.phone {
        let digits = phone.bytes().filter(u8::is_ascii_digit).count();
        if digits < 7 {
            errors.push(PayloadError::new(
                &format!("{prefix}.phone"),
                "tel",
                format!("{prefix} phone number looks incomplete"),
            ));
        }
    }
    errors
}

/// Validate a whole shipment payload. Empty vec = buyable.
pub fn validate_shipment(payload: &ShipmentPayload) -> Vec<PayloadError> {
    let mut errors = validate_address("from", &payload.from);
    errors.extend(validate_address("to", &payload.to));
    if payload.pairing_id.trim().is_empty() {
        errors.push(PayloadError::new(
            "pairing_id",
            "required",
            "pairing id links the outbound and return labels".to_string(),
        ));
    }
    let dims = payload.parcel.dims();
    if !(dims.length_cm > 0.0
        && dims.width_cm > 0.0
        && dims.height_cm > 0.0
        && dims.length_cm < 500.0
        && dims.width_cm < 500.0
        && dims.height_cm < 500.0)
    {
        errors.push(PayloadError::new(
            "parcel",
            "dims",
            "parcel dimensions must be positive and under 500 cm".to_string(),
        ));
    }
    if dims.weight_g == 0 || dims.weight_g > 30_000 {
        errors.push(PayloadError::new(
            "parcel",
            "weight",
            "parcel weight must be 1 g – 30 kg".to_string(),
        ));
    }
    errors
}

impl fmt::Display for PayloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}

impl std::error::Error for PayloadError {}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn sample_address() -> PostalAddress {
        PostalAddress {
            name: "Jane Doe".to_string(),
            street1: "123 Main St".to_string(),
            street2: None,
            city: "Seattle".to_string(),
            state_province: "WA".to_string(),
            postal: "98101".to_string(),
            country: "US".to_string(),
            phone: Some("+1 555 123 4567".to_string()),
        }
    }

    pub(crate) fn sample_payload() -> ShipmentPayload {
        ShipmentPayload {
            from: sample_address(),
            to: PostalAddress {
                name: "WA Secretary of State".to_string(),
                street1: "PO Box 40234".to_string(),
                ..sample_address()
            },
            parcel: Parcel::LetterEnvelope,
            service: ServiceClass::TwoDay,
            pairing_id: "ship-synthetic-001".to_string(),
        }
    }

    #[test]
    fn valid_payload_passes() {
        assert!(validate_shipment(&sample_payload()).is_empty());
    }

    #[test]
    fn defects_are_reported() {
        let mut payload = sample_payload();
        payload.to.country = "usa".to_string();
        payload.to.phone = Some("12".to_string());
        payload.pairing_id.clear();
        payload.parcel = Parcel::Custom(ParcelDims {
            length_cm: 0.0,
            width_cm: 1.0,
            height_cm: 1.0,
            weight_g: 0,
        });
        let codes: Vec<String> = validate_shipment(&payload)
            .iter()
            .map(|e| e.code.clone())
            .collect();
        assert!(codes.contains(&"country".to_string()));
        assert!(codes.contains(&"tel".to_string()));
        assert!(codes.contains(&"required".to_string()));
        assert!(codes.contains(&"dims".to_string()));
        assert!(codes.contains(&"weight".to_string()));
    }

    #[test]
    fn payload_schema_round_trip() {
        let payload = sample_payload();
        let json = serde_json::to_string(&payload).unwrap();
        let back: ShipmentPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(back, payload);
        // Carrier-neutral: no carrier-native keys leak into the schema.
        for banned in ["FEDEX_", "STANDARD_OVERNIGHT", "mailClass", "packagingType"] {
            assert!(!json.contains(banned), "schema leaks {banned}");
        }
    }
}
