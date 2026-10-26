-- Migration 001: initial evidence-store schema.
-- See docs/design.md §1. All timestamps are INTEGER unix seconds.
-- No FOREIGN KEY enforcement: references are plain INTEGERs checked in code.

-- What every row is indexed by: the office / jurisdiction it applies to.
CREATE TABLE authority(
  id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL,           -- country | state | province | office
  country TEXT NOT NULL,
  region TEXT NOT NULL DEFAULT '',
  name TEXT NOT NULL,
  slug TEXT NOT NULL UNIQUE
);

-- Official vs fake domains. The anti-spoofing list.
CREATE TABLE domain(
  name TEXT PRIMARY KEY,
  authority_id INTEGER,         -- authority.id, NULL when unattributed
  status TEXT NOT NULL,         -- official | known_thirdparty | unverified | fake
  evidence_capture_id INTEGER,  -- capture.id proving the status
  notes TEXT NOT NULL DEFAULT ''
);

-- Tracked origins of information.
CREATE TABLE source(
  id INTEGER PRIMARY KEY,
  authority_id INTEGER NOT NULL,  -- authority.id
  kind TEXT NOT NULL,             -- website_official | website_thirdparty
                                  -- | email_official | email_customer
                                  -- | phone | visit | comment | document
  uri TEXT NOT NULL,              -- URL, email address, phone number, ...
  domain_id TEXT,                 -- domain.name, NULL when not applicable
  robots TEXT NOT NULL DEFAULT '',-- robots.txt verdict + checked_at (websites)
  notes TEXT NOT NULL DEFAULT ''
);
CREATE INDEX idx_source_authority ON source(authority_id);

-- Support channels, split by audience.
CREATE TABLE channel(
  id INTEGER PRIMARY KEY,
  authority_id INTEGER NOT NULL,  -- authority.id
  visibility TEXT NOT NULL,       -- public | private
  kind TEXT NOT NULL,             -- phone | email | webform | chat | in_person
  value TEXT NOT NULL,            -- number, address, URL, office address
  details TEXT NOT NULL DEFAULT '', -- extensions, hours, team/volume notes
  purpose TEXT NOT NULL           -- customer_steering | our_validation
);
CREATE INDEX idx_channel_authority ON channel(authority_id);

-- One collected artifact. Metadata here, bytes in the file store.
CREATE TABLE capture(
  id INTEGER PRIMARY KEY,
  authority_id INTEGER NOT NULL,  -- authority.id
  source_id INTEGER,              -- source.id, NULL when unattributed
  captured_at INTEGER NOT NULL,
  kind TEXT NOT NULL,             -- warc | file | scan | photo | video
                                  -- | audio | email | transcript | comment
  path TEXT NOT NULL,             -- location in the file store
  bytes_raw INTEGER NOT NULL,
  bytes_stored INTEGER NOT NULL,
  sha256 TEXT NOT NULL,
  authenticity TEXT NOT NULL DEFAULT '',  -- JSON: TLS or DMARC/DKIM evidence
  trust TEXT NOT NULL,            -- official | staff_verified | community | rejected
  notes TEXT NOT NULL DEFAULT ''
);
CREATE INDEX idx_capture_authority ON capture(authority_id, captured_at);

-- Atomic, cited claims. The unit the handbook is built from.
CREATE TABLE fact(
  id INTEGER PRIMARY KEY,
  authority_id INTEGER NOT NULL,  -- authority.id
  capture_id INTEGER NOT NULL,    -- capture.id (provenance)
  topic TEXT NOT NULL,            -- address | hours | fee | form | ...
  statement TEXT NOT NULL,        -- one short claim
  status TEXT NOT NULL,           -- proposed | accepted | superseded | rejected
  created_by TEXT NOT NULL DEFAULT '',
  created_at INTEGER NOT NULL
);
CREATE INDEX idx_fact_lookup ON fact(authority_id, topic, status);

-- Full text over extracted text sidecars, for conflict hunting.
CREATE VIRTUAL TABLE capture_fts USING fts5(capture_id UNINDEXED, body);

-- Correction / validation work items for handbook pages.
CREATE TABLE task(
  id INTEGER PRIMARY KEY,
  authority_id INTEGER NOT NULL,  -- authority.id
  kind TEXT NOT NULL,             -- validate_fact | correction | call
                                  -- | email_check | visit
  title TEXT NOT NULL,
  detail TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL,           -- open | batched | in_progress
                                  -- | blocked | done | rejected
  priority INTEGER NOT NULL DEFAULT 0,
  dedup_key TEXT NOT NULL,        -- sha256(authority_id + normalized topic)
  batch_id INTEGER,               -- batch.id, NULL until batched
  created_by TEXT NOT NULL DEFAULT '',
  created_at INTEGER NOT NULL,
  resolved_at INTEGER,            -- NULL until resolved
  resolution TEXT NOT NULL DEFAULT ''
);
CREATE UNIQUE INDEX idx_task_dedup ON task(dedup_key)
  WHERE status IN ('open', 'batched', 'in_progress');
CREATE INDEX idx_task_batch ON task(batch_id);

-- One outbound contact holding many questions.
CREATE TABLE batch(
  id INTEGER PRIMARY KEY,
  authority_id INTEGER NOT NULL,  -- authority.id
  channel_id INTEGER NOT NULL,    -- channel.id
  status TEXT NOT NULL,           -- draft | scheduled | sent | done
  scheduled_for INTEGER,          -- NULL until scheduled
  notes TEXT NOT NULL DEFAULT ''
);

-- Recorded, transcribed validation calls.
CREATE TABLE call_session(
  id INTEGER PRIMARY KEY,
  batch_id INTEGER,               -- batch.id, NULL for ad-hoc calls
  authority_id INTEGER NOT NULL,  -- authority.id
  channel_id INTEGER NOT NULL,    -- channel.id
  consent TEXT NOT NULL DEFAULT 'unknown', -- unknown | granted | refused
  consent_at INTEGER,             -- NULL until answered
  script TEXT NOT NULL DEFAULT '',-- questions the app prepared
  audio_capture_id INTEGER,       -- capture.id (kind='audio'), 365-day retention
  transcript_capture_id INTEGER,  -- capture.id (kind='transcript'), kept forever
  outcome TEXT NOT NULL DEFAULT '',
  conducted_by TEXT NOT NULL DEFAULT '',
  conducted_at INTEGER            -- NULL until conducted
);

-- Human annotations on a transcript: ignore ranges and corrections.
CREATE TABLE transcript_note(
  id INTEGER PRIMARY KEY,
  capture_id INTEGER NOT NULL,    -- capture.id (kind='transcript')
  start_byte INTEGER NOT NULL,
  end_byte INTEGER NOT NULL,
  kind TEXT NOT NULL,             -- ignore | correction
  text TEXT NOT NULL,             -- replacement text or reason
  author TEXT NOT NULL DEFAULT '',
  created_at INTEGER NOT NULL
);
CREATE INDEX idx_note_capture ON transcript_note(capture_id);

-- Handbook/algorithm build manifest. Two repos, two hashes.
CREATE TABLE build(
  id INTEGER PRIMARY KEY,
  at INTEGER NOT NULL,
  code_git_commit TEXT NOT NULL,
  content_git_commit TEXT,        -- NULL until the content repo exists
  build_no INTEGER NOT NULL UNIQUE,
  facts_hash TEXT NOT NULL,
  notes TEXT NOT NULL DEFAULT ''
);

-- Append-only audit of everything that changed content or code output.
CREATE TABLE audit(
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
  reason TEXT NOT NULL DEFAULT ''
);
CREATE INDEX idx_audit_at ON audit(at);
