OPERATOR ORDER 2026-11-02 — "push to submission". It overrides AGENTS.md §6's subagent limits until
Wed 11-04 12:00 PST. Everything else in AGENTS.md §7 (secrets, production, deploy rails) stands.

0. Before anything else, write this whole message verbatim to docs/ops/push-plan.md and commit it
   (P251). Re-read it at the start of every turn and after every compaction. It is the plan
   until the submission tag.

DECISIONS (operator)
- Deploy order: when I paste the unit-file + admin.allow_ips evidence, ship 3836bfb (stamp,
  Run 12 9/10, live demo 3/3) at once with --allow-today-overlap, prod-before/after, rollback
  rules unchanged. Do not hold it for new work.
- A second deploy on Monday is allowed only for a stamped, staging-green SHA ready by 22:00 PST.
  No deploy after Mon 23:59 PST.
- The meta lane is closed. Merge meta-1 only if it is green and you judge it useful; its paths
  return to you.
- Open-weights lane (review N6): <YES|NO>.

GOAL by Wed 11-04 10:00 PST:
(a) a judge who clones the public repo runs ONE command and gets the handbook with data and
    admin;
(b) every runtime model is a Meta model (keep model-gate green);
(c) production shows approved facts and the live demo take passes on production;
(d) the README tells the truth about what is live (production SHA) versus in the repo.

SUBAGENTS
- Up to 4 write-capable subagents at once. Each works in its own isolated worktree on branch
  sub/<name> and owns ONLY the paths listed for it. Read-only subagents (research, review, dry
  runs) are unlimited.
- Never give two subagents the same file.
- No subagent touches .agents/, .muse/, AGENTS.md, docs/ops/operator-today.md,
  docs/prompts/LOG.md, web/package*.json, app.css, any +layout.svelte, vite.config.ts, secrets
  or production. Synthetic or public data only.
- Every brief you write states:
  - the exact paths it owns;
  - the acceptance tests;
  - the gates it runs in its worktree (shared CARGO_TARGET_DIR; cargo test + clippy
    -D warnings + fmt for the crates it touched; the web gates of §4 for web/);
  - "report: branch, SHA, tests with counts, what you could not do".

YOU ALONE
- Merge one branch at a time (--no-ff).
- Before every push: a clean-worktree build of web + xtask, tests for every touched crate, and
  clippy on plant's toolchain. Plant is the lint source of truth.
- Edit the shared files: xtask/src/main.rs subcommand arms, crates/web/src/main.rs router and
  access-log lines, web/package*.json, LOG.md, operator-today.md.
- Submit the FULL SHA to plant after each merge batch. Run staging acceptance before any deploy.
  Deploy.
- A red subagent branch goes back to the subagent with the failing output. Never fix it on main.

START NOW (parallel)
- SA-1 "tokens". Owns crates/contracts/src/model.rs, crates/handbook-agent/src/{model,runner}.rs,
  crates/web/src/draft.rs, web/src/routes/(admin)/trace/+page.svelte and its tests.
  - Carry cached_tokens and reasoning_tokens from the Meta API usage block through
    contracts::Usage → RunReport → draft events → the /trace console, with per-call display.
  - Replay-fixture tests. No prompt change; the eval must be unchanged.
- SA-2 "unsubscribe" (review N1b). Owns crates/notify/**, crates/web/src/follow.rs,
  web/src/routes/(public)/notify/**.
  - List-Unsubscribe → /apostille/api/v1/notify/unsubscribe?token=… with an RFC 8058 form POST,
    idempotent 200.
  - One stable token per subscription: stop the per-digest rotate_unsub.
  - Unknown token → 410 with a clear page message. The page needs a button click (no auto-POST).
  - Confirm mail off the async thread, with an equal response time for new and existing
    addresses.
  - Reclaim stale leased rows; the daily cap counts emails.
  - Prove it with the file mailer.
  - Builder: the access-log path-only change (main.rs ~113).
- SA-3 "judge-run" (review N3; the top priority). Owns xtask/src/up.rs (new), rust-toolchain.toml,
  .nvmrc, README.md "Run it yourself" + "Models" sections, evals/captures/** (new), and in
  crates/config/src/lib.rs ONLY the release-mode dev-seam guard (~1815) and its tests.
  - `cargo xtask up [--live] [--fresh] [--check]`:
    - toolchain check;
    - npm ci (skipped when the lock is unchanged) + both web builds;
    - cargo build --release -p web -p xtask;
    - .local/ store + config-enroll file KEK (DEV CRYPTO banner);
    - config-set 127.0.0.1 binds, ports 18318/18319, auth.rp_id=localhost,
      auth.origin=http://localhost:18318, paths;
    - packs-load all;
    - captures built from the synthetic evidence in evals/authorities/*.json → analyze
      --scan-captures (stub; --live uses META_API_KEY and fetch-slug), so /review has
      proposals;
    - recorded A2 verdicts installed; eval → quality.json;
    - setup-key-mint;
    - start web in the foreground; print the URLs, the setup key and the try-this steps.
  - Allow the dev seams in release ONLY when every bind IP is loopback (test both ways).
  - Print a one-time, loopback-only staff login link, so judges without a passkey authenticator
    can open /review. It is refused unless every bind is loopback.
  - --check starts, probes /healthz, /api/v1/authorities and the admin setup status, then exits.
  - Builder: package.json "engines", and a package-lock.json that `npm ci` accepts on Node 22 LTS
    with npm 10 AND npm 11 (prove both); `up --check` in the plant gate on a clean checkout.
- RO-3 "public-readiness" (read-only). Writes a report you commit as
  docs/ops/public-readiness.md:
  - S1 items for me: LICENSE text (MIT: I apply it), the two prompt files needing IP redaction,
    docs/ops/security-review-2026-10-27.md to move or fix, brand SVGs, the video line;
  - gitleaks over the full history + secrets-scan;
  - model-gate;
  - every README claim checked against the production SHA (live) versus main (in the repo).

AFTER SA-3 MERGES
- SA-4 "any-domain" (review M14). Owns the hostname sites: crates/auth/src/{lib,csrf}.rs;
  crates/web/src/{host,passkey,admin,ws_bus}.rs (hostname code only); crates/acme/src/config.rs;
  the net.admin_host key in crates/config; xtask/src/release.rs smoke URL.
  - Hosts{public, admin} from net.public_host + net.admin_host;
  - ACME default domains derived from them; acme.directory=letsencrypt;
  - andrii.ai defaults byte-identical.
- RO-2 "judge dry run" (read-only). A fresh clone in a clean directory on plant AND on the Mac,
  following README literally (Node 22/npm 10, default rustup). Time it; report every stumble.
  Re-run after each fix.

AFTER SA-4 MERGES (if time; otherwise after judging)
- SA-5 "provision" (review M16, minus the client-CA step: P221 removed mTLS). Owns
  xtask/src/{provision,restore,package}.rs (new), xtask/ops/provision-remote.sh (new),
  docs/ops/provision.md (new).
  - Prompts for SSH target and domains (flags for each); --dry-run; --only <step>.
  - KEK escrow back to the operator; `restore --new-kek`.
  - Unit tests plus a --dry-run transcript. A live VM proof only if I provide a VM.

AFTER THE DEPLOY AND MY APPROVALS
- RO-1 "prod-proof" (read-only). The production scoreboard, then the DEMO_CAPTIONS=1 take on
  https://andrii.ai/apostille. Report the rows and the take paths for the video.

IF THE OPEN-WEIGHTS LANE = YES
- SA-6 "open-weights" (review N6/M18). Owns xtask/ops/a2-record.py and a new
  ops/open-weights/** (no Qwen; Meta open weights only; model-gate stays green).
  - SAM 3 open weights on plant's RTX 3060 replace the hosted sam-3.1 call in the A2 recorder,
    with the same box output.
  - Report the weights source and licence, latency, peak VRAM.
  - README Models row: "SAM 3 (open weights) — plant RTX 3060 — finds document marks in
    precheck photos".
  - Re-recording the production samples is my data step.

REPORT every 2 hours and at the end:
- a table: subagent | state | branch/SHA | tests;
- production version + scoreboard; staging SHA + score;
- what waits on me.

WED 11-04 10:00 PST
- Stop merging. RO-2 once more on main. Tag submission-2026-11-04 on the SHA that passes.
- Hand me the tag, the README video line to fill, and the final checklist.
