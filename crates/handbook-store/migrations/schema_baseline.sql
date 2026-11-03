-- schema_baseline.sql — v11 baseline squash (prompt 055, dc400a9 live on prod).
-- = concatenated + normalized 001_init..011_review_queue (deleted; see git history).
-- Fresh databases apply this file once and stamp user_version=11.
-- Databases at v1..v10 MUST upgrade through the last incremental
-- release first (migrate() refuses them loudly, never silently).
-- Later migrations resume the N+1 convention: 012_service_notice.sql
-- (v12) runs on top of this file for fresh and v11 databases alike.

CREATE VIRTUAL TABLE capture_fts USING fts5(capture_id UNINDEXED, body);
CREATE TABLE build(
  id INTEGER PRIMARY KEY,
  at INTEGER NOT NULL,
  code_git_commit TEXT NOT NULL,
  content_git_commit TEXT,        -- NULL until the content repo exists
  build_no INTEGER NOT NULL UNIQUE,
  facts_hash TEXT NOT NULL,
  notes TEXT NOT NULL DEFAULT ''
);
CREATE TABLE demo_session(
  token TEXT PRIMARY KEY,
  customer_ref TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL
);
CREATE TABLE watermark(
  authority_slug TEXT PRIMARY KEY,
  last_run_at INTEGER NOT NULL,
  last_prompt_version TEXT NOT NULL
, last_capture_id INTEGER NOT NULL DEFAULT 0);
CREATE TABLE "authority"(
  id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL CHECK(kind IN ('country', 'state', 'province', 'office')),
  country TEXT NOT NULL,
  region TEXT NOT NULL DEFAULT '',
  name TEXT NOT NULL,
  slug TEXT NOT NULL UNIQUE
, replica_default TEXT NOT NULL DEFAULT 'unknown'
  CHECK(replica_default IN ('yes', 'no', 'unknown')), ship_to_fact_id INTEGER REFERENCES fact(id));
CREATE TABLE "domain"(
  name TEXT PRIMARY KEY,
  authority_id INTEGER,         -- authority.id, NULL when unattributed
  status TEXT NOT NULL CHECK(status IN ('official', 'known_thirdparty', 'unverified', 'fake')),
  evidence_capture_id INTEGER,  -- capture.id proving the status
  notes TEXT NOT NULL DEFAULT '',
  FOREIGN KEY(authority_id) REFERENCES authority(id),
  FOREIGN KEY(evidence_capture_id) REFERENCES capture(id)
);
CREATE TABLE "source"(
  id INTEGER PRIMARY KEY,
  authority_id INTEGER NOT NULL,  -- authority.id
  kind TEXT NOT NULL CHECK(kind IN ('website_official', 'website_thirdparty', 'email_official', 'email_customer', 'phone', 'visit', 'comment', 'document')),
  uri TEXT NOT NULL,
  domain_id TEXT,                 -- domain.name, NULL when not applicable
  robots TEXT NOT NULL DEFAULT '',
  notes TEXT NOT NULL DEFAULT '', poll_cadence_sec INTEGER NOT NULL DEFAULT 86400, etag TEXT NOT NULL DEFAULT '', last_modified TEXT NOT NULL DEFAULT '', content_hash TEXT NOT NULL DEFAULT '', simhash INTEGER NOT NULL DEFAULT 0, last_checked_at INTEGER NOT NULL DEFAULT 0,
  FOREIGN KEY(authority_id) REFERENCES authority(id),
  FOREIGN KEY(domain_id) REFERENCES domain(name)
);
CREATE INDEX idx_source_authority ON source(authority_id);
CREATE TABLE "channel"(
  id INTEGER PRIMARY KEY,
  authority_id INTEGER NOT NULL,  -- authority.id
  visibility TEXT NOT NULL CHECK(visibility IN ('public', 'private')),
  kind TEXT NOT NULL CHECK(kind IN ('phone', 'email', 'webform', 'chat', 'in_person')),
  value TEXT NOT NULL,
  details TEXT NOT NULL DEFAULT '',
  purpose TEXT NOT NULL CHECK(purpose IN ('customer_steering', 'our_validation')),
  FOREIGN KEY(authority_id) REFERENCES authority(id)
);
CREATE INDEX idx_channel_authority ON channel(authority_id);
CREATE TABLE "capture"(
  id INTEGER PRIMARY KEY,
  authority_id INTEGER NOT NULL,  -- authority.id
  source_id INTEGER,              -- source.id, NULL when unattributed
  captured_at INTEGER NOT NULL,
  kind TEXT NOT NULL CHECK(kind IN ('warc', 'file', 'scan', 'photo', 'video', 'audio', 'email', 'transcript', 'comment')),
  path TEXT NOT NULL,
  bytes_raw INTEGER NOT NULL,
  bytes_stored INTEGER NOT NULL,
  sha256 TEXT NOT NULL,
  authenticity TEXT NOT NULL DEFAULT '',
  trust TEXT NOT NULL CHECK(trust IN ('official', 'staff_verified', 'community', 'rejected')),
  notes TEXT NOT NULL DEFAULT '',
  FOREIGN KEY(authority_id) REFERENCES authority(id),
  FOREIGN KEY(source_id) REFERENCES source(id)
);
CREATE INDEX idx_capture_authority ON capture(authority_id, captured_at);
CREATE TABLE "fact"(
  id INTEGER PRIMARY KEY,
  authority_id INTEGER NOT NULL,  -- authority.id
  capture_id INTEGER NOT NULL,    -- capture.id (provenance)
  topic TEXT NOT NULL CHECK(topic IN ('hours', 'fees', 'forms', 'address', 'parking', 'route_photos', 'rejections', 'contacts', 'po_box', 'walkin', 'mailing', 'appointment', 'payment', 'processing_time', 'jurisdiction', 'legalization_chain', 'website', 'phone_script', 'holiday_hours', 'id_requirements', 'copies', 'translation', 'notarization', 'authentication', 'dropbox')),
  statement TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('proposed', 'accepted', 'superseded', 'rejected')),
  created_by TEXT NOT NULL DEFAULT '',
  created_at INTEGER NOT NULL, citation_start INTEGER NOT NULL DEFAULT -1, citation_end INTEGER NOT NULL DEFAULT -1, reasoning TEXT NOT NULL DEFAULT '',
  FOREIGN KEY(authority_id) REFERENCES authority(id),
  FOREIGN KEY(capture_id) REFERENCES capture(id)
);
CREATE INDEX idx_fact_lookup ON fact(authority_id, topic, status);
CREATE TABLE "batch"(
  id INTEGER PRIMARY KEY,
  authority_id INTEGER NOT NULL,  -- authority.id
  channel_id INTEGER NOT NULL,    -- channel.id
  status TEXT NOT NULL CHECK(status IN ('draft', 'scheduled', 'sent', 'done')),
  scheduled_for INTEGER,          -- NULL until scheduled
  notes TEXT NOT NULL DEFAULT '',
  FOREIGN KEY(authority_id) REFERENCES authority(id),
  FOREIGN KEY(channel_id) REFERENCES channel(id)
);
CREATE TABLE "task"(
  id INTEGER PRIMARY KEY,
  authority_id INTEGER NOT NULL,  -- authority.id
  kind TEXT NOT NULL CHECK(kind IN ('validate_fact', 'correction', 'call', 'email_check', 'visit')),
  title TEXT NOT NULL,
  detail TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL CHECK(status IN ('open', 'batched', 'in_progress', 'blocked', 'done', 'rejected')),
  priority INTEGER NOT NULL DEFAULT 0,
  dedup_key TEXT NOT NULL,
  batch_id INTEGER,               -- batch.id, NULL until batched
  created_by TEXT NOT NULL DEFAULT '',
  created_at INTEGER NOT NULL,
  resolved_at INTEGER,            -- NULL until resolved
  resolution TEXT NOT NULL DEFAULT '',
  FOREIGN KEY(authority_id) REFERENCES authority(id),
  FOREIGN KEY(batch_id) REFERENCES batch(id)
);
CREATE UNIQUE INDEX idx_task_dedup ON task(dedup_key)
  WHERE status IN ('open', 'batched', 'in_progress');
CREATE INDEX idx_task_batch ON task(batch_id);
CREATE TABLE "call_session"(
  id INTEGER PRIMARY KEY,
  batch_id INTEGER,               -- batch.id, NULL for ad-hoc calls
  authority_id INTEGER NOT NULL,  -- authority.id
  channel_id INTEGER NOT NULL,    -- channel.id
  consent TEXT NOT NULL DEFAULT 'unknown' CHECK(consent IN ('unknown', 'granted', 'refused')),
  consent_at INTEGER,             -- NULL until answered
  script TEXT NOT NULL DEFAULT '',
  audio_capture_id INTEGER,       -- capture.id (kind='audio'), 365-day retention
  transcript_capture_id INTEGER,  -- capture.id (kind='transcript'), kept forever
  outcome TEXT NOT NULL DEFAULT '',
  conducted_by TEXT NOT NULL DEFAULT '',
  conducted_at INTEGER,           -- NULL until conducted
  FOREIGN KEY(batch_id) REFERENCES batch(id),
  FOREIGN KEY(authority_id) REFERENCES authority(id),
  FOREIGN KEY(channel_id) REFERENCES channel(id),
  FOREIGN KEY(audio_capture_id) REFERENCES capture(id) ON DELETE SET NULL,
  FOREIGN KEY(transcript_capture_id) REFERENCES capture(id)
);
CREATE TABLE "transcript_note"(
  id INTEGER PRIMARY KEY,
  capture_id INTEGER NOT NULL,    -- capture.id (kind='transcript')
  start_byte INTEGER NOT NULL,
  end_byte INTEGER NOT NULL,
  kind TEXT NOT NULL CHECK(kind IN ('ignore', 'correction')),
  text TEXT NOT NULL,
  author TEXT NOT NULL DEFAULT '',
  created_at INTEGER NOT NULL,
  FOREIGN KEY(capture_id) REFERENCES capture(id)
);
CREATE INDEX idx_note_capture ON transcript_note(capture_id);
CREATE TABLE "audit"(
  id INTEGER PRIMARY KEY,
  at INTEGER NOT NULL,
  actor TEXT NOT NULL DEFAULT '',
  action TEXT NOT NULL,
  entity TEXT NOT NULL,
  entity_id INTEGER NOT NULL,
  code_git_commit TEXT NOT NULL DEFAULT '',
  content_git_commit TEXT,
  build_id INTEGER,               -- build.id, NULL when unrelated to a build
  before_ref TEXT NOT NULL DEFAULT '',
  after_ref TEXT NOT NULL DEFAULT '',
  reason TEXT NOT NULL DEFAULT '',
  FOREIGN KEY(build_id) REFERENCES build(id)
);
CREATE INDEX idx_audit_at ON audit(at);
CREATE TRIGGER audit_no_update BEFORE UPDATE ON audit
BEGIN
  SELECT RAISE(ABORT, 'audit is append-only: UPDATE rejected');
END;
CREATE TRIGGER audit_no_delete BEFORE DELETE ON audit
BEGIN
  SELECT RAISE(ABORT, 'audit is append-only: DELETE rejected');
END;
CREATE TABLE config(
  key TEXT PRIMARY KEY,             -- e.g. 'fedex.account_id'
  type TEXT NOT NULL                -- 'bool' | 'text' | 'secret'
    CHECK(type IN ('bool', 'text', 'secret')),
  value_enc BLOB NOT NULL,          -- nonce(24) || XChaCha20-Poly1305 ct
  enc_version INTEGER NOT NULL,     -- KEK generation
  updated_at INTEGER NOT NULL,      -- unix seconds
  updated_by TEXT NOT NULL          -- audit actor
) STRICT;
CREATE TABLE config_kek(
  version INTEGER PRIMARY KEY,      -- 1, 2, ...
  cred_path TEXT NOT NULL,          -- sealed .cred path for this generation
  created_at INTEGER NOT NULL,
  retired_at INTEGER                -- NULL = active
) STRICT;
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
CREATE INDEX idx_source_due ON source(poll_cadence_sec, last_checked_at);
CREATE TABLE mail_poll(
  id INTEGER PRIMARY KEY CHECK (id = 1),
  email TEXT NOT NULL DEFAULT '',
  history_id TEXT NOT NULL DEFAULT '',
  last_poll_at INTEGER NOT NULL DEFAULT 0,
  last_error TEXT NOT NULL DEFAULT ''
);
CREATE TABLE suppress_rule(
  id INTEGER PRIMARY KEY,
  authority_id INTEGER,
  topic TEXT,
  statement_norm TEXT NOT NULL,
  statement TEXT NOT NULL,
  reason TEXT NOT NULL DEFAULT '',
  created_by TEXT NOT NULL DEFAULT '',
  created_at INTEGER NOT NULL,
  revoked_at INTEGER
);
CREATE INDEX idx_rule_match ON suppress_rule(authority_id, topic, statement_norm)
  WHERE revoked_at IS NULL;
CREATE TABLE suppression(
  id INTEGER PRIMARY KEY,
  capture_id INTEGER NOT NULL,
  authority_id INTEGER NOT NULL,
  topic TEXT NOT NULL,
  statement TEXT NOT NULL,
  statement_norm TEXT NOT NULL,
  rule_id INTEGER,
  path TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE INDEX idx_suppression_capture ON suppression(capture_id);
CREATE INDEX idx_suppression_at ON suppression(created_at);
-- Seed rows (squashed 005): the unattributed sentinels backing id-0
-- references (fact.capture_id = 0 until ingest passes real ids).
INSERT OR IGNORE INTO authority(id, kind, country, region, name, slug)
  VALUES (0, 'office', '', '', 'UNATTRIBUTED sentinel (migration 005; backs fact.capture_id = 0 until ingest passes real ids)', 'sentinel-unattributed-005');
INSERT OR IGNORE INTO capture(id, authority_id, source_id, captured_at, kind, path, bytes_raw, bytes_stored, sha256, authenticity, trust, notes)
  VALUES (0, 0, NULL, 0, 'file', '', 0, 0, '', '', 'rejected', 'sentinel: no capture (migration 005)');
