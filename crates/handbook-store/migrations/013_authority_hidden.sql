-- Migration 013: authority visibility, location columns and document
-- routing (additive).
--
-- Every statement is an ADD COLUMN with a default or a CREATE ... IF
-- NOT EXISTS, so no table is rebuilt and every existing row, id and
-- foreign key stays as it is (not a HARD migration; no older binary
-- opens a v13 store, so the operator backup is still the rollback).
--
-- - `hidden_at`: unix seconds a pack load or staff hid the row; NULL
--   means public. Every public read filters `hidden_at IS NULL`; the
--   row, its facts and its audit trail stay for staff.
-- - `iso2`, `subdivision` (ISO 3166-1 alpha-2 and ISO 3166-2, e.g.
--   `US`, `US-WA`), `level` (federal | state | regional | local |
--   consular): where an office sits, for flags and location pages.
-- - `name_en`, `aliases` (newline-separated): search terms beside the
--   display name ("Bayern", "Deutschland").
-- - `authority_path`: readable addresses (`usa/new_york/chemung`),
--   already created at boot by the web binary with this exact DDL;
--   owned by the store from here on so pack loads can write it.
-- - `authority_scope`: document routing a pack proposes ("this office
--   apostilles vital records issued in US-WA, after a certified copy",
--   read at `source_uri`). Not a fact: place pages show a route as
--   unverified until staff accept a `jurisdiction` fact
--   `doc_rule: <doc_class> …` on the same office; each pack load also
--   opens a review task for it. `doc_class` is checked in code (a
--   closed list a later release may grow without a table rebuild).

ALTER TABLE authority ADD COLUMN hidden_at INTEGER;
ALTER TABLE authority ADD COLUMN iso2 TEXT NOT NULL DEFAULT '';
ALTER TABLE authority ADD COLUMN subdivision TEXT NOT NULL DEFAULT '';
ALTER TABLE authority ADD COLUMN level TEXT NOT NULL DEFAULT ''
  CHECK(level IN ('', 'federal', 'state', 'regional', 'local', 'consular'));
ALTER TABLE authority ADD COLUMN name_en TEXT NOT NULL DEFAULT '';
ALTER TABLE authority ADD COLUMN aliases TEXT NOT NULL DEFAULT '';

CREATE TABLE IF NOT EXISTS authority_path (
    path TEXT PRIMARY KEY,
    authority_slug TEXT NOT NULL,
    is_primary INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL DEFAULT 0
);
CREATE UNIQUE INDEX IF NOT EXISTS authority_path_one_primary
    ON authority_path(authority_slug) WHERE is_primary = 1;

CREATE TABLE IF NOT EXISTS authority_scope (
    id INTEGER PRIMARY KEY,
    authority_id INTEGER NOT NULL REFERENCES authority(id),
    doc_class TEXT NOT NULL,
    issued_in TEXT NOT NULL DEFAULT '',
    prereq TEXT NOT NULL DEFAULT '',
    source_uri TEXT NOT NULL,
    pack TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL DEFAULT 0
);
CREATE UNIQUE INDEX IF NOT EXISTS authority_scope_one
    ON authority_scope(authority_id, doc_class, issued_in);

-- The mail poller's catch-all row (`mail-inbox`, E-2) attributes
-- unmatched inbound mail; it is internal and never a public dossier.
INSERT INTO audit(at, actor, action, entity, entity_id, reason)
  SELECT CAST(strftime('%s', 'now') AS INTEGER), 'migration-013', 'authority_hide',
         'authority', id, 'mail-inbox: internal catch-all for unmatched mail'
  FROM authority WHERE slug = 'mail-inbox';
UPDATE authority SET hidden_at = CAST(strftime('%s', 'now') AS INTEGER)
  WHERE slug = 'mail-inbox';
