-- 009: per-source watch fetch state (B5 sweep).
--
-- Each website `source` row carries its poll schedule plus the last
-- fetch's validators and content fingerprint, so the sweep can skip
-- unchanged pages without ingesting (conditional GET → 304, content
-- hash compare, SimHash near-dup gate). Rollback: dropping these
-- columns loses fetch state only — the next sweep re-fetches every
-- URL unconditionally. Rollback-safe.

ALTER TABLE source ADD COLUMN poll_cadence_sec INTEGER NOT NULL DEFAULT 86400;
ALTER TABLE source ADD COLUMN etag TEXT NOT NULL DEFAULT '';
ALTER TABLE source ADD COLUMN last_modified TEXT NOT NULL DEFAULT '';
ALTER TABLE source ADD COLUMN content_hash TEXT NOT NULL DEFAULT '';
ALTER TABLE source ADD COLUMN simhash INTEGER NOT NULL DEFAULT 0;
ALTER TABLE source ADD COLUMN last_checked_at INTEGER NOT NULL DEFAULT 0;

-- Due-URL enumeration: per-source cadence against its last check.
CREATE INDEX IF NOT EXISTS idx_source_due ON source(poll_cadence_sec, last_checked_at);
