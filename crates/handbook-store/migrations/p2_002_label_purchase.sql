-- P2 PII store, file 2: label-purchase options + profile defaults.
-- Additive only: one new table, three new columns with defaults.
-- Backfill: existing label rows read as prepaid 4x6 PDFs, which is
-- what every writer produced before this migration.

-- Saved per-customer label choices (Option A format + Option B paper).
CREATE TABLE profile_label_defaults(
  customer_ref TEXT PRIMARY KEY,
  option_a_format TEXT NOT NULL DEFAULT 'four_by_six',
  option_b_paper TEXT NOT NULL DEFAULT 'four_by_six',
  updated_at INTEGER NOT NULL
);

ALTER TABLE shipping_label
  ADD COLUMN purchase_option TEXT NOT NULL DEFAULT 'prepaid'
    CHECK(purchase_option IN ('printable', 'prepaid'));
ALTER TABLE shipping_label
  ADD COLUMN label_format TEXT NOT NULL DEFAULT 'four_by_six';
ALTER TABLE shipping_label
  ADD COLUMN label_sha256 TEXT NOT NULL DEFAULT '';
ALTER TABLE shipping_label
  ADD COLUMN content_type TEXT NOT NULL DEFAULT 'application/pdf';
