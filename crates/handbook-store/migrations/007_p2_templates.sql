-- Migration 007: P2 form-template registry + authority P2 columns
-- (docs/specs/p2-forms-labels.md §8 P2-D1/D5; vocabularies from §3/§5/§6).
-- Non-PII, so it lives in handbook.sqlite regardless of the OQ-1 PII
-- decision. The PII tables (form_submission, shipment, shipping_label) live
-- in the separate p2.sqlite (migrations/p2_001_init.sql, proposed default
-- per roadmap §7; OQ-1 still open for author confirmation).
-- New table + partial index + two ADD COLUMNs: no rewrites. Existing
-- authority rows get replica_default='unknown', ship_to_fact_id=NULL.

CREATE TABLE form_template(
  id INTEGER PRIMARY KEY,
  authority_id INTEGER NOT NULL,    -- authority.id
  code TEXT NOT NULL,               -- stable slug, e.g. 'wa-apostille-request'
  title TEXT NOT NULL,
  version INTEGER NOT NULL,         -- our internal monotonic int
  authority_revision TEXT NOT NULL DEFAULT '',  -- authority's edition mark
  status TEXT NOT NULL CHECK(status IN ('draft', 'active', 'needs_review', 'superseded', 'retired')),
  schema_json TEXT NOT NULL,        -- ordered field list (P2-F2)
  layout_json TEXT NOT NULL DEFAULT '{}',
  accepts_replica TEXT NOT NULL DEFAULT 'unknown'
    CHECK(accepts_replica IN ('yes', 'no', 'unknown')),
  accepts_replica_capture_id INTEGER, -- capture.id proving replica acceptance
  accepts_drawn_signature TEXT NOT NULL DEFAULT 'no'
    CHECK(accepts_drawn_signature IN ('yes', 'no')),
  signature_mode TEXT NOT NULL DEFAULT 'wet_ink'
    CHECK(signature_mode IN ('wet_ink', 'drawn_optional')),
  source_capture_id INTEGER,        -- capture.id of the ingested blank form
  fact_ids_json TEXT NOT NULL DEFAULT '[]', -- facts backing prefilled values
  supersedes_id INTEGER,            -- form_template.id of the previous edition
  last_verified_at INTEGER,         -- NULL = never verified
  verify_cadence_days INTEGER NOT NULL DEFAULT 90,
  created_by TEXT NOT NULL DEFAULT '',
  created_at INTEGER NOT NULL,
  FOREIGN KEY(authority_id) REFERENCES authority(id),
  FOREIGN KEY(accepts_replica_capture_id) REFERENCES capture(id),
  FOREIGN KEY(source_capture_id) REFERENCES capture(id),
  FOREIGN KEY(supersedes_id) REFERENCES form_template(id)
);
CREATE UNIQUE INDEX idx_ft_active ON form_template(authority_id, code)
  WHERE status = 'active';

-- P2-D5: per-authority replica default (template value always wins) and
-- preferred mailing-address fact. ADD COLUMN keeps existing rows: the CHECK
-- validates against the 'unknown' default, the FK column stays NULL.
ALTER TABLE authority ADD COLUMN replica_default TEXT NOT NULL DEFAULT 'unknown'
  CHECK(replica_default IN ('yes', 'no', 'unknown'));
ALTER TABLE authority ADD COLUMN ship_to_fact_id INTEGER REFERENCES fact(id);
