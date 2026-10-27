//! P2 shipping labels: carrier-neutral payloads, API-buy clients,
//! and staff-manual fallback.
//!
//! The payload schema is the P3 shopping list
//! ([carrier-apis.md](../../../../docs/specs/carrier-apis.md) §3): it
//! collects the API-buy superset so staff-manual (P2) and API-buy (P3)
//! share one shape. Decided direction (specs README §10): **carrier
//! API buy with staff-manual as automatic fallback** until USPS Labels
//! approval + FedEx certification land, and **no prices or dates in
//! any UI** until the numbered pricing table is supplied.
//!
//! Per-hop carrier constraints come from the chain research: the Egypt
//! consulate mail hop is USPS-only (FedEx refused). [`MailHop`]
//! enforces that at buy time, not in copy.
//!
//! Start at [`ShipmentPayload`], [`LabelClient`], and
//! [`buy_with_fallback`]. [`LabelSummary`] is the only type the UI
//! layer may render: it cannot carry a price or a date by
//! construction.

pub mod carrier;
pub mod client;
pub mod display;
pub mod manual;
pub mod payload;

pub use carrier::{Carrier, CarrierError, MailHop, carrier_for_hop, service_name};
pub use client::{
    BuyOutcome, BuyRequest, HttpResponse, HttpTransport, LabelClient, LabelError, TransportError,
    UreqTransport,
};
pub use display::LabelSummary;
pub use manual::{FallbackOutcome, LabelStatus, ManualError, ManualLabelTask, buy_with_fallback};
pub use payload::{
    Direction, Parcel, ParcelDims, PayloadError, PostalAddress, ServiceClass, ShipmentPayload,
    validate_address, validate_shipment,
};
