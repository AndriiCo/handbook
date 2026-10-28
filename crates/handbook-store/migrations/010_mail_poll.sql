-- 010: single-mailbox Gmail poll cursor (phase B).
--
-- One row (id = 1): the polled address, the historyId watermark, and
-- last-run bookkeeping. Multi-mailbox is a later migration (key by
-- address). Rollback: dropping the table loses the cursor only — the
-- next poll reseeds from getProfile and backfills 7 days.
-- Rollback-safe.

CREATE TABLE mail_poll(
  id INTEGER PRIMARY KEY CHECK (id = 1),
  email TEXT NOT NULL DEFAULT '',
  history_id TEXT NOT NULL DEFAULT '',
  last_poll_at INTEGER NOT NULL DEFAULT 0,
  last_error TEXT NOT NULL DEFAULT ''
);
