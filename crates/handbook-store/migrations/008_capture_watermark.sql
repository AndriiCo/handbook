-- Migration 008: capture-id watermark for the Option-B analyze scan.
-- Additive only: one nullable-by-default column, no data changes.
-- get_capture_watermark() reads it (0 = nothing analyzed yet);
-- advance happens with MAX() inside each capture's transaction.

ALTER TABLE watermark ADD COLUMN last_capture_id INTEGER NOT NULL DEFAULT 0;
