-- Migration 005: enforce controlled vocabularies (CHECK) + references (FK).
-- SEC finding B2: 001 declares "No FOREIGN KEY enforcement" and 002 shipped
-- only audit triggers. Every table below is rebuilt as CREATE `*_new` with
-- constraints -> INSERT..SELECT -> DROP old -> RENAME new, preserving rows.
--
-- Why this shape: migrate() runs inside one transaction with
-- `PRAGMA foreign_keys=ON`, and that pragma is a no-op inside a
-- transaction, so FKs stay on throughout. With FKs on, ALTER TABLE RENAME
-- rewrites other tables' REFERENCES to the new name, so the temp name must
-- live on the NEW table (nothing references it): renaming `*_new` back to
-- the real name rewrites nothing. DROP TABLE performs an implicit DELETE
-- that trips child FKs, so enforcement is deferred with
-- `PRAGMA defer_foreign_keys=ON` (settable mid-transaction, unlike
-- `foreign_keys`) for the rebuild. Turning defer off mid-transaction
-- discards pending checks, so a guard table re-verifies every FK via
-- `pragma_foreign_key_check` BEFORE defer is turned back off: dangling
-- legacy rows abort the migration instead of shipping silently.
--
-- The audit triggers from 002 die with DROP TABLE, so they are dropped
-- before the audit rebuild and recreated identically after.
--
-- Deliberate data note: runner.rs stores proposed facts with capture_id = 0
-- (UNATTRIBUTED_CAPTURE_ID) until ingest passes real ids. A real FK needs a
-- parent row, so this migration seeds ONE clearly-labeled sentinel authority
-- (id 0) + sentinel capture (id 0, OR IGNORE so upgrades never fail).
--
-- Deliberate action note: retention_sweep deletes the audio capture row BEFORE
-- nulling call_session.audio_capture_id, so that one FK is ON DELETE SET NULL
-- (the sweep's explicit UPDATE becomes a harmless no-op). All other FKs use
-- the default NO ACTION.

PRAGMA defer_foreign_keys=ON;

-- authority.kind: 001:8.
CREATE TABLE authority_new(
  id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL CHECK(kind IN ('country', 'state', 'province', 'office')),
  country TEXT NOT NULL,
  region TEXT NOT NULL DEFAULT '',
  name TEXT NOT NULL,
  slug TEXT NOT NULL UNIQUE
);
INSERT INTO authority_new(id, kind, country, region, name, slug)
  SELECT id, kind, country, region, name, slug FROM authority;
DROP TABLE authority;
ALTER TABLE authority_new RENAME TO authority;

-- domain.status: 001:19.
CREATE TABLE domain_new(
  name TEXT PRIMARY KEY,
  authority_id INTEGER,         -- authority.id, NULL when unattributed
  status TEXT NOT NULL CHECK(status IN ('official', 'known_thirdparty', 'unverified', 'fake')),
  evidence_capture_id INTEGER,  -- capture.id proving the status
  notes TEXT NOT NULL DEFAULT '',
  FOREIGN KEY(authority_id) REFERENCES authority(id),
  FOREIGN KEY(evidence_capture_id) REFERENCES capture(id)
);
INSERT INTO domain_new(name, authority_id, status, evidence_capture_id, notes)
  SELECT name, authority_id, status, evidence_capture_id, notes FROM domain;
DROP TABLE domain;
ALTER TABLE domain_new RENAME TO domain;

-- source.kind: 001:28-30.
CREATE TABLE source_new(
  id INTEGER PRIMARY KEY,
  authority_id INTEGER NOT NULL,  -- authority.id
  kind TEXT NOT NULL CHECK(kind IN ('website_official', 'website_thirdparty', 'email_official', 'email_customer', 'phone', 'visit', 'comment', 'document')),
  uri TEXT NOT NULL,
  domain_id TEXT,                 -- domain.name, NULL when not applicable
  robots TEXT NOT NULL DEFAULT '',
  notes TEXT NOT NULL DEFAULT '',
  FOREIGN KEY(authority_id) REFERENCES authority(id),
  FOREIGN KEY(domain_id) REFERENCES domain(name)
);
INSERT INTO source_new(id, authority_id, kind, uri, domain_id, robots, notes)
  SELECT id, authority_id, kind, uri, domain_id, robots, notes FROM source;
DROP TABLE source;
ALTER TABLE source_new RENAME TO source;
CREATE INDEX idx_source_authority ON source(authority_id);

-- channel vocabularies: 001:42-46.
CREATE TABLE channel_new(
  id INTEGER PRIMARY KEY,
  authority_id INTEGER NOT NULL,  -- authority.id
  visibility TEXT NOT NULL CHECK(visibility IN ('public', 'private')),
  kind TEXT NOT NULL CHECK(kind IN ('phone', 'email', 'webform', 'chat', 'in_person')),
  value TEXT NOT NULL,
  details TEXT NOT NULL DEFAULT '',
  purpose TEXT NOT NULL CHECK(purpose IN ('customer_steering', 'our_validation')),
  FOREIGN KEY(authority_id) REFERENCES authority(id)
);
INSERT INTO channel_new(id, authority_id, visibility, kind, value, details, purpose)
  SELECT id, authority_id, visibility, kind, value, details, purpose FROM channel;
DROP TABLE channel;
ALTER TABLE channel_new RENAME TO channel;
CREATE INDEX idx_channel_authority ON channel(authority_id);

-- capture vocabularies: 001:56-63.
CREATE TABLE capture_new(
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
INSERT INTO capture_new(id, authority_id, source_id, captured_at, kind, path, bytes_raw, bytes_stored, sha256, authenticity, trust, notes)
  SELECT id, authority_id, source_id, captured_at, kind, path, bytes_raw, bytes_stored, sha256, authenticity, trust, notes FROM capture;
DROP TABLE capture;
ALTER TABLE capture_new RENAME TO capture;
CREATE INDEX idx_capture_authority ON capture(authority_id, captured_at);

-- fact.topic: the 25 docs/specs/TOPICS.md strings (001:73 is a stale
-- placeholder); fact.status: 001:75.
CREATE TABLE fact_new(
  id INTEGER PRIMARY KEY,
  authority_id INTEGER NOT NULL,  -- authority.id
  capture_id INTEGER NOT NULL,    -- capture.id (provenance)
  topic TEXT NOT NULL CHECK(topic IN ('hours', 'fees', 'forms', 'address', 'parking', 'route_photos', 'rejections', 'contacts', 'po_box', 'walkin', 'mailing', 'appointment', 'payment', 'processing_time', 'jurisdiction', 'legalization_chain', 'website', 'phone_script', 'holiday_hours', 'id_requirements', 'copies', 'translation', 'notarization', 'authentication', 'dropbox')),
  statement TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('proposed', 'accepted', 'superseded', 'rejected')),
  created_by TEXT NOT NULL DEFAULT '',
  created_at INTEGER NOT NULL,
  FOREIGN KEY(authority_id) REFERENCES authority(id),
  FOREIGN KEY(capture_id) REFERENCES capture(id)
);
INSERT INTO fact_new(id, authority_id, capture_id, topic, statement, status, created_by, created_at)
  SELECT id, authority_id, capture_id, topic, statement, status, created_by, created_at FROM fact;
DROP TABLE fact;
ALTER TABLE fact_new RENAME TO fact;
CREATE INDEX idx_fact_lookup ON fact(authority_id, topic, status);

-- batch.status: 001:111.
CREATE TABLE batch_new(
  id INTEGER PRIMARY KEY,
  authority_id INTEGER NOT NULL,  -- authority.id
  channel_id INTEGER NOT NULL,    -- channel.id
  status TEXT NOT NULL CHECK(status IN ('draft', 'scheduled', 'sent', 'done')),
  scheduled_for INTEGER,          -- NULL until scheduled
  notes TEXT NOT NULL DEFAULT '',
  FOREIGN KEY(authority_id) REFERENCES authority(id),
  FOREIGN KEY(channel_id) REFERENCES channel(id)
);
INSERT INTO batch_new(id, authority_id, channel_id, status, scheduled_for, notes)
  SELECT id, authority_id, channel_id, status, scheduled_for, notes FROM batch;
DROP TABLE batch;
ALTER TABLE batch_new RENAME TO batch;

-- task vocabularies: 001:88-93.
CREATE TABLE task_new(
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
INSERT INTO task_new(id, authority_id, kind, title, detail, status, priority, dedup_key, batch_id, created_by, created_at, resolved_at, resolution)
  SELECT id, authority_id, kind, title, detail, status, priority, dedup_key, batch_id, created_by, created_at, resolved_at, resolution FROM task;
DROP TABLE task;
ALTER TABLE task_new RENAME TO task;
CREATE UNIQUE INDEX idx_task_dedup ON task(dedup_key)
  WHERE status IN ('open', 'batched', 'in_progress');
CREATE INDEX idx_task_batch ON task(batch_id);

-- call_session.consent: 001:122.
CREATE TABLE call_session_new(
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
INSERT INTO call_session_new(id, batch_id, authority_id, channel_id, consent, consent_at, script, audio_capture_id, transcript_capture_id, outcome, conducted_by, conducted_at)
  SELECT id, batch_id, authority_id, channel_id, consent, consent_at, script, audio_capture_id, transcript_capture_id, outcome, conducted_by, conducted_at FROM call_session;
DROP TABLE call_session;
ALTER TABLE call_session_new RENAME TO call_session;

-- transcript_note.kind: 001:138.
CREATE TABLE transcript_note_new(
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
INSERT INTO transcript_note_new(id, capture_id, start_byte, end_byte, kind, text, author, created_at)
  SELECT id, capture_id, start_byte, end_byte, kind, text, author, created_at FROM transcript_note;
DROP TABLE transcript_note;
ALTER TABLE transcript_note_new RENAME TO transcript_note;
CREATE INDEX idx_note_capture ON transcript_note(capture_id);

-- audit: the 002 triggers die with DROP TABLE, so drop them first and
-- recreate them identically on the rebuilt table.
DROP TRIGGER audit_no_update;
DROP TRIGGER audit_no_delete;
CREATE TABLE audit_new(
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
INSERT INTO audit_new(id, at, actor, action, entity, entity_id, code_git_commit, content_git_commit, build_id, before_ref, after_ref, reason)
  SELECT id, at, actor, action, entity, entity_id, code_git_commit, content_git_commit, build_id, before_ref, after_ref, reason FROM audit;
DROP TABLE audit;
ALTER TABLE audit_new RENAME TO audit;
CREATE INDEX idx_audit_at ON audit(at);
CREATE TRIGGER audit_no_update BEFORE UPDATE ON audit
BEGIN
  SELECT RAISE(ABORT, 'audit is append-only: UPDATE rejected');
END;
CREATE TRIGGER audit_no_delete BEFORE DELETE ON audit
BEGIN
  SELECT RAISE(ABORT, 'audit is append-only: DELETE rejected');
END;

-- Sentinel parent for runner.rs UNATTRIBUTED_CAPTURE_ID (fact.capture_id = 0).
-- OR IGNORE: an explicit id-0 row can only pre-exist by hand-insert.
INSERT OR IGNORE INTO authority(id, kind, country, region, name, slug)
  VALUES (0, 'office', '', '', 'UNATTRIBUTED sentinel (migration 005; backs fact.capture_id = 0 until ingest passes real ids)', 'sentinel-unattributed-005');
INSERT OR IGNORE INTO capture(id, authority_id, source_id, captured_at, kind, path, bytes_raw, bytes_stored, sha256, authenticity, trust, notes)
  VALUES (0, 0, NULL, 0, 'file', '', 0, 0, '', '', 'rejected', 'sentinel: no capture (migration 005)');

-- Verify every FK BEFORE leaving deferred mode (CHECKs already fired
-- per-row above; only FKs were deferred). A nonzero count aborts the
-- whole migration via the CHECK below instead of shipping bad rows.
CREATE TEMP TABLE fk_check_guard(n INTEGER CHECK(n = 0));
INSERT INTO fk_check_guard SELECT COUNT(*) FROM pragma_foreign_key_check;
DROP TABLE fk_check_guard;

PRAGMA defer_foreign_keys=OFF;
