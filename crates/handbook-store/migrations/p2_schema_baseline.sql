-- p2_schema_baseline.sql — P2 v2 baseline squash (prompt 055).
-- = p2_001_init + p2_002_label_purchase (deleted; see git history).
-- Fresh P2 databases apply this file once and stamp user_version=2.
-- P2 databases at v1 MUST upgrade through the last incremental
-- release first. Future P2 migrations resume p2_N+1.

CREATE TABLE form_submission(
  id INTEGER PRIMARY KEY,
  form_template_id INTEGER NOT NULL,  -- form_template.id (handbook.sqlite, opaque)
  authority_id INTEGER NOT NULL,      -- authority.id (handbook.sqlite, opaque)
  customer_ref TEXT NOT NULL,
  values_json TEXT NOT NULL DEFAULT '{}', -- keyed by schema key
  customer_override INTEGER NOT NULL DEFAULT 0 CHECK(customer_override IN (0, 1)),
  replica_unconfirmed INTEGER NOT NULL DEFAULT 0 CHECK(replica_unconfirmed IN (0, 1)),
  signature_mode_used TEXT CHECK(signature_mode_used IN ('wet_ink', 'drawn_optional')),
  signature_capture_id INTEGER,       -- capture.id (handbook.sqlite, opaque), drawn PNG
  pdf_capture_id INTEGER,             -- capture.id (handbook.sqlite, opaque), rendered PDF
  pdf_sha256 TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL CHECK(status IN ('draft', 'submitted', 'superseded', 'voided')),
  created_at INTEGER NOT NULL,
  submitted_at INTEGER                -- NULL until submitted
);
CREATE TABLE shipment(
  id INTEGER PRIMARY KEY,
  submission_id INTEGER NOT NULL,
  authority_id INTEGER NOT NULL,      -- authority.id (handbook.sqlite, opaque)
  status TEXT NOT NULL CHECK(status IN ('draft', 'labels_pending', 'ready', 'voided')),
  from_json TEXT NOT NULL DEFAULT '{}',
  to_json TEXT NOT NULL DEFAULT '{}',
  parcel_json TEXT NOT NULL DEFAULT '{}',
  postage_cost_cents INTEGER,         -- recorded, never charged (P2-L4)
  paid_by TEXT NOT NULL DEFAULT '',   -- vocabulary open (OQ-3); free text until decided
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  FOREIGN KEY(submission_id) REFERENCES form_submission(id)
);
CREATE TABLE shipping_label(
  id INTEGER PRIMARY KEY,
  shipment_id INTEGER NOT NULL,
  direction TEXT NOT NULL CHECK(direction IN ('outbound', 'return')),
  carrier TEXT NOT NULL CHECK(carrier IN ('fedex', 'usps')),
  service TEXT NOT NULL,              -- carrier service code; vocabulary owned
                                      -- by S6 (no CHECK: codes grow with P3)
  tracking_no TEXT NOT NULL DEFAULT '',
  label_capture_id INTEGER,           -- capture.id (handbook.sqlite, opaque), label PDF
  payload_json TEXT NOT NULL DEFAULT '{}',
  status TEXT NOT NULL DEFAULT 'pending'
    CHECK(status IN ('pending', 'issued', 'voided', 'reissued')),
  created_at INTEGER NOT NULL,
  issued_at INTEGER, purchase_option TEXT NOT NULL DEFAULT 'prepaid'
    CHECK(purchase_option IN ('printable', 'prepaid')), label_format TEXT NOT NULL DEFAULT 'four_by_six', label_sha256 TEXT NOT NULL DEFAULT '', content_type TEXT NOT NULL DEFAULT 'application/pdf',                  -- NULL until issued
  FOREIGN KEY(shipment_id) REFERENCES shipment(id)
);
CREATE INDEX idx_label_tracking ON shipping_label(tracking_no);
CREATE TABLE profile_label_defaults(
  customer_ref TEXT PRIMARY KEY,
  option_a_format TEXT NOT NULL DEFAULT 'four_by_six',
  option_b_paper TEXT NOT NULL DEFAULT 'four_by_six',
  updated_at INTEGER NOT NULL
);
