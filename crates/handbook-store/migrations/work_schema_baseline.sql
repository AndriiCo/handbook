-- work_schema_baseline.sql — work-queue v1 baseline (P392).
-- Separate file: `work.sqlite`, its own user_version (independent of
-- handbook.sqlite and p2.sqlite). Fresh databases apply this file once
-- and stamp user_version=1. Future work migrations resume work_N+1.
-- Spec: docs/specs/work-queue.md.

CREATE TABLE work_item(
  id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL,                 -- e.g. 'research.nonparty' (vocabulary owned by the web lane)
  payload_json TEXT NOT NULL DEFAULT '{}',
  status TEXT NOT NULL DEFAULT 'pending'
    CHECK(status IN ('pending', 'leased', 'done', 'failed')),
  lease_owner TEXT,                   -- '<worker>:<lease token>' while leased, else NULL
  lease_expires_at INTEGER,           -- unix seconds while leased, else NULL
  attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts >= 0),
  max_attempts INTEGER NOT NULL DEFAULT 3 CHECK(max_attempts >= 1),
  result_sha256 TEXT,                 -- store blob/xx/yy/<sha> (content-addressed), NULL until done
  result_bytes INTEGER,
  result_content_type TEXT,
  summary_json TEXT,                  -- small worker-reported summary, NULL until done/failed
  error TEXT,                         -- last failure reason
  created_by TEXT NOT NULL DEFAULT '',
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  CHECK((status = 'leased') = (lease_owner IS NOT NULL AND lease_expires_at IS NOT NULL))
);
CREATE INDEX idx_work_item_status ON work_item(status, id);

CREATE TABLE audit(
  id INTEGER PRIMARY KEY,
  at INTEGER NOT NULL,
  actor TEXT NOT NULL DEFAULT '',
  action TEXT NOT NULL,
  entity TEXT NOT NULL,
  entity_id INTEGER NOT NULL,
  reason TEXT NOT NULL DEFAULT ''
);
CREATE INDEX idx_work_audit_entity ON audit(entity, entity_id);
CREATE TRIGGER audit_no_update BEFORE UPDATE ON audit
BEGIN
  SELECT RAISE(ABORT, 'audit is append-only: UPDATE rejected');
END;
CREATE TRIGGER audit_no_delete BEFORE DELETE ON audit
BEGIN
  SELECT RAISE(ABORT, 'audit is append-only: DELETE rejected');
END;
