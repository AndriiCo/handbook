# 004 — Design decisions + Phase 1 implementation

- Prompt timestamp (UTC): 2026-10-26 ~18:30 (approximate, same session)
- Response timestamp (UTC): 2026-10-26 18:41:01
- Author: Andrii Tereshchenko
- Agent/model: Muse Code, powered by Meta Muse Spark

## Prompt

1. Content pages and any matherials used are separate from this repo. This repo may have schema for SQL database and all migrations (without customer specific data) but not data. 2. It will be using Meta Models APIs. 3. Yes. 365 days for voice data, unlimited for actual transcript based on it. Transcripts can be annotated by humamns to ignore some parts of it or fix transcription errors / mispellings as easy as possible before voice data will be lost. 4. Pick Washington state first. But make it global. MVP is Canada / Mexico / USA, but ultimate goal is to have all countries on Earth (and Mars eventually in some future, if Fedex will dekiver to then) to be on handbook.

## Response

Recorded the four decisions in `docs/design.md` and — treating the
answers as go-ahead for the previously teed-up Phase 1 — implemented the
schema + store + `xtask` foundation. Assumption: no repo seed data of any
kind (even public authority rows), since the repo holds schema/migrations
only; Washington is the first *runtime* row, entered via `xtask`.

Design updates (`6e9be5c`): repo-scope paragraph (§0); `audio` capture
kind + `transcript_note` table + `audio_capture_id` on `call_session`;
two commit hashes on `build`/`audit`; retention section (365-day voice,
unlimited transcripts, `retention-sweep` semantics); Meta Models API
transcription (§5, §8); traceability rows; §11 rewritten as decided
(materials repo location, API auth details, and photo retention still
open). Fixed the table count (thirteen, was misstated as eleven).

Phase 1 code (`ef51f9f`):

- `crates/handbook/migrations/001_init.sql` — 13-table schema, unix
  timestamps, plain-INTEGER references (no FK enforcement), task dedup
  partial unique index.
- `crates/handbook/src/store.rs` — `Store`: init dirs, `blob/xx/yy/…`
  content-addressed read/write with sha256 dedup, `sha256_hex`.
- `crates/handbook/src/db.rs` — `open`, idempotent `migrate`
  (`user_version`), `add_authority`, `retention_sweep` (deletes expired
  session-linked audio blobs + rows, nulls links, writes one audit row
  each; transcripts/annotations/consent untouched).
- `xtask`: `init`, `add-authority`, `retention-sweep` (hand-rolled flags;
  `--store`, default `store/`); `xtask` now depends on the `handbook` lib.
- Deps added: `rusqlite` (bundled, FTS5), `sha2`, dev-only `tempfile`.

Files changed: `docs/design.md`, `Cargo.lock`,
`crates/handbook/Cargo.toml`, `crates/handbook/src/lib.rs`,
`crates/handbook/src/{db,store}.rs`,
`crates/handbook/migrations/001_init.sql`, `xtask/Cargo.toml`,
`xtask/src/main.rs`, `docs/prompts/README.md`, this entry.

Verification (all observed this turn):

- `cargo xtask ci` — green (fmt, clippy `-D warnings`, 7 tests pass).
- Two self-caught issues fixed along the way: `cargo fmt` diffs, and a
  test bug (fake `"t"` digest tripped the `blob_path` length assert —
  replaced with a real blob).
- End-to-end smoke in /tmp (never in the repo): `init` → `add-authority`
  (WA Secretary of State pilot row, id 1) → `retention-sweep` (0
  deleted) — all succeed; store layout (`blob/`, `mbox/`, `text/`,
  `warc/`, `handbook.sqlite`) created as designed.
- `git status` clean after commits; no store data in the repo. Note:
  `store/` is not yet in `.gitignore` (left for the author to approve);
  only named files are ever staged.

No push (not asked this turn, per AGENTS.md §5).

## Commits

- `6e9be5c` — Record design decisions from prompt 004 (design doc).
- `ef51f9f` — Phase 1: SQLite schema, file store, xtask foundation.
- This entry's own commit: the commit adding this file (see
  `git log -- docs/prompts/2026-10-26-004-decisions-phase1.md`).

## Follow-ups

- Approve adding runtime `store/` to `.gitignore` (schema-only repo).
- Decide content-repo location; Meta Models API endpoint/auth spec.
- Decide customer-photo retention (only voice decided so far).
- Phase 2: ingest (`fetch-url` with WARC+TLS capture, mbox import, file
  import, text sidecars + FTS indexing).
