//! Frozen Viet Nam end-to-end drill (roadmap item 10).
//!
//! `xtask demo-vietnam`: seeds a TEMP store only (never prod, synthetic
//! only), runs the HCCH → routing → form → label → MCP chain, prints a
//! step-by-step PASS report, and exits nonzero on any red. Frozen inputs:
//! VN effective date 2026-09-11; the 2026-10-13 misroute class is the
//! pre-effective-date row that must still route consular.

use std::collections::BTreeMap;

use forms::FormRenderer as _;
use handbook_store::db::{self, NewAuthority};
use handbook_store::store::Store;

const VN_EFFECTIVE: &str = "2026-09-11";
const VN_DAY_BEFORE: &str = "2026-09-10";
const VN_SLUG: &str = "vietnam";

/// CLI entry: run every step, print PASS/FAIL lines, exit 1 on any red.
pub fn run(args: &[String]) {
    if !args.is_empty() {
        eprintln!("demo-vietnam takes no flags (frozen drill on a temp store)");
        std::process::exit(2);
    }
    // Temp store by construction: nothing here names the prod path, and
    // there is no --store flag to smuggle one in.
    let tmp = tempfile::tempdir().unwrap_or_else(|e| fail("temp store", &e.to_string()));
    let root = tmp.path().join("store");
    println!(
        "demo-vietnam: temp store {} (prod untouched)",
        root.display()
    );
    let store = Store::new(&root);
    ok("temp store root", store.init().map_err(|e| e.to_string()));
    let conn = handbook_store::db::open(&store).unwrap_or_else(|e| {
        fail("open temp database", &e.to_string());
    });
    ok(
        "migrate temp database",
        db::migrate(&conn).map_err(|e| e.to_string()),
    );

    // [1] Seed synthetic VN authority + HCCH-derived routing fact.
    let status = ingest::hcch::MemberStatus {
        country: "Viet Nam".to_string(),
        member: true,
        effective: Some(VN_EFFECTIVE.to_string()),
    };
    let routing = ingest::hcch::derive_routing(&status, VN_EFFECTIVE);
    check(
        "derive_routing says apostille on 2026-09-11",
        routing == ingest::hcch::Routing::Apostille,
    );
    let statement = ingest::hcch::routing_statement(&status, routing);
    check(
        "routing_statement renders",
        statement == "Viet Nam: apostille (effective 2026-09-11)",
    );
    let vn = db::add_authority(
        &conn,
        &NewAuthority {
            kind: "country",
            country: "Viet Nam",
            region: "",
            name: "Viet Nam (synthetic drill)",
            slug: VN_SLUG,
            actor: "demo-vietnam",
            at: 1,
        },
    )
    .unwrap_or_else(|e| fail("seed VN authority", &e.to_string()));
    conn.execute(
        "INSERT INTO capture(authority_id, captured_at, kind, path,
                             bytes_raw, bytes_stored, sha256, trust)
         VALUES (?1, 1, 'file', 'blob/demo-vn', 1, 1, ?2, 'staff_verified')",
        rusqlite::params![vn, "d".repeat(64)],
    )
    .unwrap_or_else(|e| fail("seed VN capture", &e.to_string()));
    let capture_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO fact(authority_id, capture_id, topic, statement,
                          status, created_by, created_at)
         VALUES (?1, ?2, 'legalization_chain', ?3, 'accepted', 'demo-vietnam', 1)",
        rusqlite::params![vn, capture_id, statement],
    )
    .unwrap_or_else(|e| fail("seed VN routing fact", &e.to_string()));
    conn.execute(
        "INSERT INTO build(at, code_git_commit, build_no, facts_hash)
         VALUES (1, '', 1, 'demo-vietnam-frozen')",
        [],
    )
    .unwrap_or_else(|e| fail("seed build row", &e.to_string()));
    println!("[1/5] seed synthetic VN authority + HCCH routing fact ... PASS");

    // [2] Misroute class: pre-effective date must route consular.
    let stale = ingest::hcch::derive_routing(&status, VN_DAY_BEFORE);
    check(
        "2026-10-13 misroute class caught (2026-09-10 routes consular)",
        stale == ingest::hcch::Routing::ConsularLegalization,
    );
    println!("[2/5] misroute class caught (pre-effective -> consular) ... PASS");

    // [3] Render one form PDF via the forms crate.
    let template = forms::FormTemplate {
        authority: VN_SLUG.to_string(),
        code: "vn-apostille-cover".to_string(),
        title: "Viet Nam Apostille Cover (synthetic drill)".to_string(),
        version: 1,
        authority_revision: "drill".to_string(),
        status: forms::TemplateStatus::Draft,
        fields: vec![forms::FieldDef {
            key: "full_name".to_string(),
            label: "Full name".to_string(),
            help: None,
            field_type: forms::FieldType::Text,
            required: true,
            max_len: Some(80),
            pattern: None,
            options: Vec::new(),
            prefill_topic: None,
            pdf: forms::PdfRect {
                page: 1,
                x_mm: 20.0,
                y_mm: 60.0,
                w_mm: 170.0,
                h_mm: 8.0,
                font_size_pt: 11.0,
                multiline: false,
            },
        }],
        accepts_replica: forms::ReplicaAcceptance::Unknown,
        accepts_replica_capture_id: None,
        accepts_drawn_signature: false,
        signature_mode: forms::SignatureMode::WetInk,
        source_capture_id: None,
        fact_ids: Vec::new(),
        supersedes_id: None,
        last_verified_at: None,
        verify_cadence_days: 90,
        created_by: "demo-vietnam".to_string(),
        created_at: 1,
    };
    template
        .validate_schema()
        .unwrap_or_else(|e| fail("form schema", &e.to_string()));
    let values: BTreeMap<String, serde_json::Value> = BTreeMap::from([(
        "full_name".to_string(),
        serde_json::Value::String("Nguyen Van Synthetic".to_string()),
    )]);
    let artifact = forms::PdfRenderer::new()
        .render(&forms::Submission {
            template: &template,
            values: &values,
            generated_at_unix: 1_791_979_200,
            build_no: Some(1),
            replica_notice: None,
        })
        .unwrap_or_else(|e| fail("render form PDF", &e.to_string()));
    check(
        "form PDF magic + sha",
        artifact.bytes.starts_with(b"%PDF-") && artifact.sha256_hex.len() == 64,
    );
    println!(
        "[3/5] render one form PDF ({} bytes) ... PASS",
        artifact.bytes.len()
    );

    // [4] Label payloads: printable, NOT-POSTAGE.
    let payload = labels::ShipmentPayload {
        from: labels::PostalAddress {
            name: "Synthetic Sender".to_string(),
            street1: "123 Test St".to_string(),
            street2: None,
            city: "Seattle".to_string(),
            state_province: "WA".to_string(),
            postal: "98101".to_string(),
            country: "US".to_string(),
            phone: None,
        },
        to: labels::PostalAddress {
            name: "Viet Nam Consulate (synthetic)".to_string(),
            street1: "PO Box 99999".to_string(),
            street2: None,
            city: "Seattle".to_string(),
            state_province: "WA".to_string(),
            postal: "98101".to_string(),
            country: "US".to_string(),
            phone: None,
        },
        parcel: labels::Parcel::LetterEnvelope,
        service: labels::ServiceClass::TwoDay,
        pairing_id: "demo-vietnam-001".to_string(),
    };
    check(
        "label payload validates",
        labels::validate_shipment(&payload).is_empty(),
    );
    let service = labels::service_name(labels::Carrier::Usps, labels::ServiceClass::TwoDay);
    for direction in [labels::Direction::Outbound, labels::Direction::Return] {
        let html = labels::render_printable(
            &payload,
            direction,
            labels::Carrier::Usps,
            labels::PrintFormat::FourBySix,
            &service,
        );
        check(
            "printable label is NOT-POSTAGE",
            html.contains("NOT POSTAGE") && html.contains("Affix USPS postage"),
        );
    }
    println!("[4/5] label payloads validate + printable NOT-POSTAGE ... PASS");

    // [5] MCP lookup_authority shape serves it (pure fn, direct call).
    let found = mcp::lookup_authority(&conn, VN_SLUG)
        .unwrap_or_else(|e| fail("mcp lookup_authority", &e.to_string()))
        .unwrap_or_else(|| fail("mcp lookup_authority", "unknown authority"));
    check(
        "mcp serves the routing fact",
        found
            .facts
            .iter()
            .any(|f| f.topic == "legalization_chain" && f.statement == statement),
    );
    check(
        "mcp build_no + no conflict banner",
        found.build_no == 1 && found.conflict_banner.is_none(),
    );
    println!("[5/5] mcp lookup_authority shape serves it ... PASS");

    println!("demo-vietnam: 5/5 PASS");
}

/// PASS-or-exit helper for `Result<(), String>` steps.
fn ok(step: &str, result: Result<(), String>) {
    if let Err(detail) = result {
        fail(step, &detail);
    }
}

/// Assert a drill predicate; on red, print FAIL and exit 1.
fn check(step: &str, green: bool) {
    if !green {
        fail(step, "predicate false");
    }
}

fn fail(step: &str, detail: &str) -> ! {
    println!("FAIL: {step}: {detail}");
    std::process::exit(1);
}
