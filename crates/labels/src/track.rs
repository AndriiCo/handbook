//! Carrier-neutral tracking: the scan and status shapes the poller
//! stores, plus FedEx Track v1 request and parsing (USPS Tracking v3
//! lives in [`crate::usps`]).

use serde_json::{Value, json};

/// One scan from a carrier's tracking answer, carrier-neutral.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackingScan {
    /// Carrier event code (`eventCode`), else the event type text.
    pub code: String,
    /// Human text as the carrier writes it.
    pub text: String,
    /// ISO-8601 time as the carrier writes it (no time zone math here).
    pub at: String,
    /// "City, ST" when present.
    pub place: String,
}

/// The neutral status buckets the cabinet shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackingStatus {
    PreTransit,
    InTransit,
    Delivered,
    Alert,
}

/// FedEx Track v1: `POST {base}/track/v1/trackingnumbers`.
pub const FEDEX_TRACK_PATH: &str = "/track/v1/trackingnumbers";

/// One tracking number, with scan detail.
pub fn fedex_track_body(tracking_no: &str) -> Value {
    json!({
        "includeDetailedScans": true,
        "trackingInfo": [{ "trackingNumberInfo": { "trackingNumber": tracking_no.trim() } }],
    })
}

/// Status bucket and scans of a FedEx Track answer (first result).
/// `latestStatusDetail.code`: DL delivered; DE, CA, SE, RS exceptions;
/// OC, LP label created; anything else is on its way.
pub fn parse_fedex_track(json: &Value) -> (Option<TrackingStatus>, Vec<TrackingScan>) {
    let result = &json["output"]["completeTrackResults"][0]["trackResults"][0];
    let status = match result["latestStatusDetail"]["code"].as_str().map(str::trim) {
        Some("DL") => Some(TrackingStatus::Delivered),
        Some("DE" | "CA" | "SE" | "RS") => Some(TrackingStatus::Alert),
        Some("OC" | "LP") => Some(TrackingStatus::PreTransit),
        Some("") | None => None,
        Some(_) => Some(TrackingStatus::InTransit),
    };
    let scans = result["scanEvents"]
        .as_array()
        .map(|events| {
            events
                .iter()
                .filter_map(|e| {
                    let text = e["eventDescription"].as_str()?.trim().to_string();
                    let code = e["eventType"]
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| text.clone());
                    let at = e["date"].as_str().unwrap_or("").trim().to_string();
                    let location = &e["scanLocation"];
                    let city = location["city"].as_str().unwrap_or("").trim();
                    let state = location["stateOrProvinceCode"]
                        .as_str()
                        .unwrap_or("")
                        .trim();
                    let place = [city, state]
                        .iter()
                        .filter(|p| !p.is_empty())
                        .copied()
                        .collect::<Vec<_>>()
                        .join(", ");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fedex_answers_map_to_neutral_status_and_scans() {
        let answer = json!({"output": {"completeTrackResults": [{"trackResults": [{
            "latestStatusDetail": {"code": "DL", "description": "Delivered"},
            "scanEvents": [
                {"date": "2026-11-05T10:12:00-08:00", "eventType": "DL",
                 "eventDescription": "Delivered", "scanLocation": {"city": "OLYMPIA", "stateOrProvinceCode": "WA"}},
                {"date": "2026-11-04T08:00:00-08:00", "eventType": "IT",
                 "eventDescription": "In transit", "scanLocation": {}}
            ]
        }]}]}});
        let (status, scans) = parse_fedex_track(&answer);
        assert_eq!(status, Some(TrackingStatus::Delivered));
        assert_eq!(scans.len(), 2);
        assert_eq!(scans[0].place, "OLYMPIA, WA");
        assert_eq!(scans[1].place, "");
        let code = |c: &str| {
            parse_fedex_track(
                &json!({"output": {"completeTrackResults": [{"trackResults": [
                {"latestStatusDetail": {"code": c}}]}]}}),
            )
            .0
        };
        assert_eq!(code("IT"), Some(TrackingStatus::InTransit));
        assert_eq!(code("DE"), Some(TrackingStatus::Alert));
        assert_eq!(code("OC"), Some(TrackingStatus::PreTransit));
        assert_eq!(parse_fedex_track(&json!({})).0, None);
        assert_eq!(
            fedex_track_body(" 1234 ")["trackingInfo"][0]["trackingNumberInfo"]["trackingNumber"],
            "1234"
        );
    }
}
