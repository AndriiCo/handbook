//! SA-19 sandbox proof: FedEx quote + label + void against recorded
//! synthetic fixtures (no credentials, no network).
//!
//! Builder wiring — add to `xtask/src/main.rs` (this file is new; the
//! arm below is the exact snippet to splice in, `mod` list included):
//!
//! ```rust,ignore
//! mod fedex; // alongside the other `mod` lines at the top
//! // inside the match in `main()`:
//! Some("fedex") => fedex::run(&args[1..]),
//! ```
//!
//! And one help line in `print_help()`:
//!
//! ```text
//!   fedex       live-fedex sandbox proof: quote + label + void (recorded fixtures)
//! ```
//!
//! Usage: `cargo xtask fedex`.

use labels::live::{
    BuyParams, FixtureSet, Leg, LiveConfig, LiveShipment, MemLedger, SandboxTransport, VoidParams,
    idempotency_key, oauth_token_url, parse_token_response,
};
use labels::{BuyRequest, Direction, ServiceClass};

/// Run the sandbox proof and print the transcript to stdout.
pub fn run(args: &[String]) {
    if args
        .iter()
        .any(|a| a == "--help" || a == "-h" || a == "help")
    {
        println!(
            "Usage: cargo xtask fedex\n\n\
             Sandbox proof for live FedEx (SA-19): OAuth, rate quote,\n\
             outbound + return label, void — all against recorded\n\
             synthetic fixtures. No credentials, no network.\n\
             Prints the transcript to stdout; exits nonzero on failure."
        );
        return;
    }
    match proof() {
        Ok(lines) => {
            for line in lines {
                println!("{line}");
            }
        }
        Err(message) => {
            eprintln!("fedex sandbox proof FAILED: {message}");
            std::process::exit(1);
        }
    }
}

fn proof() -> Result<Vec<String>, String> {
    let mut lines = vec!["fedex sandbox proof (recorded fixtures, no network)".to_string()];
    let cfg = LiveConfig {
        enabled: true,
        daily_cap: 2,
        max_usd_cents: 6_000,
        ship_to_allow: vec!["98001".to_string()],
    };

    // 1. OAuth against the sandbox token endpoint (fixture replay).
    let token_url = oauth_token_url("https://apis-sandbox.fedex.com");
    lines.push(format!("oauth POST {token_url}"));
    let transport = SandboxTransport::new(FixtureSet::synthetic());
    let token_body = FixtureSet::synthetic().token;
    let bearer = parse_token_response(&token_body).map_err(|e| e.to_string())?;
    lines.push("oauth ok (token bytes redacted)".to_string());
    let _ = bearer;

    // 2. Rate quote (synthetic USD 12.50) + cap check.
    let shipment = LiveShipment::new("https://apis-sandbox.fedex.com", transport);
    let payload = synthetic_payload();
    let request = BuyRequest {
        payload: &payload,
        direction: Direction::Outbound,
        payment_ref: "acct-synthetic",
    };
    let cents = shipment
        .quote(&cfg, "bearer-synthetic", &request)
        .map_err(|e| e.to_string())?;
    cfg.check_cost(cents).map_err(|e| e.to_string())?;
    lines.push(format!("quote ok: {cents}c (cap {}c)", cfg.max_usd_cents));

    // 3. Outbound + return label (one order, one leg each).
    let mut ledger = MemLedger::new();
    let mut trackings = Vec::new();
    for leg in [Leg::Outbound, Leg::Return] {
        let key = idempotency_key("order-synthetic-1", leg);
        let bought = shipment
            .buy_pair(
                BuyParams {
                    cfg: &cfg,
                    actor: "staff-synthetic",
                    at: 1,
                    order_id: "order-synthetic-1",
                    leg,
                    bearer: "bearer-synthetic",
                    request: &request,
                    quoted_cents: Some(cents),
                },
                &mut ledger,
            )
            .map_err(|e| e.to_string())?;
        lines.push(format!(
            "label ok: leg={leg} key={key} tracking={}",
            bought[0].tracking_no
        ));
        trackings.push(bought[0].tracking_no.clone());
    }

    // 4. Packet cover (prefilled form for the packet).
    let cover = labels::render_packet_cover(
        &payload,
        "FedEx 2Day",
        &trackings[0],
        &trackings[1],
        "order-synthetic-1",
    );
    if !cover.contains(&trackings[0]) || !cover.contains(&trackings[1]) {
        return Err("packet cover misses tracking numbers".to_string());
    }
    lines.push(format!("packet cover ok ({} bytes)", cover.len()));

    // 5. Void the return leg; ledger stays closed, audit grows.
    shipment
        .void(
            VoidParams {
                cfg: &cfg,
                actor: "staff-synthetic",
                at: 2,
                order_id: "order-synthetic-1",
                leg: Leg::Return,
                bearer: "bearer-synthetic",
            },
            &mut ledger,
        )
        .map_err(|e| e.to_string())?;
    lines.push("void ok: order-synthetic-1:return".to_string());
    let actions: Vec<_> = ledger
        .audit_trail()
        .iter()
        .map(|r| r.action.as_str())
        .collect();
    lines.push(format!("audit trail: {}", actions.join(",")));
    if actions != ["bought", "bought", "voided"] {
        return Err(format!("unexpected audit trail: {actions:?}"));
    }
    lines.push("fedex sandbox proof PASSED".to_string());
    Ok(lines)
}

fn synthetic_payload() -> labels::ShipmentPayload {
    use labels::{Parcel, PostalAddress, ShipmentPayload};
    ShipmentPayload {
        from: PostalAddress {
            name: "Synthetic Sender".to_string(),
            street1: "1 Synthetic Way".to_string(),
            street2: None,
            city: "Seattle".to_string(),
            state_province: "WA".to_string(),
            postal: "98101".to_string(),
            country: "US".to_string(),
            phone: None,
        },
        to: PostalAddress {
            name: "Synthetic Recipient".to_string(),
            street1: "2 Fixture Ave".to_string(),
            street2: None,
            city: "Federal Way".to_string(),
            state_province: "WA".to_string(),
            postal: "98001".to_string(),
            country: "US".to_string(),
            phone: None,
        },
        parcel: Parcel::LetterEnvelope,
        service: ServiceClass::TwoDay,
        pairing_id: "ship-synthetic-fedex-001".to_string(),
    }
}
