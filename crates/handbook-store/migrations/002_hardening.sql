-- Migration 002: foundation hardening.
-- Append-only audit: writers only ever INSERT into audit. These two
-- triggers abort any UPDATE or DELETE (including from retention sweeps
-- or ad-hoc SQL) with RAISE(ABORT).
-- No destructive statements in this migration: triggers only, no data
-- changes, no table rewrites.

CREATE TRIGGER audit_no_update BEFORE UPDATE ON audit
BEGIN
  SELECT RAISE(ABORT, 'audit is append-only: UPDATE rejected');
END;

CREATE TRIGGER audit_no_delete BEFORE DELETE ON audit
BEGIN
  SELECT RAISE(ABORT, 'audit is append-only: DELETE rejected');
END;
