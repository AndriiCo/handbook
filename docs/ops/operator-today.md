# Operator — Sunday 2026-11-01 (Pacific; updated ~09:15 PDT)

Production runs `0.1.0+69ca7fa` (deployed 2026-11-01 ~06:35Z on your
"deploy now" order, `--allow-today-overlap`) — scoreboard **5/10**
(P0, GP1, GP4, A2, A5 pass; see `web/shots/prod-before.json` →
`web/shots/prod-after.json`, no regression). First red: GP2-data
(vietnam 0/8, usa-wa-sos 1/8 accepted). Staging 9/10 (only S1 red) on
117ef58 (stamp + live demo 3/3 — Monday's candidate); tunnel live
during builder turns.

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

1. DEPLOY BLOCKER (root, one comment line, no restart): Sunday's
   staging-green candidate `5bb462a` (stamp `ci_green: true`, staging
   9/10 + live demo 3/3) cannot ship until the installed unit file
   matches the repo — `ship-release.sh` aborts on any drift, and a
   rollback would not restore the unit (AGENTS.md §7). The whole
   difference is line 6, comment-only:
   `runbook-release.md` → `staging.md § Steps` (P180 doc retarget).
   From your checkout, as hal: `scp docs/ops/apostille.service
   andrii.ai:/tmp/apostille.service.new` and check
   `sha256sum /tmp/apostille.service.new` =
   `a5c416fb67ccafa52a5730627d057e77030b2eebe8f5ff0b85c5f116e809f964`.
   Then as root: `diff /etc/systemd/system/apostille.service
   /tmp/apostille.service.new` (expect only line 6),
   `install -m 644 /tmp/apostille.service.new
   /etc/systemd/system/apostille.service && systemctl daemon-reload`
   (no restart, no downtime), then `systemctl cat apostille | diff -
   /tmp/apostille.service.new` (expect empty) and
   `systemctl is-active apostille`. Paste back: the first diff, the
   empty second diff, is-active. Monday 11-02 is the last code-deploy
   day — this gates it. Then, BEFORE the deploy (old binaries
   ignore the unknown row; the new binary reads it at boot), set
   prod `admin.allow_ips` to cover the IP you approve from — the
   default is loopback-only and would 403 you the moment the new
   binary starts. As hal on the server (same KEK-staging pattern
   as `ops-gp2-proposals.sh`: copy `/etc/apostille/handbook-kek`
   to a mktemp dir, `trap` cleanup):
   `/srv/apostille/current/xtask config-set --store
   /var/lib/apostille --kek-file <staged-kek> --key
   admin.allow_ips --type text --value "<your-ip-or-range>"`
   (find your IP in any 403 message, or `curl ifconfig.me`).
   Verify with the readback the command prints, then tell the
   builder: it re-runs ship + deploy, and the new binary logs a
   WARNING at boot if the value ever goes bad (loopback-only).
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
   (item 1) and retry.
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
- Inference workstream (full design at docs/ops/plant-inference.md):
  `cargo xtask plant-run` + `ops/plant-inference/` Qwen2.5-VL-7B
  pilot (uncompiled/unproven — needs `cargo clippy -p xtask`, a live
  `setup`, and the `job-001` run with its three numbers).

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
