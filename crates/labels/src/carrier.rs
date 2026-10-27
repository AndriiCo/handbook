//! Carriers, per-hop constraints, and service-code mapping.
//!
//! The Egypt consulate mail hop is USPS-only — the consulate refuses
//! FedEx/UPS (egypt-consular.md). That constraint is enforced here at
//! buy time so no UI copy or caller mistake can route a FedEx label
//! onto that hop.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Label carrier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Carrier {
    Usps,
    FedEx,
}

impl fmt::Display for Carrier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Carrier::Usps => write!(f, "USPS"),
            Carrier::FedEx => write!(f, "FedEx"),
        }
    }
}

/// Which leg of a chain a label serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MailHop {
    /// Any hop without a carrier restriction.
    Standard,
    /// Mail to an Egypt consulate/embassy: USPS-only.
    EgyptConsulate,
}

/// Resolve the carrier for a hop: the Egypt hop coerces to USPS and
/// rejects a FedEx request instead of silently swapping it.
pub fn carrier_for_hop(hop: MailHop, requested: Carrier) -> Result<Carrier, CarrierError> {
    match (hop, requested) {
        (MailHop::EgyptConsulate, Carrier::FedEx) => Err(CarrierError::EgyptHopUspsOnly),
        (_, carrier) => Ok(carrier),
    }
}

/// USPS mail class for a neutral tier (carrier-apis §1: Ground
/// Advantage / Priority / Express).
pub fn usps_mail_class(service: crate::ServiceClass) -> &'static str {
    match service {
        crate::ServiceClass::Overnight => "EXPRESS",
        crate::ServiceClass::TwoDay => "PRIORITY",
        crate::ServiceClass::Ground => "GROUND_ADVANTAGE",
    }
}

/// FedEx `serviceType` for a neutral tier (carrier-apis §2).
/// Final enum choice is a product decision (demo-slice Q7); these are
/// the documented values to start from.
pub fn fedex_service_type(service: crate::ServiceClass) -> &'static str {
    match service {
        crate::ServiceClass::Overnight => "STANDARD_OVERNIGHT",
        crate::ServiceClass::TwoDay => "FEDEX_2_DAY",
        crate::ServiceClass::Ground => "FEDEX_GROUND",
    }
}

/// Human service name without price, e.g. `USPS Priority` — the
/// only service text the UI may show.
pub fn service_name(carrier: Carrier, service: crate::ServiceClass) -> String {
    let tier = match service {
        crate::ServiceClass::Overnight => match carrier {
            Carrier::Usps => "Express",
            Carrier::FedEx => "Overnight",
        },
        crate::ServiceClass::TwoDay => match carrier {
            Carrier::Usps => "Priority",
            Carrier::FedEx => "2Day",
        },
        crate::ServiceClass::Ground => match carrier {
            Carrier::Usps => "Ground Advantage",
            Carrier::FedEx => "Ground",
        },
    };
    format!("{carrier} {tier}")
}

/// [`carrier_for_hop`] failure.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CarrierError {
    EgyptHopUspsOnly,
}

impl fmt::Display for CarrierError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CarrierError::EgyptHopUspsOnly => {
                write!(f, "the Egypt consulate hop accepts USPS only")
            }
        }
    }
}

impl std::error::Error for CarrierError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ServiceClass;

    #[test]
    fn egypt_hop_is_usps_only() {
        assert_eq!(
            carrier_for_hop(MailHop::EgyptConsulate, Carrier::Usps).unwrap(),
            Carrier::Usps
        );
        assert_eq!(
            carrier_for_hop(MailHop::EgyptConsulate, Carrier::FedEx).unwrap_err(),
            CarrierError::EgyptHopUspsOnly
        );
        assert_eq!(
            carrier_for_hop(MailHop::Standard, Carrier::FedEx).unwrap(),
            Carrier::FedEx
        );
    }

    #[test]
    fn ui_service_names() {
        assert_eq!(
            service_name(Carrier::Usps, ServiceClass::TwoDay),
            "USPS Priority"
        );
        assert_eq!(
            service_name(Carrier::FedEx, ServiceClass::Ground),
            "FedEx Ground"
        );
    }

    #[test]
    fn service_code_mapping() {
        assert_eq!(usps_mail_class(ServiceClass::Overnight), "EXPRESS");
        assert_eq!(usps_mail_class(ServiceClass::TwoDay), "PRIORITY");
        assert_eq!(usps_mail_class(ServiceClass::Ground), "GROUND_ADVANTAGE");
        assert_eq!(
            fedex_service_type(ServiceClass::Overnight),
            "STANDARD_OVERNIGHT"
        );
        assert_eq!(fedex_service_type(ServiceClass::TwoDay), "FEDEX_2_DAY");
        assert_eq!(fedex_service_type(ServiceClass::Ground), "FEDEX_GROUND");
    }
}
