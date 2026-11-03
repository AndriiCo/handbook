# Operator — MISSION WIN (updated Mon 11-02 ~17:20 PST)

Plan: `docs/ops/mission-win.md` (you approved it 11-02). Production is
still `0.1.0+4dcd234`. On `main` since (all plant-gated green so far):
- P315 meta-1 prompt v2 (selectable; v1 stays default — live eval at the
  merge: v1 quote_found 1.000 / value_in_quote 1.000 / topic 0.667;
  v2 0.965 / 0.947 / 0.754, so v2 is not promoted).
- P316-P318 lane hooks, 26 config keys, contracts, admin console shell
  (sidebar, ⌘K, env badge, live dot), admin `/version`.
- P319 per-attempt Meta telemetry (`model_call`), `chat_json`, daily cap.
- P320 `/admin/stats`, authority kind + verified_at; P322 lane admin
  routes behind the admin guard stack.
- P321 `cargo xtask judge` (doctor + fresh boot + tour, docs/judges.md).
- P323 Meta page plans: Spark compose → validator → Wonder entailment →
  staff accept → public `/plan`.
- P324 SA-26 dashboard + review console + trace latency; SA-27 "Your
  apostille plan" dossier + runtime-only cleanup.
- P325 orders API (dark: free address label live, paid tiers paused,
  FedEx disabled with a cited PO-box fact); SA-28 /order + cabinet.
Plant: two old pipeline runs pruned (/home 16% used, ~200 G free).

## Open now, in order

**T1 DEPLOYED (~18:35 PST, on your "VPN is up, ship T1"):** production
runs `0.1.0+e25fd3e` (previous `4dcd234`, kept for rollback; pruned
`eb57cdc`). Flags announced per §7: `--skip-accept --allow-today-overlap`.
Prod-before 5/10 → prod-after 5/10, zero regressions (P0, GP1, GP4, A2,
A5 pass; every red row's detail identical). Ships: admin console shell +
dashboard + review console, Meta page plans (`/plan`), offices, orders
(free address label; paid tiers paused), /order + cabinet, runtime-only
public data, telemetry. Red rows still need your approvals in
production `/review` (GP2-data, A1, GP2-page) and the logos (GP3).


0. **Your edits (agents never touch these files).** Commit, then tell
   the builder "step 0 done":
   - `AGENTS.md` §7 Freeze: replace "the last code deploy is Mon 11-02"
     with "the last code deploy is Wed 11-04 09:00 PST", and "From Tue
     11-03 00:00 PST" with "From Wed 11-04 09:00 PST"; §1 Scope: add "The
     operator ordered on 11-02: Stripe payments, live USPS and FedEx
     postage, mailing kits (MISSION WIN); this lifts specs decisions 10
     and 37 and the p2 door-kit non-goal"; §6: raise write-capable
     subagents to 7 and allow them in Rust crates for lanes SA-22..SA-25
     (or keep the cap and start the lanes as sessions, next line).
   - `.agents/goal.md`: Calendar (deploys to Wed 09:00, lane merges to
     Wed 07:30), Lanes = SA-22..SA-29 from `docs/lanes/sa-*.md`
     (branches `sa-<NN>/<name>`), priorities = the plan's protected core.
   - `.agents/roles/builder.md:71` ("No lane merges after Monday 15:00")
     → "Wed 07:30 PST"; `:76` freeze line → Wed 11-04 09:00 PST.
   - Promote the briefs `docs/lanes/sa-22.md` … `sa-29.md` to role files
     if you want them under `.agents/roles/`.
1. **Start the lane sessions from the desktop app.** Over Remote
   Control the app refuses session starts, so type "start the lanes"
   in the Claude desktop app. Until then: the web lanes SA-26/27/28 run
   as worktree subagents (allowed today, web only, ≤3); the builder does
   SA-22 M1 (telemetry) itself for T1; SA-23, SA-24, SA-25 and SA-29
   (judge kit) wait for sessions.
2. **Sandbox credentials on plant (0600, never pasted in chat).** Create
   FedEx developer-portal TEST project keys + test account, confirm the
   USPS TEM credentials, Stripe test-mode keys, then on plant:
   `install -m 600 /dev/null ~/.config/apostille/stripe-test.env` (lines
   `STRIPE_SECRET_KEY=` and `STRIPE_WEBHOOK_SECRET=`), the same for
   `fedex-sandbox.env` (`FEDEX_CLIENT_ID=`, `FEDEX_CLIENT_SECRET=`,
   `FEDEX_ACCOUNT=`) and `usps-tem.env` (`USPS_CLIENT_ID=`,
   `USPS_CLIENT_SECRET=`, `USPS_CRID=`, `USPS_MID=`, `USPS_EPA_ACCOUNT=`).
   Paste back only "env files installed". Without FedEx sandbox keys,
   FedEx stays on recorded synthetic fixtures until a capped live buy.
3. **Monday evening.** Approve usa-wa-sos's 17 proposals in production
   `/review` (≥8 accepted, keep 2 proposed for the video); add official
   Viet Nam seed URLs in admin Workers → fetch (production has 0 for
   vietnam); commit `web/src/lib/assets/brand/google.svg` and
   `apple.svg` (GP3); re-run `ops-a5-eval.sh` and `ops-a2-record.sh`
   (A5's 72 h window ends Tue 20:29 PST).
4. **Train T1 ~21:30 PST** (builder, no action from you): shell,
   meta-1, telemetry, runtime-only cleanup if green. Report lands here.

---

## Earlier today (Monday 2026-11-02, ~04:10 PST)

DEPLOYED ~10:35 PST on your "deploy now": production runs
`0.1.0+4dcd234` (previous `2b4bc8d` kept for rollback; flags
`--skip-accept --allow-today-overlap` announced per §7).
Prod-after **5/10**, zero regressions vs prod-before (P0, GP1,
GP4, A2, A5 pass; every red row identical). First red still
GP2-data (vietnam 0/8, usa-wa-sos 1/8 accepted — flips with your
approvals below, no code needed). This deploy ships wave-2
(SA-18 egypt-chain, SA-17 106 authorities, SA-21 walk-in
reports, SA-15 promises-readme) plus the release-train gap
fix. Train fixes en route: prod-before red-allowlist (P312)
and any-order deploy flags (staged server-side). If your
browser 403s, the page itself tells you which IP (or IPv6 /64)
to add — then restart is needed (the list reads once at
boot).
Wave-2 merged since: SA-18 egypt-chain, SA-17 north-america-106
(106/106 authorities), SA-21 walk-in reports + wiring (P305),
SA-15 promises-readme; P304 closed the release-train gap (no
day gate, /healthz settle poll). All stamped `ci_green`
(P304 17:31Z, P305 17:37Z, tip `4dcd234` 17:52Z). Staging-green
on `bde970e` (Run 19: 9/10, only S1 red, demo 3/3); the tip
adds README/docs only, so the binary + SPAs are identical —
deploy candidate is `4dcd234`. Say "deploy now" and the builder
ships it via `release-train.sh`. RO-2 `up --check` green on the
tip in a fresh clone (Node v26 here; the 24/22 matrix pending).
SA-14 parked: max 1 capture per source on prod and staging,
precondition needs ≥2.

Your top lines now (details in Still-open below):
1. Approvals in `/review` (≥8 per slug, keep 2 per slug for the
   video) — flips GP2-data, A1, GP2-page.
2. Viet Nam proposals + A5: `ops-gp2-proposals.sh`, then
   `ops-a5-eval.sh` Monday evening — the 72 h window expires
   **Tue 20:29 PST** — and again Wed morning.
3. S1: LICENSE stays proprietary (no change); repo public,
   demo video link (replace the README line yourself on
   2026-11-03; builder must never upload it).
4. Prod `META_API_KEY` is already working (A2 live, 38 ms).

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
   the quality.json lines. Flips A5; proposals feed item 2. A5 window:
   re-run Monday evening whether or not Monday's deploy happens
   (current 72 h window expires Tue 20:29 PST), and again Wed morning.
4. S1 (LICENSE stays proprietary — do not change it): make
   the repo public, link the demo video in README (video lands
   11-03). Video: record the production admin approve scene
   yourself (session login from your allowlisted IP, no cert;
   builder takes desktop/mobile), upload to YouTube, paste the
   URL back.
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
7. SA-6 weights (after the code ships): accept the HuggingFace
   licence for `facebook/sam3` on the account whose token is on
   plant, then paste back "HF ok" — unblocks `ops-a2-record.sh`
   with SAM 3 and the 3 synthetic recordings for production.
8. FedEx live (after the wave-2 deploy, in admin `/config` —
   no script, your keys): set `labels.fedex_client_id`,
   `labels.fedex_client_secret`, `labels.fedex_account`, the ZIP
   allowlist (`labels.ship_to_allow`), then `labels.live=true`;
   buy one real overnight label pair to your address (void if
   unused). Caps are already in code (2/day, $60). Paste back
   the label IDs or the void confirmation.

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
