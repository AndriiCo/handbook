# TODO: promises vs code (audit 2026-11-04, base `d806520`)

What the README, `docs/judges.md` and `docs/ops/operator-today.md`
promise, checked against the code. Every item has file:line evidence,
a size (**S** < 1 h, **M** half a day, **L** > 1 day) and an owner hint
(*code* or *operator*). Items marked ✓ were re-checked by hand after the
audit; the rest come from reading the cited lines during the audit.

## Status (2026-11-04, after the P438-P441 merges)

Every code item below is done; the open ones are operator steps (video,
repo flip, brand logos, production mail, the P4-4 list). Workstreams:
A licence P430, W worker P433-P433c, H Egypt/gold set P436-P436b, the
README truth pass P437, then merges E P438, M P439, B P440, C P441.
Production still runs the pre-merge build until the next deploy
(migration 013 needs `ship-release --migration-ack`, then a pack
reload).

## How this is ranked

Startup Track criterion (event page): *"Judged on technical execution
and whether it holds up in production."* Submissions close
**Wed 2026-11-04 18:00 PT**; judging runs Nov 4–8.

Assumption: the repo is judged as it stands at the deadline, while the
live site keeps being looked at during Nov 4–8. So:

- **P0** Submission itself: missing pieces, and README claims a judge
  can disprove in a minute. Today.
- **P1** A judge's first 10 minutes: local run and staff sign-in.
- **P2** What judges see on production (andrii.ai/apostille).
- **P3** Correctness and security a judge reading the code would find.
- **P4** Product depth after the hackathon.

---

## P0: submission (before 18:00 PT today)

- [ ] **P0-1** (operator) Demo video URL on README line 6 (still "no video yet …
  on 2026-11-03"). Scoreboard S1 stays red without it. *operator, S*
- [x] **P0-2** (done: P430) ✓ Licence vs the S1 check. `LICENSE` is PolyForm
  Noncommercial 1.0.0. The scoreboard row says "OSI licence" and
  `s1()` requires MIT or Apache (`xtask/ops/scoreboard.mjs:53,256`), so
  S1 cannot pass as written. Decide the licence, then make the check
  and its row label match it. The event page has no licence rule;
  check the Official Rules. *operator, S*
- [ ] **P0-3** (full-history scan clean since P430; the flip is the operator's) Full-history `gitleaks`, then make the repo public
  (operator-today step 5). *operator, S*
- [ ] **P0-4** (operator) GP3 brand logos on the sign-in buttons (operator-today
  step 6). *operator, S*
- [x] **P0-5** (done: P432, P437) Truth pass on claims a judge can check. *code, S*
  - ✓ `README.md:108` lists "Run research and eval jobs on plant
    workers" under what shipped. No job has run, and no worker is
    connected (operator-today step 2).
  - `README.md:130` marks the notice inbox Delivered, and
    operator-today step 4 says a `service_notice` "goes out in the
    follower digest". Production `notify/status` answers
    `{"enabled":false}`, the instruct timer is not deployed, and no
    email has gone end to end in either direction (see
    [Email](#email-how-new-facts-reach-the-pages)).
  - ✓ `README.md:138` "reach the … MCP server: Delivered". The MCP
    server is a localhost-only stub (`crates/mcp/src/lib.rs:1-12`), not
    reachable from production. Say so.
  - ✓ `README.md:132` says es/fr translation is "merged but not wired".
    That understates it: accept queues translations
    (`crates/web/src/approval.rs:785,1035`), staff render them from
    /review → Translations, and the public dossier serves `?lang=es|fr`.
    Confirm production has rendered rows, then reword.
  - ✓ `README.md:262` and `docs/judges.md:27` describe a "loopback-only
    staff link". It is a plain `/review` URL with no token
    (`xtask/src/up.rs:1287`) and signs nobody in.
  - ✓ `docs/judges.md:32` and `xtask/src/judge.rs:11` say `--check` is
    "what CI runs". Nothing in `cargo xtask ci`
    (`xtask/src/main.rs:267-306`) or the plant gate runs it.
  - `README.md:267` says `setup-key-mint` "re-mints every run". It
    fails once a passkey exists (`xtask/src/up.rs:1230`).
  - `README.md:251` "one stub-fixture capture per authority" applies
    only in stub mode (`xtask/src/up.rs:193-196`).

---

## P1: a judge's first 10 minutes

### Staff console sign-in (admin login screen)

- [x] **P1-1** (done: P431) ✓ First-run dead end. Before anyone enrols, every console
  route bounces to `/staff-login`
  (`web/src/routes/(admin)/+layout.svelte:111-131`). That page asks for
  a "Staff user ID" and says "Enrolment is invite-only — ask your
  admin" (`staff-login/+page.svelte:123`). Nothing points to `/config`,
  where the setup key goes. Fix:
  - While `setup_required`, send every console route to `/config`.
  - Add "First-time setup →" on `/staff-login`.
  - Have the `up` banner say "open /config first".

  *code, S*
- [x] **P1-2** (done: P431) The setup key has to be pasted twice. `/config` keeps it
  in page state (`config/+page.svelte:435`), and its enrol link
  (`/enrol?setup=1`, `:889`) asks for it again (`enrol/+page.svelte:157-167`).
  The enrol page clears the key before the passkey prompt (`:77-78`),
  so a cancelled prompt means a third paste. Reloading after the query
  is stripped shows "Invalid invite link" (`:45`). Carry the key once
  and keep it until enrolment succeeds. *code, S*
- [x] **P1-3** (done: P431) `/staff-login` and `/enrol` render without the console
  layout (`+layout.svelte:79,155`), and their submit buttons have no
  border or background classes (`staff-login:114-120`, `enrol:186-192`).
  "Sign in with passkey" shows as plain bold text against the screen
  edge on mobile. *code, S*
- [x] **P1-4** (done: P431b) Usernameless passkey sign-in. Coming back requires a
  32-hex user ID that is shown once with no copy button
  (`enrol:132-135`, `crates/web/src/staff_passkey.rs:540`). ✓ The staff
  idle timeout is 30 min (`crates/auth/src/lib.rs:75`), so a coffee
  break or a second browser locks the judge out. The customer login
  already uses conditional mediation
  (`(customer)/login/+page.svelte:177-199`); reuse it. *code, M*
- [x] **P1-5** (done: P431) ✓ No sign-out. The admin router has no logout route
  (`crates/web/src/admin.rs:381-495`). The only one is public
  (`crates/web/src/main.rs:592`) and clears the customer cookie. Add a
  route and a button in the console shell. *code, S*
- [x] **P1-6** (done: P431) An expired session mid-use shows "Staff session missing
  or expired…" with no link (`web/src/lib/admin/http-errors.js:9-10`).
  The session is only checked on a full page load. After sign-in the
  page always goes to `/review` (`staff-login:64`) while its message
  says "Continuing to the dashboard…" (`:92`). Send any 401 to
  `/staff-login?next=…`. *code, S*
- [x] **P1-7** (done: P431) A failed passkey check (401) shows "Unknown account…
  check the user ID" (`web/src/lib/admin/staff-auth.js:231`). *code, S*
- [x] **P1-8** (done: P431) The admin host also serves the customer routes:
  `/login` there renders the customer page, which posts to `/api/v1`
  without the `/apostille` prefix and fails (`web/vite.config.ts:11,33`).
  Return 404 for them as `/mock` already does
  (`crates/web/src/admin.rs:385`). `docs/ops/staff-enrolment.md:45`
  names `/login`; the real route is `/staff-login`. *code, S*
- [x] **P1-9** (done: P431) On a keyless judge boot, the `/config` wizard lists
  production keys (TLS, ACME, Telnyx, mailer) as "Still missing"
  (`web/src/lib/admin/setup-guides.js:241-268`). Hide them in judge
  mode. *code, S*
- [x] **P1-10** (done: P431b) Docs still describe client certificates and
  `/admin/enrol` (`docs/specs/customer-auth.md` §5,
  `docs/auth-playbook.md:14-15`, `docs/specs/spa-separation.md:62-64`).
  The wizard still offers "Disable staff mTLS" (`setup-guides.js:92`).
  The code uses an IP allowlist plus passkeys. *code, S*

### From-scratch setup, templates, Docker local-only run

- [x] **P1-11** (done: P432) ✓ Run the judge path in CI: add `cargo xtask judge
  --check` to the plant gate, and later
  `docker compose run --rm judge --check`. Nothing tests it today.
  *code, S*
- [x] **P1-12** (done: P432) Add a check that runs before cargo. The doctor only
  runs after xtask compiles from cold (more than 2 min), so a missing
  C compiler, rustup or a distro rustc fails earlier with a compiler
  error. Add `ops/judge/preflight.sh` (cc, rustup, node, npm, ports,
  disk) and link it from `docs/judges.md`. Two related fixes:
  - The doctor rejects npm 10 (`xtask/src/judge.rs:94-98`), but
    `up.rs:466-482` handles it with `--legacy-peer-deps`. Make it a
    warning.
  - `--live` without `META_API_KEY` is only caught for `--record`
    (`up.rs:148-154`). Check it up front.

  *code, S*
- [x] **P1-13** (done: P432) **Docker judge path** (none exists). `ops/rehearsal/Dockerfile`
  is a privileged systemd and sshd box for production rehearsal and
  can't be reused. Two blockers:
  - The release binary refuses the dev key and `mtls_off` on any
    non-loopback bind (`crates/config/src/lib.rs:675,2365-2399`).
  - WebAuthn needs the origin to be exactly `http://localhost:<admin port>`.

  Plan:
  1. Add `xtask up --prebuilt`: skip the toolchain checks and builds,
     and find the binaries next to the running executable.
  2. `ops/judge/Dockerfile`, three stages:
     - web: `node:24-slim`, both web builds;
     - Rust: `rust:1.99-bookworm`, release build, `ARG BUILD_SHA`
       (`.dockerignore` drops `.git`);
     - runtime: `debian:bookworm-slim` plus `socat`.
  3. `ops/judge/entrypoint.sh`: socat `0.0.0.0:28318→127.0.0.1:18318`
     (same for admin), then `exec xtask up --prebuilt --fresh`. The
     server still sees loopback peers, so no gate is loosened.
  4. Root `compose.yaml`: ports published on `127.0.0.1` only
     (`127.0.0.1:18318:28318`, `127.0.0.1:18319:28319`), an optional
     `META_API_KEY`, and no volume, so every start is fresh.
  5. `docs/judges.md`: `docker compose up`, find the setup key in
     `docker compose logs`, and browse `localhost` (not 127.0.0.1 or a
     VM IP).

  *code, M–L*
- [x] **P1-14** (done: P432b) Setup wizard from scratch. `cargo xtask setup wizard`
  needs sops, age and `~/.config/sops/age/keys.txt` before it opens,
  because it saves the encrypted file first
  (`xtask/src/setup/mod.rs:294`, `wizard.rs:604`, `file.rs:557-582`).
  The only template is in code and hard-codes `andrii.ai`
  (`file.rs:225-228,376`). Fix:
  - Offer a local-only mode without sops, or create the age key under
    `private/`.
  - Ship a generic YAML template. `.env.example` is not an option:
    `xtask/src/secrets.rs:23` rejects `.env*` files.

  *code, M*
- [x] **P1-15** (done: P432) `setup --target local` ignores the target's ports and
  never passes `--fresh` (`xtask/src/setup/run.rs:351-357`), so a rerun
  after enrolment dies at setup-key minting. The wizard refuses the
  `local` target (`wizard.rs:447-451`). *code, S*

### Worker setup and coordination with the main host

- [x] **P1-16** (done: P432) Run a worker in judge mode. `up` sets 9 config keys,
  none of them `worker.token`, and starts no worker
  (`xtask/src/up.rs:610-643`). So the worker socket answers 503 and
  `/work` shows "No worker is connected". Fix:
  - Mint a token and write `.local/worker.env` (mode 0600).
  - Start `xtask worker --connect ws://127.0.0.1:<port>/…
    --workdir .local/work` alongside the server.
  - Have `--check` enqueue a `scrape.urls` job for a local page and
    wait for `done`.

  That is also the first test of the real worker against the real
  server; each side is only tested against a fake today. The `eval.gold`
  example payload is `live:true` (`crates/web/src/worker.rs:94`);
  default it to replay. *code, M*

---

## P2: visible on production

- [x] **P2-1** (done: P434) ✓ **Junk authorities.** About 100 of the 501 world rows
  (~20%, in 27 countries, by a conservative name heuristic) are
  fragments of the HCCH page. Each one is a public dossier with "Start
  an order". Examples:
  - Marshall Islands: "Limmatquai 50", "CH-8001 Zurich", "Fax: …".
  - Germany: "Land", "Oberlandesgericht", "Declaration of 2 August 2004:".
  - Austria (16 rows) and Poland (14 rows): document-scope lines. This
    is routing data that belongs in P4-1.

  Cause: world-index pairs names with URLs by list position, and its
  slugs are positional `{iso}-ca-{i}` (`xtask/src/world.rs:1200,1236`).
  `de-ca-2` "Oberlandesgericht" watches dpma.de, the patent office.
  Fix:
  - Curate the affected packs.
  - Add a name lint to `packs-validate`: lowercase start, trailing
    `:` or `,`, phone, fax, postcode or email, 6 characters or fewer.
  - Retire the bad rows (needs P2-2).

  *code + data, M*
- [x] **P2-2** (done: P434) A pack load never updates an existing authority:
  `load_pack` reuses the slug and changes nothing
  (`xtask/src/packs.rs:465`). The duplicate-slug check runs within one
  pack only (`:249`). A corrected pack therefore can't fix production.
  Add update-with-audit and a `retired` flag. P2-1 depends on this.
  *code, M*
- [x] **P2-3** (done: P434) Staged research packs can't be loaded. The worker stages
  them to `<store>/packs-inbox/<id>` and says Packs loads from there
  (`crates/web/src/worker.rs:648`). Packs only lists the release
  directories (`crates/web/src/admin_packs.rs:30-33`). This blocks
  operator-today step 2. *code, S*
- [x] **P2-4** (done: P434) ✓ Labels, flags and search:
  - `laneOf` maps any kind containing "state" to "US state"
    (`web/src/lib/public-ui/authority-meta.ts:104-124`), so 32 Mexican
    states are labelled and filtered as US states.
  - Flags exist only for nl, us and vn, and the US gets none (`:29`).
  - Search ignores `region` (`web/src/lib/public-ui/handbook.js:37-44`).
  - "Deutschland" and "Bayern" find nothing, because each authority
    has one name in one language.

  *code, S*
- [x] **P2-5** (done: P434) `/authority/usa` returns 404, and the "United States"
  breadcrumb is not a link (`crates/web/src/authority_path.rs:326`).
  P4-1 is the full fix. *code, S*
- [x] **P2-6** (done: P434) Every dossier downloads the whole index (~150 KiB) just
  to learn its own name (`[...trail]/+page.svelte:244`), and `resolve`
  rebuilds every path on each request. Add
  `GET /authorities/{slug}`. *code, M*

---

## P3: correctness and security in the code

### Email

- [x] **P3-1** (done: P435) ✓ The official sender can be spoofed. `resolve_sender`
  grants `trust=official` from the From domain alone; the DMARC result
  is parsed and stored but never checked
  (`crates/mail/src/poll.rs:380-403`). That contradicts
  `docs/specs/app-features.md:77-78` and `docs/gmail-shapes.md:61`.
  `sender_addr` also takes the first `<…>`
  (`crates/mail/src/mime.rs:84-90`), so
  `"x <ops@sos.wa.gov>" <a@evil.test>` reads as `ops@sos.wa.gov`. Fix:
  grant `official` only on a DMARC pass or aligned DKIM, parse the last
  angle address, and add tests. *code, S*
- [x] **P3-2** (done: P435) ✓ One failure stalls the instruct sweep. `sweep_all`
  returns on the first authority error (`?` at
  `crates/instruct/src/worker.rs:163`). Nothing is written and the
  cursor stays put, so every run pays for another Meta call and the
  authorities after it never run. Log the error and continue.
  *code, S*

### Worker

- [x] **P3-3** (done: P433) ✓ A finished job can be lost on reconnect. `poll()`
  takes the outcome off the channel
  (`xtask/src/worker/client.rs:144-150`). If `send` or `await_reply`
  then fails (`:229-236`), the outcome is gone, and the next session
  reports the fatal "job thread ended without a result". Keep the
  outcome until the server acknowledges it. This hits the first
  research upload. *code, S*
- [x] **P3-4** (done: P433) ✓ `precheck.glimmer` is never announced. `available()`
  reads the process environment
  (`xtask/src/worker/precheck_glimmer.rs:45`), but `GLIMMER_MODEL`
  lives in `worker.env`, which the worker parses itself
  (`xtask/src/worker/mod.rs:126`). The kind never appears in `hello`,
  so its jobs stay pending forever. This blocks operator-today step 3.
  *code, S*
- [x] **P3-5** (done: P433) The worker client accepts `ws://` to any host
  (`client.rs:377-380`) and sends the bearer token in clear text.
  Refuse `ws://` unless the host is loopback. *code, S*
- [x] **P3-6** (done: P433b) No cancel, no job time limit and no retry backoff:
  - There is no cancel route (`crates/web/src/worker.rs:975-992`).
  - Heartbeats keep a hung handler leased forever.
  - A lost lease only sets a flag (`client.rs:255-260`) while the job
    still holds the flock, so the worker's only slot stays blocked.
  - A retryable failure goes straight back to pending
    (`crates/handbook-store/src/work.rs:430-439`), so a rate limit
    uses up all 3 attempts in seconds.

  *code, M*
- [x] **P3-7** (done: P433b, P433c, P431b) Worker identity and visibility:
  - Identity: one token is shared by every worker, and the worker name
    is whatever the client claims (`crates/web/src/worker.rs:316-343,398`).
  - Rotation: rotating the token leaves open sockets connected.
  - Connection tracking: the list of connected workers is held in
    memory (`:218`), and nothing checks for a pong or an idle timeout
    (`:430-436`).
  - UI: two workers with the same name are a duplicate-key error in
    Svelte (`(admin)/work/+page.svelte:579`).
  - Version: the reported version is always `0.1.0` (`client.rs:195`).
    Send the git SHA and flag a mismatch with the host.
  - Restarts: `--restart` kills the worker mid-job
    (`ops/plant-pipeline/worker-install.sh:55`). Finish the current
    job on SIGTERM.
  - Install: `worker-install.sh:27` creates `/srv/scratch` without
    root, which contradicts operator-today's `sudo mkdir` step.
  - Ops scripts: `status.sh:9` watches the wrong process, and
    `worker.log` is never rotated.
  - Navigation: the console shows both "Workers" (URL ingest) and
    "Work queue" (plant), and `docs/specs/admin-workers.md` documents
    the wrong routes.

  *code, M*

### Authority onboarding

- [x] **P3-8** (done: P434) Observation task keys depend on position:
  `pack:{id}:obs:{index}` (`xtask/src/packs.rs:556`). Inserting an
  observation skips the new one and duplicates the shifted one. Key
  on a content hash instead. *code, S*
- [x] **P3-9** (done: P434) `world-index --live` overwrites the hand-built US, MX
  and CA packs as `kind=country` (`xtask/src/world.rs:239`). Only the US
  pack is protected by a test. *code, S*

### Other

- [x] **P3-10** (done: P431) The admin IP-refusal 403 page has no viewport meta and
  tells anyone who probes the config key name
  (`crates/web/src/admin_ip.rs:155-175`). *code, S*

---

## Email: how new facts reach the pages

Status on 2026-11-04: the code is mostly written and tested offline,
but **no email has gone end to end in production, in either
direction**.

| Step | Status | Evidence |
|---|---|---|
| Gmail poll (read-only, `historyId`, 7-day backfill) | Built; tested against a stub server | `crates/mail/src/poll.rs:62-107` |
| Gmail credentials on production | Probably missing (each run exits 2 without them) | `xtask/src/watch.rs:157-166` |
| `apostille-mail.timer` (15 min) | In production's timer list | `xtask/src/setup/file.rs:232` |
| Email stored as a capture (raw, text, auth results) | Built | `poll.rs:272-314` |
| Sender trust | **Broken**: DMARC never checked (P3-1) | `poll.rs:380-403` |
| Body and attachments | Partial: nested multipart skipped, no PDF text, no charset or encoded-subject decoding | `crates/mail/src/mime.rs:8,197` |
| Email → proposed facts (instruct worker) | Built; not proven against the live Meta API | `docs/meta-api.md:112` |
| `apostille-instruct.timer` | **Not deployed** (marked PROPOSED) | `docs/ops/apostille-instruct.service:1` |
| Review → publish | Built (ordinary `/review`) | `crates/instruct/src/propose.rs:265` |
| Follower digest: subscribe, double opt-in, one-click unsubscribe | Built and tested | `crates/web/src/follow.rs:66-150,236-300` |
| Digest outbox, retries, per-office and per-follower caps | Built | `crates/notify/src/lib.rs:340,420` |
| Digest sending (Gmail SMTP) | **Off in production; never sent** | live `notify/status` → `{"enabled":false}` |
| Bounces and complaints | Stub: `suppress()` exists, nothing calls it | `crates/notify/src/lib.rs:316` |
| Monitoring | None for mail; `/readyz` checks only the notify worker | `crates/web/src/readyz.rs:133` |

To get one notice through end to end (fix P3-1 and P3-2 first):

- [x] **E-1** (done: P435) Set Reply-To on outreach emails to the polled mailbox.
  Today outreach goes out from `mailer.from` with no Reply-To
  (`crates/web/src/first_source.rs:972`), so replies may never be read.
  *code, S*
- [x] **E-2** (done: P434) Hide the `mail-inbox` catch-all authority from the public
  index and from follows (`crates/web/src/approval.rs:1161`). *code, S*
- [ ] **E-3** (instruct timer provisioned in P432b; credentials, notify.enabled and the first notice are the operator's) Production setup:
  - Seal the Gmail credentials.
  - Enable `apostille-instruct.timer`.
  - Set the postal address and SMTP, and turn on `notify.enabled`.
  - Run one real notice through Gmail → capture → proposal → /review →
    dossier → digest, and record it.

  *operator, M*
- [x] **E-4** (done: P435b) Parser: walk nested multipart, decode charsets and
  encoded subjects, and extract text from PDF attachments. Today most
  real notices reach the model as headers and subject only. *code, M*
- [x] **E-5** (done: P435b) Add mail-poll freshness to `/readyz` and show the poll's
  last error on an admin page. A dead Gmail token only shows up in
  `mail_poll.last_error` today. *code, M*
- [x] **E-6** (done: P435b) PII:
  - Email text files carry the From and To addresses, so
    `xtask pii-sweep` (`xtask/src/pii_sweep.rs:63-80`) would fail.
  - Up to 400 characters of email text are mailed to followers as the
    quote.

  Redact before the model and before publishing. *code, L*
- [x] **E-7** (done: P435b; the sending domain's SPF/DKIM/DMARC is the operator's) Feed bounces and complaints into `suppress()`, add an
  overall send cap, and send from a dedicated domain with SPF, DKIM
  and DMARC. *code + operator, L*

---

## P4: product depth (after submission)

- [x] **P4-1** (done: P434) **Location page with disambiguation** ("which office
  handles my document?"). Today the `authority` table has only
  free-text kind, country and region
  (`crates/handbook-store/src/schema_baseline.sql:29`), and 400 of 501
  world rows have an empty region. The US group is a flat list of
  59 rows: the federal and court offices sit among the states, with no
  guidance on which to use. Sketch:
  - **Data (pack v2, additive):**
    - Per authority: `iso2`, `subdivision` (ISO 3166-2, e.g. `US-WA`,
      `DE-BY`), `level` (federal, state, regional, local, consular) and
      `paths[]`, which writes `authority_path`; nothing writes that
      table today.
    - `scope[]` entries of the form `{doc_class, issued_in, prereq,
      source_uri}`, using a closed `DocClass` list (vital record, state
      court, federal court, notarized private, company, state agency,
      federal agency, education, FBI check).
    - Scope entries load as review tasks. Once accepted, they become
      `jurisdiction:doc_rule` facts, so packs still never write facts.
  - **API:** `GET /api/v1/locations/{path}` returns the authorities
    grouped by level plus `routes[{doc_class, authority, prereq,
    fact_ids, verified}]`. A state inherits the federal routes.
  - **Route:** `/in/usa`, `/in/usa/washington`, `/in/de/by`. The
    breadcrumbs and the home index country rows link there.
  - **UI:**
    - A "What is your document?" chip picker, and an answer card that
      names the office and any prerequisite. Show an "unverified" badge
      until the rule is accepted.
    - "All offices here" grouped Federal / State / Local, plus "Not
      sure? We'll route it" leading to an order.

  An MVP for `/in/usa` and `/in/usa/<state>` from a small hand-written
  routing table is **M**; the general version is **L**. Then MX, CA
  and DE.
- [x] **P4-2** (done: P434b) Onboarding a new authority without a release:
  - Staff UI to create, edit and retire authorities (today the only
    admin route is a GET, `crates/web/src/first_source.rs:1247`).
  - An explicit `merges_into` field instead of load-order luck and the
    hard-coded `-6` in the test (`xtask/src/packs.rs:832`).
  - Validation across all pack directories.
  - English names and aliases for search.

  *code, L*
- [x] **P4-3** (done: P431b) Staff management UI: list, revoke, add a second passkey,
  invite. Today this is command-line only, and `admin-invite` defaults
  to the production origin and store (`xtask/src/admin_staff.rs:10,42`).
  *code, L*
- [ ] **P4-4** (operator; code support landed: Glimmer announce P433, staged packs load P434, V-19 P436, gold-export P436b) Operator-owned items already tracked in operator-today:
  - first live FedEx label;
  - `research.nonparty` run (after P3-3 and P2-3);
  - Glimmer weights (after P3-4);
  - envelope kits;
  - Egypt chain step validation V-1…V-19;
  - growing the gold set past 30 cases.

---

## Setup wizard (`cargo xtask setup wizard`): see and configure the whole system

Requested 2026-11-04. What the wizard did then (W-2 to W-7 below
changed the SSH, brand, sops and map points):

- It is a localhost page (`xtask/src/setup/wizard.rs`, single-file
  `wizard.html`, 308 lines).
- It edits targets of four kinds: local, docker, plant, server
  (`xtask/src/setup/file.rs:53-61`).
- It sets shared and per-target values, runs the key checks, and
  streams a target run live.
- SSH targets need an existing `~/.ssh/config` alias. The machine's
  architecture is only read at build time (`uname -m`,
  `xtask/src/setup/build.rs:66`).
- Brand assets need terms acceptance, then files dropped into
  `private/brand/` by hand (`xtask/src/setup/brand.rs:1-8`).
- It needs sops and age before it opens (P1-14).

Ranked by how much each helps a judge or the operator get a working
system:

- [x] **W-1** Staff sign-in by email instead of the 32-hex user ID
  (extends P1-4). **Done, commit 98afa44**:
  - Enrolment (setup key and invite) asks for an email and saves the
    passkey under it (its account name), so the browser lists and
    autofills it by email. Stored in `staff_email`; a passkey added from
    the Staff page reuses it; the Staff page shows it.
  - `/staff-login` has an Email field
    (`type="email" autocomplete="username webauthn"`) that offers the
    passkeys through conditional mediation, plus the button. The typed
    email is never sent, so it is no enumeration oracle.
  - Discoverable credentials came with P431b; the user-ID fallback stays
    for older keys. Lost-passkey recovery stays an invite.
  - Open: accounts enrolled before W-1 have no email and no screen to
    add one.
- [x] **W-2** **Done, commit 1bfcbd8**: each brand file has a Source
  URL (`brand.sources`, overridable; prefilled only where checked),
  the zip entry to take, and Fetch, disabled until acceptance is
  ticked. HTTPS only, 20 MB, 30 s; zips through `unzip`; the SVG
  sanitizer refuses `<script`, `on…=`, `<foreignObject` and outside
  links; the hash goes into the acceptance. Open: the only checked
  direct download is Google's bundle
  (`developers.google.com/static/identity/images/signin-assets.zip`),
  and all its current SVGs carry a `<foreignObject>` (a Figma gradient
  export), so Fetch refuses them; Apple's logo is a .dmg (the button
  API serves PNG only). Every asset is therefore still manual in
  practice, and uploads are not sanitized. Original item: One-click
  brand assets: prefilled source URLs, and
  **Fetch** downloads, unpacks and places each file. The tool still
  never accepts terms itself (`brand.rs:5-8`): Fetch is enabled only
  after the operator ticks acceptance. The wizard then:
  - downloads the file or zip, picks the expected file by name, and
    rejects SVGs that carry scripts or external references;
  - records the sha256 in the acceptance and writes the file to
    `private/brand/`.

  If the fetch fails, the step shows where to download by hand and an
  upload box. This also closes the red GP3 scoreboard row. Sources,
  checked 2026-11-04:
  - **Google:** the branding page offers pre-approved PNG and SVG
    buttons
    ([branding guidelines](https://developers.google.com/identity/branding-guidelines)).
    The bundle link changes between versions, so keep its URL in the
    setup file rather than in code.
  - **Apple:** PNG download or button API at
    [appleid.apple.com/signinwithapple/button](https://appleid.apple.com/signinwithapple/button);
    logo SVG and PDF under
    [Apple Design Resources](https://developer.apple.com/design/resources/).
  - **FIDO passkey icon:** behind a request form
    ([get the passkey icon](https://fidoalliance.org/get-the-passkey-icon/)),
    so it can't be fetched automatically. Manual instructions plus
    upload by default.
  - **Meta logo:** the guidelines must be accepted on
    [Meta brand resources](https://www.meta.com/brand/resources/)
    first. Likely manual.
  - **Vogun font:** a purchased licence with no public download. Keep
    manual upload or `$BRAND_FONT_DIR`. The Source Serif 4 fallback is
    OFL and already vendored (`web/static/fonts/fallback/`).

  *code, M*
- [ ] **W-3** A **Fake** mode for every outside service. Each
  integration gets one mode: `fake | sandbox | live`.
  - **Guard:** fake is refused unless every bind is loopback, or on a
    staging target, mirroring the dev-key gate
    (`crates/config/src/lib.rs:675`). Fake output is watermarked, and
    the console shows a FAKE ribbon.
  - **FedEx and USPS:** a demo label PDF stamped "DEMO — NOT POSTAGE",
    with a tracking number from a reserved fake range. Today
    `labels.live = false` means the carriers' sandbox
    (`lib.rs:1017`), and a fake judge mode is deferred
    (operator-today).
  - **Stripe:** a fake checkout and a receipt marked DEMO. Webhooks are
    signed with a local fake secret through the real webhook handler.
    Stripe test keys stay the "sandbox" mode.
  - **Outgoing email** (magic links, follower digest, outreach): a fake
    outbox in the store plus an admin **Outbox** view. Today dev mode
    writes to a folder (`crates/web/src/magic.rs:21`).
  - **Incoming email:** a fake inbox with a **Compose** screen to
    inject a reply or authority notice (From, subject, body,
    attachments, DMARC pass or fail). It goes through the same poller
    path (capture → instruct worker → `/review`), so the whole email
    loop can be shown with no Gmail account. This depends on P3-1:
    DMARC must decide trust before fakes can test it.
  - **Telnyx:** a fake call that plays a recorded WAV into the
    transcribe step.
  - **Meta Model API:** "Fake (recorded)" means the existing call-cache
    replay of the recorded corpus.
  - **Hugging Face:** fake skips the download, and Glimmer verdicts
    come from recorded files.

  *code, L in total; S–M per integration*
- [x] **W-4** **Done, commit 225bf6d**: the wizard opens on the map
  (`xtask/src/setup/map.rs`, drawn in `wizard.html`). Local is the
  `local` target (`runtime: native | docker`), Server the `server`
  target (front end) or `plant` target (workers, staging); the choice
  is `map.*` in the setup file, derived when unset. Server is disabled
  with the reason when the file has no such target (no target is
  created from the map). At 390 px the map box scrolls; dark mode
  follows the page. Original item: A system map as a UML deployment
  diagram, drawn as SVG from the wizard's state JSON, with no new
  libraries:
  - **Three nodes:** Front end (public + admin site), Workers and
    Staging. Each toggles **Local | Server**, and Local offers
    **Docker | Native**.
  - **Links between nodes:** solid lines labelled with the protocol:
    HTTPS, the WSS work queue, SSH deploy.
  - Each node shows its artifacts: the web binary, the worker and
    model weights.

  *code, M*
- [x] **W-5** **Done, commit 2a3308c**: `xtask/src/setup/probe.rs`,
  on plant and server tabs and on the map's server nodes, cached in
  `targets.<name>.probe`. `StrictHostKeyChecking=yes`: an unknown key
  fails with ssh's own words plus "run `ssh <host>` once in a terminal
  to accept its key"; the wizard shows no fingerprint and never writes
  `known_hosts`. Original item: SSH box probe: enter a hostname or
  alias, and the wizard connects with keys only (`BatchMode`;
  first-use host-key fingerprint shown for confirmation). It shows on
  the node:
  - host name and user;
  - OS name and version (`/etc/os-release` or `sw_vers`), kernel and
    architecture (`uname -srm`);
  - CPU count, RAM and free disk;
  - GPU (`nvidia-smi -L`) and Docker version.

  Results are cached in the setup file. *code, S–M*
- [x] **W-6** **Done, commit 225bf6d**: links, staging URL (through
  `ssh -L`), «use» Meta with model ids and a Test button
  (`POST /api/meta`; the key stays in the wizard), «deploy» weights
  from `map.weights_repo`, five service nodes with console links and a
  mode where the file sets one (Stripe key prefix, `labels.live`,
  `usps.live`; `map::mode` is where W-3's fake mode plugs in). Original
  item: Endpoints and services appear on the map as values are filled
  in:
  - **Front end:** the HTTPS server name, with a link to open the
    public site and the admin console.
  - **Workers:** a link to the admin **Work queue** tab.
  - **Staging:** its URL.
  - **Meta Model API:** a `«use»` dependency (dashed line, open arrow)
    from each consuming node, labelled with the model IDs it calls,
    plus a **Test** button (`GET /v1/models`).
  - **Hugging Face:** a `«deploy»` edge of the weights artifact onto
    the worker node, showing repo → `<worker>:/srv/scratch/apostille/models`.
  - **Gmail, Telnyx, FedEx, USPS, Stripe:** one node each, showing its
    mode (fake, sandbox or live) and linking to its console.

  *code, M*
- [x] **W-7** **Done**: the Docker judge path and the sops-less wizard
  landed in P432/P432b (merged in P441); commit 225bf6d makes the map's
  Local | Docker choice run them: a local target with `runtime: docker`
  plans `docker compose up --build --detach` and a healthz wait instead
  of `xtask up`. Original item: The Docker and native local paths, and
  the wizard without sops: see P1-13 and P1-14. W-4's Local | Docker
  choice runs those paths.

---

## From-scratch discovery

The first slice is on branch `claude/discovery-bootstrap`: the
`discovery.bootstrap` work-queue kind (`docs/specs/work-queue.md` §6).
It does three things:
- builds the country list from the live HCCH table, including the
  entry-into-force column;
- asks Muse for the party list from memory and reports where it
  disagrees with the table;
- asks Muse for official URLs in batches and fetch-checks every one.

It writes a report only. Next:

- [ ] **D-1** `research.nonparty` skips future parties. Thailand is on
  the status table, so it is subtracted, yet the convention only takes
  effect for it on 2027-02-28; until then it needs legalization. The
  world pack still says "Thailand is a listed Contracting Party"
  (`packs/world/TH.json`). Fix: treat future parties as non-parties
  until their entry-into-force date, and add an entry-into-force line
  to the world-pack observation. *code, S*
- [ ] **D-2** Read the objection markers in the status table (`A**`,
  e.g. Viet Nam and Kosovo). An objecting state does not accept
  apostilles from the acceding state, which matters for routing.
  *code, S*
- [ ] **D-3** Turn discovery output into pack updates (watch seeds,
  `unverified` domains). Needs P2-2 (a load that updates existing
  authorities) and P2-3 (staged packs can be loaded). *code, M*
- [ ] **D-4** Crawl the kept URLs with link keywords in local languages
  (`xtask/src/crawl.rs` matches only English words today), then sort
  the pages with a local Meta model on plant: relevant or not,
  language, authority, topics. Facts are still drafted by Spark with
  the quote check, then approved in `/review`. Before the local model
  drafts anything, run the 30-case gold eval against it. *code, L*
- [ ] **D-5** Have the local model split HCCH authority pages into
  authority, address, contact, document scope and note, each quoted
  from the page. This replaces positional parsing (the P2-1 junk rows),
  and scope lines become routing rules for P4-1. *code, M*

---

## Suggested order for today

1. P0-1 … P0-5: the submission and the README truth pass.
2. P1-1, P1-3, P1-5, P1-7, P1-11: small fixes so a judge's first two
   minutes in the staff console work.
3. P3-1, P3-2, P3-3, P3-4: small fixes on the email and worker paths.
4. P2-4, then the P2-1 lint and hiding the worst rows if time allows.
5. If P1-13 (Docker) can't land by the deadline, say so in
   `docs/judges.md`; don't imply it exists.
