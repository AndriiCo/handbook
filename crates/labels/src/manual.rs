//! Staff-manual purchase + automatic API fallback (P2-L3–L5).
//!
//! Until carrier certification lands, every buy queues a
//! ready-to-buy payload for staff: staff buys in the carrier portal,
//! then pastes back the tracking number and uploads the label PDF.
//! The customer-visible artifact (label PDF + tracking) is identical
//! either way, and the fallback is automatic — the caller never
//! branches on certification itself.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::carrier::Carrier;
use crate::client::{BuyOutcome, BuyRequest, LabelClient};
use crate::payload::{Direction, ServiceClass};

/// Label lifecycle (P2-L5): staff-only transitions, audit-logged by
/// the web layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum LabelStatus {
    Pending,
    Issued,
    Voided,
    Reissued,
}

/// One staff-manual buy task: the ready-to-buy payload plus the
/// pasted-back result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManualLabelTask {
    pub carrier: Carrier,
    pub service: ServiceClass,
    pub direction: Direction,
    pub pairing_id: String,
    /// The prevalidated buy payload staff copies into the portal.
    pub payload_json: Value,
    pub status: LabelStatus,
    pub tracking_no: Option<String>,
    pub label_sha256: Option<String>,
    /// Carrier/API failure that routed this task to manual, if any.
    pub api_error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl ManualLabelTask {
    pub fn new(
        carrier: Carrier,
        request: &BuyRequest,
        payload_json: Value,
        api_error: Option<String>,
        now_unix: i64,
    ) -> Self {
        Self {
            carrier,
            service: request.payload.service,
            direction: request.direction,
            pairing_id: request.payload.pairing_id.clone(),
            payload_json,
            status: LabelStatus::Pending,
            tracking_no: None,
            label_sha256: None,
            api_error,
            created_at: now_unix,
            updated_at: now_unix,
        }
    }

    /// Staff paste-back: tracking + label PDF sha move the task to
    /// `issued` (or `reissued` after a void).
    ///
    /// # Errors
    ///
    /// Returns [`ManualError`] when the task is already issued or the
    /// paste-back values are blank.
    pub fn paste_back(
        &mut self,
        tracking_no: &str,
        label_sha256: &str,
        now_unix: i64,
    ) -> Result<(), ManualError> {
        if !matches!(self.status, LabelStatus::Pending | LabelStatus::Voided) {
            return Err(ManualError::AlreadyIssued);
        }
        if tracking_no.trim().is_empty() || label_sha256.trim().is_empty() {
            return Err(ManualError::EmptyPasteBack);
        }
        self.tracking_no = Some(tracking_no.trim().to_string());
        self.label_sha256 = Some(label_sha256.trim().to_string());
        self.status = match self.status {
            LabelStatus::Voided => LabelStatus::Reissued,
            _ => LabelStatus::Issued,
        };
        self.updated_at = now_unix;
        Ok(())
    }

    /// Void an issued label (staff-only).
    ///
    /// # Errors
    ///
    /// Returns [`ManualError`] unless the task is currently issued.
    pub fn void(&mut self, now_unix: i64) -> Result<(), ManualError> {
        if self.status != LabelStatus::Issued {
            return Err(ManualError::NotIssued);
        }
        self.status = LabelStatus::Voided;
        self.updated_at = now_unix;
        Ok(())
    }
}

/// [`ManualLabelTask`] transition failure.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ManualError {
    AlreadyIssued,
    NotIssued,
    EmptyPasteBack,
}

impl fmt::Display for ManualError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ManualError::AlreadyIssued => write!(f, "label is already issued"),
            ManualError::NotIssued => write!(f, "only an issued label can be voided"),
            ManualError::EmptyPasteBack => {
                write!(f, "tracking number and label sha are required")
            }
        }
    }
}

impl std::error::Error for ManualError {}

/// Buy result: either a carrier-bought label or a queued manual task.
#[derive(Debug)]
#[non_exhaustive]
pub enum FallbackOutcome {
    Bought(BuyOutcome),
    QueuedForManual(ManualLabelTask),
}

/// Buy through the carrier API, queueing a staff-manual task on any
/// failure (uncertified client, transport error, carrier rejection).
/// The fallback is automatic: callers match on the outcome, never on
/// certification state.
pub fn buy_with_fallback<C: LabelClient>(
    client: &C,
    bearer: &str,
    request: &BuyRequest,
    now_unix: i64,
) -> FallbackOutcome {
    if !client.ready() {
        return FallbackOutcome::QueuedForManual(queue(
            client.carrier(),
            request,
            Some("carrier api not certified yet".to_string()),
            now_unix,
        ));
    }
    match client.buy(bearer, request) {
        Ok(outcome) => FallbackOutcome::Bought(outcome),
        Err(error) => FallbackOutcome::QueuedForManual(queue(
            client.carrier(),
            request,
            Some(error.to_string()),
            now_unix,
        )),
    }
}

fn queue(
    carrier: Carrier,
    request: &BuyRequest,
    api_error: Option<String>,
    now_unix: i64,
) -> ManualLabelTask {
    let payload_json = match carrier {
        Carrier::Usps => crate::client::usps_label_body(request),
        Carrier::FedEx => crate::client::fedex_ship_body(request),
    };
    ManualLabelTask::new(carrier, request, payload_json, api_error, now_unix)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{LabelError, TransportError};

    struct ScriptedClient {
        ready: bool,
        result: Result<BuyOutcome, LabelError>,
    }

    impl LabelClient for ScriptedClient {
        fn carrier(&self) -> Carrier {
            Carrier::Usps
        }

        fn ready(&self) -> bool {
            self.ready
        }

        fn buy(&self, _bearer: &str, _request: &BuyRequest) -> Result<BuyOutcome, LabelError> {
            match &self.result {
                Ok(outcome) => Ok(outcome.clone()),
                Err(LabelError::Transport(e)) => Err(LabelError::Transport(e.clone())),
                Err(LabelError::Api { status, message }) => Err(LabelError::Api {
                    status: *status,
                    message: message.clone(),
                }),
                Err(other) => panic!("scripted for transport/api only, got {other:?}"),
            }
        }

        fn cancel(&self, _bearer: &str, _tracking_no: &str) -> Result<(), LabelError> {
            Ok(())
        }
    }

    fn outcome() -> BuyOutcome {
        BuyOutcome {
            tracking_no: "track-1".to_string(),
            label_pdf: b"%PDF-1.4 synthetic".to_vec(),
            postage_cost_cents: Some(999),
            carrier_service: "PRIORITY".to_string(),
        }
    }

    #[test]
    fn api_success_buys() {
        let client = ScriptedClient {
            ready: true,
            result: Ok(outcome()),
        };
        let payload = crate::payload::tests::sample_payload();
        let request = BuyRequest {
            payload: &payload,
            direction: Direction::Outbound,
            payment_ref: "pay",
        };
        assert!(matches!(
            buy_with_fallback(&client, "b", &request, 100),
            FallbackOutcome::Bought(_)
        ));
    }

    #[test]
    fn api_failure_queues_manual_with_reason() {
        for error in [
            LabelError::Transport(TransportError::new("dns down")),
            LabelError::Api {
                status: 500,
                message: "carrier blew up".to_string(),
            },
        ] {
            let client = ScriptedClient {
                ready: true,
                result: Err(error),
            };
            let payload = crate::payload::tests::sample_payload();
            let request = BuyRequest {
                payload: &payload,
                direction: Direction::Outbound,
                payment_ref: "pay",
            };
            let FallbackOutcome::QueuedForManual(task) =
                buy_with_fallback(&client, "b", &request, 100)
            else {
                panic!("expected the manual queue");
            };
            assert_eq!(task.status, LabelStatus::Pending);
            assert!(task.api_error.is_some());
            assert_eq!(task.payload_json["imageInfo"]["imageType"], "PDF");
        }
    }

    #[test]
    fn uncertified_queues_without_calling() {
        let client = ScriptedClient {
            ready: false,
            result: Ok(outcome()),
        };
        let payload = crate::payload::tests::sample_payload();
        let request = BuyRequest {
            payload: &payload,
            direction: Direction::Outbound,
            payment_ref: "pay",
        };
        let FallbackOutcome::QueuedForManual(task) = buy_with_fallback(&client, "b", &request, 100)
        else {
            panic!("expected the manual queue");
        };
        assert_eq!(
            task.api_error.as_deref(),
            Some("carrier api not certified yet")
        );
    }

    #[test]
    fn paste_back_void_reissue_cycle() {
        let payload = crate::payload::tests::sample_payload();
        let request = BuyRequest {
            payload: &payload,
            direction: Direction::Outbound,
            payment_ref: "pay",
        };
        let mut task = queue(Carrier::Usps, &request, None, 100);
        assert!(task.void(101).is_err());
        task.paste_back("track-9", "sha", 102).unwrap();
        assert_eq!(task.status, LabelStatus::Issued);
        assert!(task.paste_back("track-9", "sha", 103).is_err());
        task.void(104).unwrap();
        assert_eq!(task.status, LabelStatus::Voided);
        task.paste_back("track-10", "sha2", 105).unwrap();
        assert_eq!(task.status, LabelStatus::Reissued);
        assert_eq!(task.tracking_no.as_deref(), Some("track-10"));
    }
}
