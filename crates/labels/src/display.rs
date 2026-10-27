//! UI-safe label summaries: no prices, no dates (decided direction).
//!
//! The pricing table is still open, so no price or date may render in
//! any label UI. [`LabelSummary`] enforces that by construction: it
//! has no cost field and no timestamp field, so the SPA cannot leak
//! what it cannot see. Cost stays in [`BuyOutcome`](crate::BuyOutcome)
//! for the storage layer only.

use serde::{Deserialize, Serialize};

use crate::carrier::Carrier;
use crate::client::BuyOutcome;
use crate::manual::LabelStatus;
use crate::payload::Direction;

/// Everything the UI may show about a label. Deliberately missing:
/// `postage_cost_cents`, `created_at`, `issued_at`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelSummary {
    pub direction: Direction,
    pub carrier: Carrier,
    /// Human service name without price, e.g. `USPS Priority`.
    pub service_name: String,
    pub status: LabelStatus,
    pub tracking_no: Option<String>,
    pub label_url: Option<String>,
}

impl LabelSummary {
    /// Summary of a carrier-bought label. The outcome's recorded cost
    /// is intentionally dropped here.
    pub fn from_outcome(
        direction: Direction,
        carrier: Carrier,
        service: crate::ServiceClass,
        outcome: &BuyOutcome,
        label_url: Option<String>,
    ) -> Self {
        Self {
            direction,
            carrier,
            service_name: crate::service_name(carrier, service),
            status: LabelStatus::Issued,
            tracking_no: Some(outcome.tracking_no.clone()),
            label_url,
        }
    }

    /// Summary of a staff-manual task at any lifecycle stage.
    pub fn from_task(task: &crate::ManualLabelTask, label_url: Option<String>) -> Self {
        Self {
            direction: task.direction,
            carrier: task.carrier,
            service_name: crate::service_name(task.carrier, task.service),
            status: task.status,
            tracking_no: task.tracking_no.clone(),
            label_url,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_carries_no_price_or_date() {
        let outcome = BuyOutcome {
            tracking_no: "track-1".to_string(),
            label_pdf: b"%PDF-1.4 x".to_vec(),
            postage_cost_cents: Some(999),
            carrier_service: "PRIORITY".to_string(),
        };
        let summary = LabelSummary::from_outcome(
            Direction::Outbound,
            Carrier::Usps,
            crate::ServiceClass::TwoDay,
            &outcome,
            Some("/apostille/api/v1/shipments/1/label".to_string()),
        );
        assert_eq!(summary.service_name, "USPS Priority");
        let json = serde_json::to_string(&summary).unwrap();
        for banned in [
            "999",
            "cent",
            "cost",
            "price",
            "created",
            "issued_at",
            "_at",
        ] {
            assert!(!json.contains(banned), "summary leaks {banned}: {json}");
        }
        let keys: Vec<String> = serde_json::from_str::<serde_json::Value>(&json)
            .unwrap()
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        assert_eq!(
            keys,
            vec![
                "carrier",
                "direction",
                "label_url",
                "service_name",
                "status",
                "tracking_no",
            ]
        );
    }
}
