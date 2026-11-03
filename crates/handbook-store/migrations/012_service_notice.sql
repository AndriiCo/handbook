-- Migration 012: widen the fact.topic CHECK by one frozen topic,
-- `service_notice` (closures, outages, suspended services; operator
-- sign-off 2026-11-03, docs/specs/TOPICS.md).
--
-- SQLite cannot ALTER a CHECK, so this is the documented 12-step
-- rebuild (lang_altertable.html §7) for the one table that changes,
-- in the 005 shape: CREATE `fact_new` -> INSERT..SELECT -> DROP ->
-- RENAME, inside migrate()'s single transaction with
-- `foreign_keys=ON` throughout.
--
-- - The temp name lives on the NEW table, so the RENAME rewrites no
--   other table's REFERENCES (`authority.ship_to_fact_id REFERENCES
--   fact(id)` keeps pointing at `fact`).
-- - DROP TABLE runs an implicit DELETE that would trip
--   `authority.ship_to_fact_id`; `defer_foreign_keys` (settable
--   mid-transaction, unlike `foreign_keys`) holds those checks, and
--   the guard table below re-verifies every FK via
--   `pragma_foreign_key_check` BEFORE defer is turned back off
--   (turning it off discards pending checks). Any dangling row
--   aborts the migration and the transaction rolls back whole.
-- - Rows keep their ids (INTEGER PRIMARY KEY copied verbatim), so
--   every reference into `fact` (authority, notify outbox, form
--   templates' fact_ids_json, audit entity ids) stays valid.
-- - `idx_fact_lookup` dies with the old table and is recreated.
--   No trigger or view names `fact`.
--
-- The new CHECK is a strict superset of v11's, so every existing row
-- satisfies it; the INSERT would abort the migration otherwise.

PRAGMA defer_foreign_keys=ON;

CREATE TABLE fact_new(
  id INTEGER PRIMARY KEY,
  authority_id INTEGER NOT NULL,  -- authority.id
  capture_id INTEGER NOT NULL,    -- capture.id (provenance)
  topic TEXT NOT NULL CHECK(topic IN ('hours', 'fees', 'forms', 'address', 'parking', 'route_photos', 'rejections', 'contacts', 'po_box', 'walkin', 'mailing', 'appointment', 'payment', 'processing_time', 'jurisdiction', 'legalization_chain', 'website', 'phone_script', 'holiday_hours', 'id_requirements', 'copies', 'translation', 'notarization', 'authentication', 'dropbox', 'service_notice')),
  statement TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('proposed', 'accepted', 'superseded', 'rejected')),
  created_by TEXT NOT NULL DEFAULT '',
  created_at INTEGER NOT NULL,
  citation_start INTEGER NOT NULL DEFAULT -1,
  citation_end INTEGER NOT NULL DEFAULT -1,
  reasoning TEXT NOT NULL DEFAULT '',
  FOREIGN KEY(authority_id) REFERENCES authority(id),
  FOREIGN KEY(capture_id) REFERENCES capture(id)
);
INSERT INTO fact_new(id, authority_id, capture_id, topic, statement, status,
                     created_by, created_at, citation_start, citation_end, reasoning)
  SELECT id, authority_id, capture_id, topic, statement, status,
         created_by, created_at, citation_start, citation_end, reasoning
  FROM fact;
DROP TABLE fact;
ALTER TABLE fact_new RENAME TO fact;
CREATE INDEX idx_fact_lookup ON fact(authority_id, topic, status);

CREATE TEMP TABLE fk_check_guard(n INTEGER CHECK(n = 0));
INSERT INTO fk_check_guard SELECT COUNT(*) FROM pragma_foreign_key_check;
DROP TABLE fk_check_guard;

PRAGMA defer_foreign_keys=OFF;
