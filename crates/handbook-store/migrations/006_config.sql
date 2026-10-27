-- Migration 006: encrypted-config tables (docs/specs/secure-config.md §1.4).
-- Ciphertext rows live in handbook.sqlite (one DB, one backup); the H stream
-- owns the XChaCha20-Poly1305 envelope + KEK handling, the admin API, and the
-- xtask enroll/rotate/verify commands. Every insert/update MUST also write an
-- audit row (entity='config', before_ref/after_ref = key+version only, NEVER
-- values) -- enforced by code review, not by trigger.
-- STRICT tables per the spec (new-code default): wrong-type writes fail.
-- New tables only: no data changes, no table rewrites.

CREATE TABLE config(
  key TEXT PRIMARY KEY,             -- e.g. 'fedex.account_id'
  type TEXT NOT NULL                -- 'bool' | 'text' | 'secret'
    CHECK(type IN ('bool', 'text', 'secret')),
  value_enc BLOB NOT NULL,          -- nonce(24) || XChaCha20-Poly1305 ct
  enc_version INTEGER NOT NULL,     -- KEK generation
  updated_at INTEGER NOT NULL,      -- unix seconds
  updated_by TEXT NOT NULL          -- audit actor
) STRICT;

CREATE TABLE config_kek(
  version INTEGER PRIMARY KEY,      -- 1, 2, ...
  cred_path TEXT NOT NULL,          -- sealed .cred path for this generation
  created_at INTEGER NOT NULL,
  retired_at INTEGER                -- NULL = active
) STRICT;
