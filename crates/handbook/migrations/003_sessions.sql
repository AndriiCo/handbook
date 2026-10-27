-- Migration 003: DEMO-ONLY session table.
-- Demo customers log in with no password (roadmap S3 Day 2 slice);
-- passkey/OAuth replaces this before any real PII flows. Never store
-- real credentials or customer secrets here.
CREATE TABLE demo_session(
  token TEXT PRIMARY KEY,
  customer_ref TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL
);
