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

ADDENDUM 2026-11-02 (operator order): world archive of apostille authorities as
WARC files + CSVs for judges (review §13). Repo-and-files work only: no production
store load, no deploy, no schema change. Model rule unchanged: only Meta models
(Muse Spark) anywhere in it; model-gate stays green. Every Meta call follows
`.agents/meta-api-findings.md` and the ai-slice skill (public HCCH text only;
quote-check every extracted name/URL against the cited page, reuse the
analyze.rs/runner.rs byte-exact pattern; recorded fixtures, replay tests).

- SA-7 "world-index" (write-capable, branch `sub/world-index`). Owns
  xtask/src/world.rs (new), packs/world/** (new), evals/world/** (new).
  `cargo xtask world-index`: fetch HCCH 1961 status table (cid=41) + every
  Contracting Party competent-authority page (EN); deterministic table parse;
  Muse Spark only for irregular pages with the facts quote check; write
  packs/world/<ISO2>.json (authorities[], watch[] all `needs_review`, each
  citing its HCCH URL) + archive/<run>/authorities.csv. Tests on recorded
  HCCH fixtures (public text). Report: parties found (expect 130),
  authorities, URLs, parties with zero URLs. Never touches xtask/src/main.rs,
  Cargo.toml/lock, LOG.md (builder wires CLI + commits those).
- SA-8 "crawler" (write-capable, branch `sub/crawler`; starts when SA-7 has a
  first packs/world). Owns crates/ingest/src/warc.rs (new), xtask/src/crawl.rs
  (new), xtask/src/archive.rs (new). `cargo xtask crawl --packs packs/world
  --out archive/<run>/`: WARC/1.1 .warc.gz, per-record gzip; warcinfo, request
  + response per redirect hop, metadata (slug, pack, robots verdict, TLS peer
  chain); sha1-base32 payload + block digests, Record-ID, Date, Target-URI,
  Concurrent-To; robots FAILS CLOSED (disallow logged in CSV); politeness
  1 req/2 s per host, 429/Retry-After honoured, 32 hosts parallel, 25 MiB PDF
  cap; UA "handbook-crawl/0.1 (+https://andrii.ai/apostille)"; depth 0 plus
  same-host apostille|legaliz|authentic|fee|form links (depth 1); files
  ≤ 1 GB at warc/<ISO2>/<slug>-NNNNN.warc.gz. `cargo xtask archive-package
  --run <run>`: manifest.csv, authorities.csv, coverage.csv (exact review §13
  columns), cdx/index.cdxj, WACZ 1.1 (world + per country), text/<sha256>.txt,
  SHA256SUMS, index.html. Validate WARCs with an independent reader + WACZ
  with `wacz validate` if available; report results. Same no-touch shared
  files as SA-7. Report: parties/authorities/URLs; ok/failed/disallowed/
  missing; bytes; crawl time; validation output.
- RO-4 "rights-and-access" (read-only, branch `sub/rights`). Commits only
  docs/ops/world-archive-rights.md: per-country governing terms/copyright
  note + robots status, HCCH terms, what publishes openly (CSVs, coverage,
  quotes) vs behind a judges-only token (warc/, wacz/).
- Builder after SA-7/SA-8 merge: wire `cargo xtask up --from-archive <url|dir>
  [--country ISO2]` into up.rs (sha256 verify, import response records as
  kind='warc' captures for analyze). Builder merges one branch at a time.

RUN (operator-run; prepared, never run against production):
1. Tonight: wget T0–T1 pass from the operator-chosen crawl origin (/24 or
   plant): `wget --input-file archive/<run>/t1-urls.txt --warc-file
   archive/<run>/warc/t1 --warc-cdx --wait=2 --random-wait -e robots=on
   --timeout=30 --tries=2 --user-agent "handbook-crawl/0.1
   (+https://andrii.ai/apostille)"`, then archive-package.
2. Mon/Tue: Rust crawler over T0–T2, then archive-package.
3. Hand the operator files/ + SHA256SUMS for the hermes site.

ADDENDUM 2 (operator order 2026-11-02, verbatim). The subagent rules of
push-plan.md still apply: disjoint paths, the builder alone merges, gates,
submits and deploys.

STOP-THE-LINE (builder, before any other merge)
B1 Deploy blocker: crates/notify/src/lib.rs:38-40 runs `ALTER TABLE notify_subscription ADD
   COLUMN unsub_blob`, and xtask/ops/ship-release.sh:74-76 aborts on any added ALTER against
   live 3836bfb.
   - Move the sealed token into `CREATE TABLE IF NOT EXISTS notify_unsub(sub_id INTEGER
     PRIMARY KEY, blob BLOB NOT NULL)` and delete the ALTER.
   - Test: a DB created by 3836bfb's notify schema opens, mints and stores tokens without
     ALTER.
   - Run ship-release.sh's schema grep locally against 3836bfb and show it passes.
B2 Public URLs: everything built from cfg.auth_origin (crates/web/src/main.rs:1018-1092) uses
   https://admin.andrii.ai in production, because the setup wizard says auth.origin = the admin
   origin (web/src/lib/admin/setup-guides.js:90-98). Live today: dossier canonical, og:url,
   JSON-LD url and every sitemap URL point at the IP-gated admin host.
   - Add one public_origin() = "https://{net.public_host}" and use it for: unfurl
     canonical/og/JSON-LD, sitemap, the Google/Apple/Meta OAuth redirect URIs (the callbacks
     are served on the public host), magic-link base, notify base, Meta delete-status base.
   - auth.origin stays the staff WebAuthn origin only. Fix the wizard copy.
   - Tests: auth.origin=https://admin.andrii.ai + net.public_host=andrii.ai → every public URL
     on andrii.ai.
   - Ask me, don't guess, whether production customer OAuth/passkey sign-in works today.
B3 .gitignore: /.local/ (it holds a raw KEK) and /archive/. Make crates/web/build.rs fall back
   to "unknown" when .git is absent.
B4 Tweak xtask/ops/a2-record.py (unless SA-6 open-weights owns it):
   - record per-model seconds and usage in each verdict JSON;
   - fail loudly when SAM returns 0 boxes on the clean sample.
   I re-run ops-a2-record.sh after the deploy: production replays carry boxes: [] today.
B5 Truth fixes:
   - README "approvals run against the Meta API" (they don't call a model);
   - operator-today.md header (still says 69ca7fa; production is 3836bfb) plus the missing
     prod-after record for that deploy.
DEPLOY ORDER (given now): ship the first stamped, staging-green SHA that contains B1-B5 plus
whatever below merged by Mon 20:00 PST, with --allow-today-overlap, by Mon 22:00 PST. No
deploy after 23:59.

SUBAGENTS, start now, disjoint paths
SA-9 "gate" (ideas 2, 12, 14, 1). Owns crates/handbook-agent/src/analyze.rs, the `value` schema
in crates/handbook-agent/src/model.rs (~330), xtask/src/eval.rs, the accept-with-edit check in
crates/web/src/approval.rs (~347-356), web/src/lib/public-ui/{guide.js,CitationMark.svelte},
the dossier Evidence popover in web/src/routes/(public)/authority/[slug]/+page.svelte, and their
tests.
- Reject empty, whitespace-only or <2-character values (validator + schema minLength).
- Accept-with-edit: when the edited statement is `field: value`, its value must be in the
  citation quote (same normalization); else 422 with the reason.
- markQuote highlights the exact value span (normalized match keeping digits and $/:/-); unit
  test over all 26 gold cases: 26/26 marked (today 14/26).
- "Source" links open the page at our quote with a text fragment #:~:text=<start>,<end>
  (percent-encode, escape - & ,); plain URL for .pdf.
- `cargo xtask eval --planted`: deterministic mutations of the gold set (value swap, span
  moved, digit change, empty value, truncated value, dropped "not"/"only", wrong topic) → a
  caught/missed table; after the fixes, report before → after.
- Add `planted` and `misses[]` to quality.json (new fields only; home reads quote_found_rate).
- README eval card: n=26 synthetic; planted N/N caught; the misses; why topic_accuracy
  understates (one gold topic per case, several drafts per case).
SA-10 "visible-checks" (ideas 23, 3, 22, 6). Owns crates/handbook-agent/src/runner.rs (attempt
recording), crates/web/src/draft.rs, web/src/routes/(admin)/trace/+page.svelte,
web/src/routes/(admin)/review/+page.svelte, the dossier ledger "approved Ns ago" in
web/src/lib/public-ui/handbook.js, and their tests.
- Record every attempt (n, refused/truncated/validation error kinds) in RunReport → run view
  → run_finished event. The trace shows e.g. "attempt 1: 2 rejected (value not in quote) →
  retry → attempt 2: 3 proposed", with cached and reasoning tokens.
- One `analyze.rejected` audit row per rejected record (kind, topic, field, value ≤200 chars,
  span) inside the existing transaction; staff GET /draft-runs/{id}/rejections.
- /review gains "Blocked by the verifier" with a counter.
- /review sorts riskiest first (changed value vs live sibling, conflict, missing citation,
  low-trust source) with a one-word badge saying why.
- The dossier ledger says "approved Ns ago".
- Keep the demo's exact strings ("Review queue", "Accept", "Fact #7 decided: accepted.",
  "Trace console").
SA-11 "up-fixes" (judges). Owns xtask/src/up.rs, rust-toolchain.toml, .nvmrc. The builder applies
web/package.json and its lock.
- Accept Node >=24.
- engines {"node": ">=24", "npm": ">=11"}; with npm 10, print the one-line fix or fall back to
  `npm ci --legacy-peer-deps` with a note.
- Stub mode must not publish a 0% quality chip: either install the recorded production
  quality.json, labelled "recorded production eval, 2026-11-01", or hide the chip when model
  is stub.
- The staff passkey works on the loopback admin port: allow http://localhost:18319 as a
  WebAuthn origin under the dev seam only; print localhost URLs, never 127.0.0.1.
- wait_healthy stops when web exits.
- rust-toolchain.toml targets += x86_64-unknown-linux-gnu.
- Then RO-2 (read-only judge dry run): a fresh clone on Node 24/npm 11 AND Node 22/npm 10,
  README followed literally; report every stumble.
SA-12 "crawler-fixes" (repo-only; by Wed 10:00). Owns crates/ingest/src/warc.rs,
xtask/src/crawl.rs, xtask/src/archive.rs.
- SSRF: on every hop resolve and reject loopback, RFC 1918, link-local, CGNAT, ULA, metadata;
  pin the resolved IP.
- robots.txt per RFC 9309 (group matching, * and $ wildcards), cached per host, Crawl-delay
  honoured; a redirect to a new host re-checks robots and politeness.
- Wire fidelity: no transparent gzip (or record the real request headers); Host with port;
  drop Transfer-Encoding for de-chunked bodies; UUID v4 bits.
- CSVs exactly per review §13: manifest per URL with warc_offset/warc_length; authorities per
  authority; coverage per party with hague_status and coverage_pct.
- Standard CDXJ (SURT key; url, offset, length, filename).
- WACZ 1.1: datapackage.json hash+bytes, pages.jsonl header, ZIP64, streamed, not in memory.
- Validate with `warcio check` and `wacz validate`; attach the output.
SA-13 "agent-cites-us" (idea 11; Meta tooling breadth; repo-only by Wed). Owns xtask/src/ask.rs
(new) and examples/agent-cites/**.
- `cargo xtask ask "<question>" --slug usa-wa-sos`: Muse Spark with function calling and ONE
  tool, lookup_authority(slug), backed by GET /api/v1/authorities/{slug}/facts on a base URL
  (production, staging or localhost).
- The answer must quote verbatim and cite source_url + captured_at; refuse when no fact covers
  the question.
- Record one live staging run as a fixture plus a 10-second demo beat. model-gate stays green.
OPTIONAL SA-14 "source-changed" (idea 25), only if staging or production has ≥2 captures of the
same source.
- Per fact: "re-confirmed on <date>" when the quote is in the newest capture, amber "source
  changed" when not.
- Additive public field; dossier badge. Check the data first and report the counts.

REPORT every 2 hours: subagent | state | branch/SHA | tests; staging SHA + score; production
version + scoreboard; what waits on me.
