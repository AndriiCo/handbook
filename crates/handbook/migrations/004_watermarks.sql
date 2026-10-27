-- Migration 004: per-authority analysis watermark (S2 lane).
-- One row per authority, upserted on every analysis run. No
-- destructive statements: new table only, no data changes.
-- (Numbering provisional in the worktree; parent reconciles.)

CREATE TABLE watermark(
  authority_slug TEXT PRIMARY KEY,
  last_run_at INTEGER NOT NULL,
  last_prompt_version TEXT NOT NULL
);
