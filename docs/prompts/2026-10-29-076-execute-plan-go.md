# Prompt 076 — execute the plan (go)

- Prompt timestamp (UTC): 2026-10-29T11:35:00Z
- Response timestamp (UTC): 2026-10-29T13:07:00Z
- Author: Andrii
- Agent/model: Muse Code (Muse Spark)

## Prompt

go

## Response

Executed the prompt-075 plan with two prep subagents
(triage sheets, loop grounding; both terminal, texts
harvested). Deployed 3× (b5b1c3f, b82c458, b577c10);
prod healthy on b577c10 at handoff.

Step 1: deploy + verify green, incl. Playwright proof
that old `/admin/*` renders client 404 (server 200 shell
is uniform SPA fallback) and API old prefix 404s.
Step 2: PARKED with green preconditions (CA set, 1 valid
unrevoked staff cert to 2027-01, strict mode) — flip
needs the author's own cert-present check first (prompt
070 showed browser non-presentation; recovery needs
operator). Step 3: preflight green (packs clean, counts
0-effectively, meta key+terms rows present since 04:42,
partial mailer/telnyx rows from setup); mutating run
parked — hal is store-read-only, needs operator/author
execution. Step 4: agent closes applied (2,6,12,18,25,
29,30,31); 16 decisions queued for the author. Step 5:
partial unlock rows predate asks (setup session 04:41–
05:30); missing pieces listed for the author. Step 6:
demo-vietnam 5/5, F2 + MCP suites green, live MCP leg
green (VN authority, -32004 unknown), drill 1 3/3,
drill 2 rollback legs 3/3, drill 3 blocked (no nightly
backups; mechanism proven once).

Two findings. (1) FIXED+DEPLOYED: `span.enter()` across
`await` logged spanless/cross-attributed responses under
burst (commit b82c458, burst test fails 3/3 pre-fix,
passes post-fix, 0 bare lines in prod re-burst).
(2) P0 FILED (not fixed): QUIC serve has no shutdown
path — every prod stop hangs 90s then SIGKILLs (9×
today); rapid restarts tripped StartLimit (5/10min) and
caused a ~4-min outage during drill 2, recovered by
waiting out the limit + restart. Mitigations: space
restarts, one-time root grant of `reset-failed` for hal,
then code fix (endpoint.close on drain + test).

## Files changed

- `crates/web/src/main.rs` — span fix + burst test
  (commit b82c458, deployed).
- Spec/playbook OPEN closes (commit b577c10, deployed).
- `docs/ops/drill.md` — log + command modernization
  (commit c1c5463, not deployed — docs only).

## Commits

- `b82c458` span fix; `b577c10` OPEN closes;
  `c1c5463` drill log; (this log; hash on commit)

## Verification

- `cargo xtask ci` exit 0 before the span-fix commit
  (one rerun past the known dial_refusal test flake).
- Prod: 3 deploys + 3 rollbacks + 3 redeploys, all
  verified by stamp/healthz/shell; post-fix burst shows
  76 span lines / 0 bare (was 14 bare).
- Drill evidence in `docs/ops/drill.md`; voice flake +
  u128-JSON latency note carried as follow-ups.

## Follow-ups

- P0: QUIC shutdown path + hal `reset-failed` grant +
  nightly backups + offsite + retention.
- Author: 16 OPEN decisions, unlock values, seed
  confirm, cert-present check, loop/staff-review run.
