-- P2 PII store, file 1 (docs/specs/p2-forms-labels.md §8 P2-D2/D3/D4).
-- Separate from handbook.sqlite's migration sequence: versioned by the
-- p2.sqlite user_version, migrated by db::migrate_p2 (same PRAGMAs, same
-- txn discipline). Placement follows the proposed default (separate
-- p2.sqlite, roadmap §7); OQ-1 still open for author confirmation.
--
-- Cross-DB references (authority / form_template / capture ids) are OPAQUE
-- integers: SQLite cannot enforce FKs across files, so readers must treat a
-- missing parent as "open a task + audit", never as a join assumption.
-- FKs below cover only in-file parents.
--
-- Mutations here are audited in handbook.sqlite's audit table
-- (entity='form_submission'|'shipment'|'shipping_label'); there is
-- deliberately no second audit table. Retention: OQ-7 open (proposed:
-- submitted 2y, drafts 90d, drawn PNGs 1y) -- no sweep until decided.
-- New tables only.

-- P2-D2: one customer fill-in (PII).
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

-- P2-D3: outbound+return pair per submission.
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

-- P2-D4: one label PDF + tracking per direction.
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
  issued_at INTEGER,                  -- NULL until issued
  FOREIGN KEY(shipment_id) REFERENCES shipment(id)
);
CREATE INDEX idx_label_tracking ON shipping_label(tracking_no);
