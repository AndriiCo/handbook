# System Design: Evidence Store (Step 1 — Collection Infrastructure)

- Status: proposed
- Date: 2026-10-26
- Prompt: [docs/prompts/2026-10-26-003-system-design.md](prompts/2026-10-26-003-system-design.md)

## 0. Goal and non-goals

**Goal.** Build the infrastructure that collects the raw material used to
reason about apostille facts: web pages, files/scans, customer photos and
videos, emails, site comments, and phone-call transcripts — plus everything
needed to validate each item (authenticity evidence, provenance, trust tier).
This is step 1; handbook generation and the MCP server consume it later.

**Non-goals.** No multi-server scaling (one server), no public API, no
auto-publishing of community input, no ML training. Everything in Rust,
including maintenance tooling (`xtask`, no shell scripts).

```text
official sites ─┐
third-party sites│  robots.txt gate
files / scans ──┼─▶ ingest (xtask) ─▶ file store ─┐
photos / videos ┤   (warc, mbox,        (local disk) │
emails ─────────┤    blobs)                         ├─▶ SQLite metadata ─▶ tasks ─▶ calls ─▶ facts ─▶ handbook build
site comments ──┘                                   │      ▲                      ▲             ▲
support channels ───────────────────────────────────┘      │                      │             │
call transcripts ──────────────────────────────────────────┘                      │             │
                                                              audit log ◀────────┴─────────────┘
```

## 1. Metadata database (SQLite, one file)

One SQLite file (`store/handbook.sqlite`). Small, explicit schema — eleven
tables, no migrations framework until it hurts (schema version in
`pragma user_version`, migrations as numbered SQL files applied by `xtask`).

```sql
-- What every row is indexed by: the office / jurisdiction it applies to.
authority(id, kind,           -- country | state | province | office
          country, region, name, slug UNIQUE);

-- Tracked origins of information.
source(id, authority_id, kind,  -- website_official | website_thirdparty
                                -- | email_official | email_customer
                                -- | phone | visit | comment | document
       uri,                     -- URL, email address, phone number, ...
       domain_id, robots,       -- robots.txt verdict + checked_at (websites)
       notes);

-- Official vs fake domains. The anti-spoofing list.
domain(name PRIMARY KEY, authority_id, status,  -- official | known_thirdparty
                                                    -- | unverified | fake
       evidence_capture_id, notes);

-- Support channels, split by audience.
channel(id, authority_id, visibility,  -- public | private
        kind,                          -- phone | email | webform | chat | in_person
        value,                         -- number, address, URL, office address
        details,                       -- extensions, hours, team/volume notes
        purpose);                      -- customer_steering | our_validation

-- One collected artifact. Metadata here, bytes in the file store (§2).
capture(id, authority_id, source_id, captured_at, kind,
            -- warc | file | scan | photo | video | email | transcript | comment
        path,               -- location in the file store
        bytes_raw, bytes_stored, sha256,
        authenticity,       -- JSON: TLS or DMARC/DKIM evidence (§3)
        trust,              -- official | staff_verified | community | rejected
        notes);
CREATE INDEX idx_capture_authority ON capture(authority_id, captured_at);

-- Atomic, cited claims. The unit the handbook is built from.
fact(id, authority_id, capture_id,   -- provenance: every fact cites a capture
     topic,                          -- address | hours | fee | form | ...
     statement,                      -- one short claim
     status,                         -- proposed | accepted | superseded | rejected
     created_by, created_at);
CREATE INDEX idx_fact_lookup ON fact(authority_id, topic, status);

-- Full text over extracted text sidecars (§2), for conflict hunting.
CREATE VIRTUAL TABLE capture_fts USING fts5(capture_id UNINDEXED, body);

-- Correction / validation work items for handbook pages.
task(id, authority_id, kind,      -- validate_fact | correction | call
                                     -- | email_check | visit
     title, detail, status,        -- open | batched | in_progress
                                   -- | blocked | done | rejected
     priority, dedup_key,          -- §4
     batch_id, created_by, created_at, resolved_at, resolution);
CREATE UNIQUE INDEX idx_task_dedup ON task(dedup_key)
  WHERE status IN ('open','batched','in_progress');

-- One outbound contact holding many questions (§4).
batch(id, authority_id, channel_id, status,  -- draft | scheduled | sent | done
      scheduled_for, notes);

-- WoW feature: recorded, transcribed validation calls (§5).
call_session(id, batch_id, authority_id, channel_id,
             consent, consent_at,        -- unknown | granted | refused
             script,                     -- questions the app prepared
             transcript_capture_id,      -- capture(kind='transcript')
             outcome, conducted_by, conducted_at);

-- Handbook/algorithm build manifest (§6).
build(id, at, git_commit, build_no, facts_hash, notes);

-- Append-only audit of everything that changed content or code output (§6).
audit(id, at, actor, action, entity, entity_id,
      git_commit, build_id, before_ref, after_ref, reason);
```

## 2. File store (local disk)

`store/` holds bytes; SQLite holds pointers (`capture.path`) and hashes.
Content addressing (sha256, two-level prefix dirs) dedups identical files
for free. Writers stream; nothing loads whole archives into memory.

```text
store/
  handbook.sqlite            # §1
  warc/YYYY/MM/<sha>.warc.gz # one WARC per web capture (§3)
  mbox/<source_id>/YYYY-MM    # raw emails, appended; Brotli (.br) once sealed
  blob/xx/yy/<sha256>        # files, scans, photos, videos, transcripts
  text/<capture_id>.txt      # extracted text sidecar, indexed by capture_fts
```

- **Websites:** one `.warc.gz` per capture (gzip keeps standard WARC tooling
  working). Includes response records plus a `warcinfo` record with fetch
  time, server IP, TLS details, and fetcher build (§3).
- **Emails:** appended to a per-source, per-month mbox; sealed months (or
  files over 64 MB) get Brotli-compressed. Readers handle plain and `.br`
  transparently.
- **Media/docs/transcripts:** single blobs by sha256. Derivatives
  (thumbnails, face/plate-redacted copies) are generated on demand by
  `xtask` and cached, never hand-edited over originals. EXIF is preserved
  in originals for evidence, stripped in derivatives.
- **Text sidecars:** every capture with readable content gets extracted
  plain text (`text/<id>.txt`), which is what `capture_fts` indexes — so
  conflict hunting searches one uniform corpus, not ten formats.

## 3. Authenticity and trust (anti-bad-actor model)

- **Web authenticity:** each WARC `warcinfo` records TLS version, cipher,
  full certificate chain (PEM), validation result against a pinned root
  bundle (bundle version recorded), server IP, and fetch time. Re-checking
  authenticity later = re-validating the recorded chain, no re-fetch needed.
- **Email authenticity:** raw message preserved in mbox; `capture.authenticity`
  records SPF, DKIM (result **and selectors used**), and DMARC verdicts
  (covers Gmail/Outlook and any other signer uniformly). Verification runs
  at ingest via a Rust mail-auth library; failures keep the mail but cap
  its trust at `community`.
- **Official vs fake:** every website/email source resolves its domain
  through the `domain` table. `official` requires staff evidence (a capture
  proving it, e.g. a government directory listing); anything new defaults
  to `unverified`; confirmed spoofs are marked `fake` and their captures
  can never rise above `rejected`.
- **Trust tiers, promotion rules:** `official` (allowlisted domain +
  authenticity checks pass) → `staff_verified` (a human confirmed it) →
  `community` (customer photos, comments, customer email — quarantined by
  default) → `rejected`. Community input never auto-publishes: promotion
  to usable requires either corroboration by two independent captures or
  explicit staff approval, and the promotion itself is an `audit` row.
- **Third-party websites** are fetched only when their robots.txt permits
  it; the verdict and check time live on `source.robots` and are re-checked
  before each crawl.

## 4. Tasks, dedup, batching (don't overwhelm help desks)

- **Dedup:** `task.dedup_key = sha256(authority_id + normalized topic)`.
  The partial unique index (§1) makes "request the same info twice"
  impossible while the first request is still live; creating a duplicate
  returns the existing task instead.
- **Batching:** open tasks for one authority are grouped into a `batch`
  bound to one `channel`. The "What's your correct shipping address?" and
  "Are you closed early every 2nd Wednesday or was that a one-time poster?"
  tasks become one phone call (or one email), not two. A batch is scheduled
  against private validation channels; public channels are only ever used
  to steer customers, never for our bulk validation.
- **Tracker workflow:** `open → batched → in_progress → done/blocked`,
  with `resolution` citing the captures/facts that closed it. `blocked`
  (e.g. call consent refused) keeps the question visible instead of
  silently dropping it.

## 5. Call transcripts (WoW feature)

1. A `batch` of phone-kind tasks plus its private `channel` produces a
   `call_session`; the app prepares `script` (the batched questions plus
   context from conflicting facts, so the human knows what to nail down).
2. The human calls, reads the consent prompt first, and records the answer:
   `consent` stays `unknown` until then. If refused → session closed,
   tasks back to `open`, batch marked `blocked` with reason. No recording,
   no transcript, no exceptions.
3. If granted → record, transcribe with self-hosted `whisper-rs`
   (single server, no audio leaves the building), store audio + transcript
   as blobs and a `capture(kind='transcript')` linked from the session.
4. Transcript lines become `proposed` facts citing the capture; normal
   validation (§4) promotes them. The consent record (`consent_at` +
   transcript offset of the "yes") travels with the capture forever.

## 6. Audit history and builds

- Every content-affecting change (fact status change, trust promotion,
  handbook page edit) and every algorithm-affecting event (new handbook
  build, model/config change) appends an `audit` row: who, what, when,
  `git_commit` of the repo, `build_id`, before/after refs, and a reason.
  Audit rows are append-only (no update/delete path in code).
- Each handbook regeneration creates a `build` row: timestamp, repo git
  commit, monotonic `build_no`, and `facts_hash` (hash over the accepted
  fact IDs + versions that went in) — so any published page traces back to
  the exact inputs and code that produced it.

## 7. Conflicting facts, made easy

Conflicts are a query, not a feature to build: same authority + same topic,
two `accepted` facts with different statements:

```sql
SELECT a.authority_id, a.topic, a.statement, b.statement
FROM fact a JOIN fact b
  ON a.authority_id = b.authority_id AND a.topic = b.topic
 AND a.id < b.id
WHERE a.status = 'accepted' AND b.status = 'accepted'
  AND a.statement != b.statement;
```

Plus `capture_fts` keyword search across all extracted text, and an
`xtask review-conflicts` command that prints each conflict with its cited
captures. Finding contradictions (like Washington's two PO Boxes) is one
command.

## 8. Rust crates and xtask tools (phased, simplest first)

```text
crates/handbook/   domain types + SQLite access (rusqlite) + store paths
xtask/             fetch-url, import-mbox, import-file, review-conflicts,
                   make-batch, make-script, transcribe, build-handbook, ci
```

No new crates until one proves necessary; collection starts as `xtask`
commands over the `handbook` lib. Candidate libraries (unpinned):
`rusqlite` (+ bundled FTS5), `warc`, `mail-auth`, `whisper-rs`, `brotli`.
Telephony stays human-operated (a phone + recorder); no dialer to build.

## 9. Disk and memory efficiency

Content-addressed blobs (dedup), Brotli for sealed text archives, gzip WARC
(standard), streaming ingest (constant memory), one-file SQLite with FTS5
(no sidecar search engine), derivatives generated on demand and cached.
Nothing here needs more than one modest server.

## 10. Requirements traceability

| Requirement | Where |
|---|---|
| Track official + third-party sites (robots-aware) | `source`, `domain`, §2 WARC, §3 |
| PDFs, files, scans | `capture(kind=file/scan)`, blob store |
| Customer photos (locations, signs, parking) | `capture(kind=photo)`, trust `community` |
| Visit-instruction videos | `capture(kind=video)` + transcript sidecar |
| Official + customer emails | mbox store, `capture(kind=email)`, §3 DMARC/DKIM |
| Site comments/corrections | `capture(kind=comment)`, → `task(kind=correction)` |
| SQLite metadata, simple | §1, one file |
| Local files, mbox / Brotli batching | §2 |
| Easy conflict finding | §7 (conflict query + FTS + xtask) |
| Index by office/jurisdiction | `authority_id` on every table |
| Audit: git commits, build numbers, timestamps | §6 (`audit`, `build`) |
| Recorded/transcribed validation calls | §5 (`call_session`, scripts, consent) |
| Task tracker for corrections/validations | `task`, §4 |
| No duplicate requests; batched calls | `dedup_key`, `batch`, §4 |
| Public vs private support channels | `channel`, §4 |
| WARC + TLS evidence for authenticity | §2, §3 |
| DMARC indication + selectors for email | `capture.authenticity`, §3 |
| Official-vs-fake domain info | `domain`, §3 |
| Bad-actor resistance | trust tiers + promotion rules, §3 |
| Rust-only, one server, simple | §0, §8, §9 |

## 11. Assumptions and open questions

Assumptions: handbook content and code live in one git repo (so one commit
hash covers both); English/Spanish/French only for now; telephony is a
human with a phone, not an auto-dialer; whisper-scale ASR fits the server.

Open questions for the author:

1. Should the handbook pages themselves live in this repo (single commit
   hash for audit) or a separate content repo?
2. Self-hosted whisper vs a transcription API for call audio?
3. Any retention limits (call audio, customer photos) for privacy?
4. Which authority should be the pilot for the first batch → call →
   transcript → fact loop?
