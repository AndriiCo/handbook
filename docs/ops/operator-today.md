# Operator — Monday 2026-11-02 (Pacific; updated ~03:00 PST)

Production runs `0.1.0+eb57cdc` (deployed 2026-11-02 ~10:50Z:
stamp + staging 9/10 + demo 3/3 on `eb57cdc`, ship rails green,
`ops-deploy.sh --skip-accept`, no `--allow-today-overlap` — the
8 h window had cleared; prod-before → prod-after identical 5/10,
no regression; previous release `3836bfb` kept for rollback).
Scoreboard **5/10** (P0, GP1, GP4, A2, A5 pass). First red:
GP2-data (vietnam 0/8, usa-wa-sos 1/8 accepted — flips with your
approvals below, no code needed). This deploy ships your
P-ADMIN-IP fix: 403s name the IP, no admin UI without an
allowlisted IP. If your browser 403s, the page itself tells you
which IP (or IPv6 /64) to add — then restart is needed (the
list reads once at boot).

Your lines that flip production rows now (all code is already live):
1. Approve the proposed facts for vietnam + usa-wa-sos in `/review`
   (flips GP2-data, A1, GP2-page — the code fixes for those rows
   shipped in this deploy).
2. Drop brand SVGs into `web/src/lib/assets/brand/` (GP3 now fails
   only on "google/apple button has no logo"; the fold fix shipped).
3. S1: LICENSE text (recommendation: MIT — see below), public repo
   visibility, demo video link (replace the README line yourself on
   2026-11-03; builder must never upload it).
4. Prod `META_API_KEY` is already working (A2 live, 48 ms).

License recommendation (builder input, your call): take MIT. Both are
permissive and judge-familiar, but Apache-2.0's extra weight is its
patent grant + retaliation clause, which only matters if you expect
contributors holding relevant patents — unlikely for an apostille
handbook, and no such contributions exist. MIT is shorter, unambiguous
for reuse of the demo code, and the hackathon default. Do not switch
without saying so; the builder changes nothing here.

Disk fixed 2026-11-01 ~04:05Z (operator cleaned plant `/home`, now 8%):
plant pipeline keeps newest 3 runs + artifacts + logs per job
(`prune_old` in `ops/plant-pipeline/worker.sh`, synced to plant
`bin/`); one-time prune done (4.9G + 855M → 395M + 136M, 18 stamps
kept). Production releases keep live + newest 3 (new step 4 in
`ops-deploy.sh`, staged to `~/apostille/scripts/`); one-time prune
done (9 → 3 dirs, ~390M → 132M, live `d93a58c` + 2 rollback targets
kept). P161 `c2bf13f` submitted clean (`20261101-040732-c2bf13f`,
polling). Newest stamped: `78b8dc9` (P153), `d8dfd02` (P151),
`868cf02` (P150).

Rootless ops stand (run everything as `hal`, never root). The KEK
group-read pre-step from this morning is done — A2 reads prove it.

## Done (verified, struck through)

- [x] Proposals script — ran 01:41Z: vietnam 0 proposed / 0 accepted,
  usa-wa-sos 17 proposed / 0 accepted, analyze proposed 0 new. Nothing
  further to run until the next deploy ships new packs.
- [x] A2 recording — ran 01:41Z, 3 samples installed, prod A2 **PASS**
  (`verdict: pass`, 150 ms, sam-3.1 + muse-spark-1.3, verified 03:50Z).
  The `chgrp apostille /var/lib/apostille/demo_precheck` fix below is
  confirmed working — no action needed.
- [x] A2-fix chgrp line of 2026-11-01 — resolved by the passing verdict above.

## Still open (in order)

1. You said "deployed unit file and allow_ips" — paste back the
   evidence so the blocker closes: the first diff (expect only
   line 6), the empty second diff, `is-active`, and the
   `config-set admin.allow_ips` readback. Until that lands here,
   the builder treats both as done-but-unverified: `ship-release.sh`
   re-checks the unit drift itself and aborts safely on any
   mismatch, and Monday 11-02 is the last code-deploy day. If the
   unit drifted again (a new commit touched
   `docs/ops/apostille.service`), the original line-6 procedure
   stands — ask the builder for it. Note the new binary logs
   `admin.allow_ips effective: …` at INFO on every boot (or
   `loopback only (unset)`), so the journal always answers which
   ranges are live.
2. Approvals — no script: open the admin `/review` (no client
   cert needed anymore — session + your IP in `admin.allow_ips`;
   do the allow_ips step inside item 1 first or you will 403),
   accept every proposed fact EXCEPT 2 per slug (leave 2
   vietnam + 2 usa-wa-sos proposed for the video's approve scene).
   GP2-data needs >= 8 accepted each; currently vietnam 0/8,
   usa-wa-sos 1/8 (verified 16:15Z). This flips
   GP2-data, A1 and GP2-page. Paste back: accepted counts per slug.
   Error guide: "session missing or expired" means
   log in again; `admin access is limited to the configured IP
   range` names your detected IP — add it to `admin.allow_ips`
   (item 1) and retry. A refused browser tab shows a plain 403
   page (no login form) naming the IP, its family (IPv4/IPv6) and
   the transport (tcp/quic): for IPv6 add the /64 it names, and
   restart — the list reads once at boot.
3. One root step, then two hal runs (all staged, paste back each
   output). (a) As root: `bash ~/apostille/scripts/ops-rootless-access.sh`
   — re-run applies the group-write on `/var/lib/apostille` the first
   run predates (prod shows `andrii-ai:andrii-ai drwxr-xr-x`, so hal
   can't write quality.json) and removes the stale 0700 `.loop-creds`.
   (b) As hal: `bash ~/apostille/scripts/ops-gp2-proposals.sh` (KEK
   staging is mktemp now; same stale-dir bug fixed there too), then
   `bash ~/apostille/scripts/ops-a5-eval.sh` (evals path + mktemp +
   store-writability preflight). Paste back: the proposals counts,
   the quality.json lines. Flips A5; proposals feed item 2.
4. S1: switch LICENSE to MIT, make the repo public, link the demo
   video in README (video lands 11-03). Video: record the production
   admin approve scene yourself (session login from your allowlisted
   IP, no cert; builder takes desktop/mobile), upload to YouTube,
   paste the URL back.
   Full-history secret scan 11-01 ~16:20Z: clean (gitleaks 8.30.1,
   385 commits, no leaks; working-tree `secrets-scan` clean, 690 files).
   Scrub list: `docs/ops/scrub-list-2026-11-01.md` — 2 prompt files need
   IP redaction, `security-review-2026-10-27.md` moves unless fixed.
5. GP3 brand assets (scoreboard demands a logo per configured
   provider button; dir still holds only README.md): run
   `cargo xtask brand-assets`, follow its four download steps (Apple,
   Google, Meta, FIDO), then `cargo xtask brand-assets --check
   --minutes 60`. Keep one SVG per provider prod configures in
   `web/src/lib/assets/brand/` (operator-owned, never committed),
   then redeploy — the login page renders text-only buttons until
   they land.
6. ~~Plant disk — fixed, see top.~~ Done.

## Queued (not today)

- New goal (memory `goals-queue.md`, handed over in chat — paste it
  into `.agents/goal.md` yourself; builder never edits it): Muse
  Glimmer self-hosted documents/facts evaluator on plant.
- Inference workstream: REMOVED under the organizer rule (P248 —
  `cargo xtask plant-run` + `ops/plant-inference/` Qwen2.5-VL-7B pilot
  deleted; see docs/ops/plant-inference.md). Its Meta open-weight
  replacement (N6) starts only on your order — say so and the builder
  scaffolds it.

## Queued (Meta API follow-ups — not today, builder slices)

F1–F14 merged to main 11-01 (`f201aa2`, LOG P207; lane eval
0.966–1.000 vs 0.7 baseline, mine pending plant SSH). Six items fell
outside the slice's file list:

1. Terms opt-in (YOUR decision first): atlas transcribe/uploads now
   fail closed until Meta's data-handling terms are read. After you
   confirm, a builder one-liner wires `.with_terms_read(true)` in
   `xtask/src/atlas.rs` `live_client`.
2. `crates/atlas/src/task.rs`: three `temperature: 0.1` sites,
   `reasoning_effort: None` everywhere, and raw model output to
   `debug/extract.txt:382` — align to defaults and confirm the
   debug dump is redacted or remove it.
3. ~~`meta.base_url` — honored in P208 (`draft.rs` + xtask env path, tested).~~ Done.
4. `cached_tokens`: the wire returns it, but `contracts::Usage`
   has no field — surfacing needs contracts + runner + web trace
   changes together.
5. One base URL / key lookup / model-id helper shared across
   crates — needs a new shared crate (restructuring; your call).
6. Staging checkout restored to main; F1–F14 SHAs stay on plant
   as `xfer-f2…xfer-f12` refs for reference.

## Builder state (info only)

Local gates green on main tip `ecb6958` (P216): web 263 tests,
clippy, svelte-check, imports, tailwind, both builds, web unit 99.
Staging 9/10 (only S1 red) on 117ef58. Main past P208 is unstamped:
plant SSH down since ~11-01 19:30Z (`No user exists for uid 501`),
submits queued (`f201aa2` → `ecb6958`, 9 SHAs). Prod 1/10 in this
sandbox's scoreboard run (browser rows fail: local Chromium cannot
launch here, Mach IPC denied — environmental; data rows fail on
approvals: vietnam 0/8, usa-wa-sos 1/8). P214 adds
`GET /p2/submissions` (orders list, newest-first, per-customer) and
wires `/orders` to it — the page no longer shows placeholder rows.
P216 adds `GET /p2/forms?authority=` and a "Save draft to my orders"
step on `/order` review (401 → sign-in prompt, 201 → order link).
P219 scopes the CUJ locator to Fees (plant gate red fixed).
Plant back: `5bb462a` stamped `ci_green: true`, staging exact at
`0.1.0+5bb462a` rescored 9/10 (only S1 red) + live demo 3/3 —
staging-green. Deploy waits on your item 1 (unit file) above.
P221 (tip `bce01ad`, on plant now): admin mTLS removed, IP
allowlist `admin.allow_ips` in — set it (item 1) before ANY
deploy of this SHA, or you 403 yourself out.
P223 guide v1 (tip `2cce16d`): staging exact rescored 9/10
(only S1 red) + live demo 3/3 — staging-green, supersedes
`5bb462a` as the deploy candidate. Ship needs item 1 (unit
file) + prod `admin.allow_ips` first.
WA form template (goal priority 5): standalone Dec-2024 form URL
re-verified dead 11-01 (404 page); live 6.2025 CORPS Records Request
(URL + $15/40234 evidence in egypt-consular.md V-5a) covers
business-entity apostilles, not the standalone request form. Say
which official form to model (or paste its fields) and the builder
scaffolds `forms/usa-wa-sos/wa-apostille-request/` from it.
Meta follow-ups (lane "Queued" section above, accepted at merge):
terms opt-in stays your decision; `task.rs` temperature/effort,
`meta.base_url` (draft.rs, xtask env-key path) and `cached_tokens`
(contracts + runner + trace) are builder slices post-merge; the
shared client crate is your call.
CORRECTION 2026-11-02 ~00:50Z: "plant SSH down" was a misdiagnosis
— plant was never reached. This sandbox cannot execute `ssh`/`scp`
at all (`ssh -V` alone dies with `No user exists for uid 501`;
uid 501 unresolvable here, no paramiko fallback). Plant itself is
likely fine. Until the sandbox is fixed, run plant submits and the
staging tunnel from your own shell: `ops/plant-pipeline/submit.sh
1454c63dfc18abe5331a9a577c53db919aaf051e`, then staging acceptance
per docs/ops/staging.md "Steps". Queued behind that: stamps for
P240–P249, staging scoreboard + live demo, Monday deploy candidate.
