# Prompt 073 — enrich tracing events, then deploy

- Prompt timestamp (UTC): 2026-10-29T07:17:00Z
- Response timestamp (UTC): 2026-10-29T07:29:00Z
- Author: Andrii
- Agent/model: Muse Code (Muse Spark)

## Prompt

Please enrich the existing tracing events. Deploy after.

## Response

Implemented the minimal enrichment from prompt 072 and
deployed it to prod.

Design (kept `tracing`, no OTel SDK): `RequestTransport`
extension stamped `"tcp"` by a serve-time outer layer and
`"quic"` by the h3 bridge; branch via middleware state
(`"public"`/`"admin"` at the 5 layer sites); route from
axum `MatchedPath` at middleware entry (`"unmatched"` for
fallbacks); 5xx at ERROR with `error_kind` from a response
extension (`store_unavailable` first kind, else
`unclassified`). Docs (`prod.md`, `threat-model.md`) updated
to the new shape.

Assumptions: prod-direct deploy is correct (staging box not
landed — `staging-plan.md` auto-relaxes to prod-direct with
smokes + atomic rollback until Oct 31); `--skip-accept`
under the established escape hatch (empty build table, no
content/schema surface — same as prompts 064/066/068); no
migration notes (no schema change); rollback-safe.

## Files changed

- `crates/web/src/main.rs` — markers, middleware, serve
  wraps, `store_unavailable` kind, 4 tests.
- `crates/web/src/quic.rs` — `"quic"` stamp.
- `crates/web/src/admin.rs`, `admin_workers.rs`,
  `admin_voice.rs` — branch labels.
- `crates/web/Cargo.toml`, `Cargo.lock` — `json` feature
  (log-capture test) + `tracing-serde`.
- `docs/ops/prod.md`, `docs/ops/threat-model.md` — new
  event shape.

## Commits

- `687ba1a` (this work)
- (this log; hash on commit)

## Verification

- `cargo test -p web`: 220/0 (4 new: log-capture schema
  test, `store_unavailable` kind, TCP + QUIC marking).
- `cargo xtask ci`: exit 0 (fmt, clippy `-D warnings`,
  workspace + node + e2e + tailwind).
- Deploy: zigbuild ELF x86-64, stamp `687ba1a`;
  `xtask deploy` → `deployed 687ba1a (previous: 5591253,
  pruned: 2ad14c4)`, smoke green.
- Prod live: `/healthz` `0.1.0+687ba1a`; journal shows
  `transport=tcp branch=public route=/apostille/healthz`
  and `branch=admin route=.../login/start` (403 probe,
  not 500); 404 maps to the `/apostille/api/{*path}`
  template. Pre-deploy journal confirmed the original
  bug in the wild (QUIC login/start 500 next to TCP 200).

## Follow-ups

- Grow `error_kind` taxonomy at failure sites as new 5xx
  classes appear (extractor rejections still log
  `unclassified` — transport+route+status splits them).
- Staging-gated deploys + `xtask accept` when the staging
  box lands (Oct 31 per plan).
- Pushed nothing (push only on request); branch remains
  ahead of origin/main.
