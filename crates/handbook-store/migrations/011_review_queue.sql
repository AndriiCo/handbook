-- 011: review queue + rejection-feedback suppression (§11).
--
-- suppress_rule: staff-authored path-B rules (revocation is a
-- timestamp, never a delete). suppression: append-only log of every
-- analyzer skip. fact gains its citation home (offsets into its own
-- capture sidecar; -1/-1 = uncited) plus the model's quarantined
-- summary ('' = predates capture). Rollback-safe: new tables ignore
-- cleanly; fact columns are read-optional.

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

ALTER TABLE fact ADD COLUMN citation_start INTEGER NOT NULL DEFAULT -1;
ALTER TABLE fact ADD COLUMN citation_end INTEGER NOT NULL DEFAULT -1;
ALTER TABLE fact ADD COLUMN reasoning TEXT NOT NULL DEFAULT '';
