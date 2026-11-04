# Analysis Agent Job Spec

- Status: built (direction approved 2026-10-28, D39, prompt 050):
  no-new-binary (`xtask analyze` + `handbook-agent` + timers), Meta chat
  API per `docs/meta-api-shapes.md`, queue-not-pause HITL, suppression
  A+B, Gmail historyId polling, vectors deferred unless discovery needs
  them. The analysis below is kept as the record.
- Date: 2026-10-26
- Source: parallel researcher draft (full text recovered from child
  session log).
- Role: this agent implements the F4/F5 drafting clause of
  [app-features.md](app-features.md). Topic-enum union rule (§6):
  freeze day 2.

Scope: in-project `ANALYSIS AGENT` batch job only. All bulk raw-data
analysis runs here, via Meta Models APIs — never in chat.

Source contract: [design.md](../design.md) §1 `fact`/`task`/`audit`,
§3 trust tiers, §4 tasks/batching, §5 calls, §7 conflicts;
[web-stack.md](../web-stack.md) §3; scaffolding prompt 001 § Prompt
(days 1–4: watch 500+ URLs, Spark turns only changes into cited
records the author approves, EN/ES/FR, flag contradictions like
Washington's two PO Boxes).

## 1. Inputs

1. `capture` rows (new since last watermark): `id, authority_id,
   source_id, captured_at, kind, path, sha256, authenticity, trust,
   notes`.
2. Text sidecars `text/<capture_id>.txt` — the only text the agent
   reads. No WARC/mbox/blob parsing inside the agent; extraction is an
   ingest precondition. Missing sidecar = skip + `task(kind=
   'correction')`, not an error retry.
3. `capture_fts` (`body` over sidecars): used for (a) retrieving
   same-authority context passages, (b) conflict-hunting recall per
   §7. Agent queries FTS; it never writes FTS directly.
4. Context rows per authority: existing `fact` (`topic, statement,
   status`), open `task` (`dedup_key`), `domain.status`,
   `source.kind`. These scope the diff, not the verdict.
5. Language tag per sidecar (`en`/`es`/`fr`, design §11). Unknown
   language = skip + task, never guess-translate into a fact.

Eligibility filter: `trust ∈ {official, staff_verified, community}`;
`rejected` and `fake`-domain captures are never analyzed. `community`
inputs can only yield `proposed` facts needing §3 promotion (two
independent captures or staff approval).

## 2. Outputs (never auto-accept)

1. `fact(status='proposed')` rows only. Agent has no write path to
   `accepted/superseded/rejected`. Each proposed fact carries:
   `authority_id, capture_id, topic, statement,
   created_by='analysis-agent/<model>/<config-hash>', created_at`,
   plus citation payload: exact source span(s) + sidecar byte offsets
   (see §5).
2. `task` rows with `dedup_key = sha256(authority_slug + normalized
   topic)` per §4 (slug, aligned prompt 011). Kinds: `validate_fact` (new/changed claim),
   `correction` (contradiction, §7-shaped), `email_check`/`visit`
   only when the evidence explicitly demands out-of-band checking.
   Duplicate creation returns the existing live task (partial unique
   index); agent treats that as success, not failure.
3. `audit` rows per §6 for every write (see §7).
4. No direct `batch`, `call_session`, or handbook writes.
   Batching/scripting stay human-driven per §4–§5.

Diff-to-record rule: emit a `proposed` fact only when the capture's
claim is new or textually differs from all live (`proposed`/
`accepted`) facts on `(authority_id, topic)`. Pure restatement =
no-op (advance watermark, no rows).

## 3. Rust runner shape

### Option A — new queue table + worker

`analysis_job(id, capture_id UNIQUE, status, attempts, next_run_at,
last_error)`. Ingest enqueues; worker claims rows.

### Option B — polling `xtask analyze` with watermark (RECOMMENDED)

No new table. `xtask analyze [--authority SLUG] [--limit N]
[--dry-run]` scans `capture` rows with `id > watermark` (watermark in
a one-row `kv`/`meta` table or a `store/.analyze-watermark` file —
prefer the `kv` row so it ships with the SQLite file), in
`(captured_at, id)` order, batching per authority.

Recommendation: **Option B**. Reasons: matches design §8 (collection
starts as `xtask` over the `handbook` lib, no new crates/tables until
one proves necessary); one server, one operator, no
concurrent-claim logic; watermark + idempotency keys give
exactly-once effect without a queue; queue-table claiming (`FOR
UPDATE SKIP LOCKED` equivalents, heartbeats, poison-pill handling) is
the complexity §0 rules out. Revisit Option A only if analysis ever
needs multi-worker parallelism or per-capture priority scheduling.

### Error / retry / idempotency policy

- Idempotency keys: fact insert keyed on `sha256(capture_id + topic +
  normalized statement)`; task insert relies on §4 `dedup_key` partial
  unique index. Re-running a capture returns existing rows and
  advances the watermark.
- Retryable (network/5xx/429/timeout from Model API): exponential
  backoff 3 attempts (e.g. 2s/8s/32s + jitter), then stop the run
  without advancing past the failed capture; next run resumes there.
  No partial-fact writes: one transaction per capture (facts + tasks +
  audit commit atomically).
- Non-retryable (4xx-auth/config, malformed sidecar, unknown
  language, rejected/fake trust): write a `task(kind='correction',
  status='open')` describing the blocker, write `audit`, advance the
  watermark. Auth failure aborts the whole run (config bug, not data
  bug).
- Determinism: low temperature (0–0.2), fixed prompt version recorded
  in `created_by`; same `(capture.sha256, prompt-version, model)`
  must yield byte-identical statements or the run is flagged
  non-reproducible in `audit.reason`.
- `CONSENT`/PII: transcript sidecars with
  `transcript_note(kind='ignore')` ranges have those ranges redacted
  before the prompt is built; ASR original stays immutable per §2.

## 4. Model-API client interface (trait-isolated)

Meta API spec decided (prompt 008 — see
[../meta-api.md](../meta-api.md)): REST + key auth (`META_API_KEY`
env, cf. web-stack §1), JSON bodies, streaming transcription via a
realtime WS endpoint. All access hides behind one trait in
`crates/handbook/` (e.g. `src/model.rs`); the agent and
transcription (§5/§8) share it.

```rust
pub trait ModelClient {
    // Draft structured records from evidence text. Streaming optional.
    fn draft_records(&self, req: &DraftRequest) -> Result<DraftResponse, ModelError>;
    // Health/auth probe for `xtask analyze --check`.
    fn check(&self) -> Result<ModelInfo, ModelError>;
}

pub struct DraftRequest {
    pub prompt_version: &'static str,
    pub authority_slug: String,
    pub lang: Lang,            // En | Es | Fr
    pub evidence: Vec<EvidenceSpan>, // sidecar id + byte range + text
    pub live_facts: Vec<LiveFact>,   // (topic, statement, status) for diffing
    pub topics: Vec<String>,         // frozen enum (§6)
    pub max_output_tokens: u32,
}

pub struct DraftResponse {
    pub model: String,
    pub records: Vec<ProposedRecord>, // topic + statement + citations
    pub usage: Usage,                  // prompt/completion tokens, latency_ms
}

pub struct ProposedRecord {
    pub topic: String,           // must be in frozen enum
    pub statement: String,       // one short claim, source-language + EN gloss
    pub citations: Vec<Citation>,// capture_id + start_byte + end_byte + quote
}

pub enum ModelError { Retryable(String), Fatal(String), BadOutput(String) }
```

Implementations: `MetaRestClient` (real; pure-Rust HTTP + `rustls`
per web-stack §1), `StubClient` (deterministic fixtures for `cargo
test` and `cargo xtask up`). Runner code depends only on `ModelClient`.
Bad-output (off-enum topic, missing/inexact citation) = `BadOutput`
→ one re-prompt with the validation error appended, then
non-retryable task.

## 5. Prompt sketch (EN/ES/FR diff-to-record, exact-span citations)

System (fixed, versioned, e.g. `aa-prompt-v1`):

> You are the Apostille Handbook analysis agent. Languages: English,
> Spanish, French. Input: evidence spans from ONE capture plus live
> facts for the same authority. Output: JSON array of records ONLY
> for claims that are new or textually different from every live fact
> on the same topic. Each record: `topic` (exactly one of the allowed
> list — never invent a topic), `statement` (one short claim in the
> source language; append ` // EN: <gloss>` when source is ES/FR),
> `citations` (each: capture id, byte offsets, verbatim quote). Quote
> spans EXACTLY — character-for-character, no paraphrase, no merging
> across sentences. If nothing new, return `[]`. Never accept,
> reject, or rank facts; drafting only.

User template per capture:

```text
AUTHORITY: {slug}   LANG: {en|es|fr}   TOPICS: [{frozen enum}]
LIVE FACTS (do not re-emit verbatim matches):
- [{topic}] {statement} ({status})

EVIDENCE (capture {id}, bytes {start}-{end}):
<<< {sidecar excerpt} >>>

Return JSON: [{"topic": "...", "statement": "...",
  "citations": [{"capture_id": N, "start_byte": A, "end_byte": B, "quote": "..."}]}]
```

Validator (code, not model): topic ∈ frozen enum; each quote occurs
byte-exactly in the named sidecar at the named offsets; statement
quotes or minimally compresses cited spans (compression must preserve
numbers/names verbatim). Violations → one repair re-prompt, else
`BadOutput` task.

## 6. Topic-enum union rule (freeze day 2)

Problem: free-text `topic` diverges (`po_box` vs `mailing_address` vs
`POBox`) and §7 conflict queries miss. Fix: freeze a closed topic
enum on day 2; the agent may only emit enum members.

- Freeze process: union existing `fact.topic` values + §1 seed topics
  (`address | hours | fee | form | …`) + any new need found during
  day-2 pilot (Washington state); normalize (`lowercase,
  _-separated`), publish as `TOPICS.md` + `pub enum Topic` in
  `crates/handbook`, prompt `TOPICS` list pinned to it.
- Post-freeze additions require a schema-adjacent change (enum
  variant + prompt version bump + audit note), never a model
  improvisation. Off-enum model output is a validation failure (§5),
  routed to a `correction` task proposing the new topic for human
  approval.
- `dedup_key` normalization (`§4`) lowercases through the same enum
  so `sha256(authority + topic)` stays stable across phrasings.

## 7. Cost / latency guardrails

- Budgets (config, overridable by flags): max sidecar excerpt per call
  (~6k chars, authority-live-facts truncated oldest-first); max 1
  call per capture + 1 repair retry; `--max-tokens-per-run` and
  `--max-captures` caps; run aborts with `audit` note when exceeded.
- Batching: group one authority's pending captures into as few calls
  as excerpt budgets allow (shared live-facts block amortized); never
  mix authorities in one call (prevents cross-authority leakage).
- Caching: skip re-analysis when `(capture.sha256, model,
  prompt_version)` matches a prior successful run recorded in
  `audit.after_ref`; content-addressed blobs (§2) make re-fetch free.
- Observability: every call logs `usage` (tokens, latency_ms, model)
  into its `audit.reason`/detail JSON; `xtask analyze --dry-run`
  prints projected call/token counts without calling the API.

## 8. Audit linkage

Every analyzer write is a §6 `audit` row:
`actor='analysis-agent/<model>/<prompt-version>'`, `action ∈
{fact.proposed, task.opened, task.duplicate_hit, analyze.skipped,
analyze.failed}`, `entity/entity_id` pointing at the created row (or
capture), both git commits, `before_ref` (live-facts snapshot hash) →
`after_ref` (new row ids + input `capture.sha256`), `reason` carrying
usage + validator outcome. Audit is append-only; the agent holds no
update/delete path. Promotion of `proposed` → `accepted` is always a
later human/staff action with its own audit row (never the agent's).

## 9. Worked examples (canonical fixtures only)

Setup: `authority = wa-sos`. Live state: one `accepted` fact
`(topic=po_box, statement="PO Box 40220")` from capture 11.

Example 1 — contradiction → propose + correction task. New capture 12
(official site recrawl, `trust=official`), sidecar bytes 200–214
contain exactly `PO Box 40234`. Agent diffs against live `PO Box
40234 ≠ PO Box 40220`, emits:

```json
[{"topic": "po_box", "statement": "PO Box 40234",
   "citations": [{"capture_id": 12, "start_byte": 200, "end_byte": 214,
                  "quote": "PO Box 40234"}]}]
```

Runner inserts `fact(status='proposed', authority_id=wa-sos,
capture_id=12, topic='po_box', statement='PO Box 40234')`, opens
`task(kind='correction', dedup_key=sha256("wa-sos|po_box"),
detail='accepted "PO Box 40220" (capture 11) vs proposed "PO Box
40234" (capture 12)')`, and writes two `audit` rows. The §7 query will
surface the pair once a human accepts the new fact; nothing
auto-accepts.

Example 2 — restatement → no-op. New capture 13
(`trust=staff_verified`), sidecar contains exactly `PO Box 40220`.
Statement matches the live accepted fact verbatim, so the agent
returns `[]`. Runner writes one `audit(action='analyze.skipped',
reason='restatement of live fact')`, advances the watermark, creates
zero fact/task rows.

## 10. Open questions

1. ~~Meta Models API spec~~ — decided prompt 008 (see
   [../meta-api.md](../meta-api.md)). Remaining: pin exact JSON
   shapes at implementation; SeamlessStreaming absent from the
   API → P3 stays conditional.
2. Content-repo form-template home: where do blank/authority form
   templates live (content repo path? LFS?) and how does the agent
   cite a form field vs a page span?
3. Frozen topic enum seed list: is `po_box` a topic or a value under
   `address`? (Examples assume `po_box`; day-2 freeze decides.)
4. Citation granularity for scans/photos: sidecar OCR has no stable
   byte offsets — accept approximate (page/region) citations or
   require re-OCRed stable sidecars?
5. `community` corroboration counting: do two agent-proposed facts
   from two captures count as the §3 "two independent captures," or
   must corroboration be human-confirmed?
6. Retention for non-voice captures: photos/videos supporting
   proposed facts — keep forever like transcripts or expire like
   audio?
7. `xtask analyze` scheduling: cron/systemd timer on the Ubuntu box,
   or on-demand after each ingest run?
8. Budget numbers: per-run token/call caps and per-authority batch
   sizes for the 500+ URL watch — what are acceptable day-1 values?

## 11. Review queue + rejection-feedback suppression

> Status: APPROVED as-is (prompt 054, D40) — implementation
> underway. Drafted from researcher R4's text, corrected by the
> parent against current code (schema v10, `approval.rs`,
> `insert_proposed_fact`).

### 11.0 Problem

B8: `task` rows exist but staff have no list/work API or UI;
`review-conflicts` only prints. Separately, D39 approved
"suppression A+B" but no section defines it: today a staff
rejection does not stop the analyzer from re-proposing the
identical claim on the next recrawl, so reviewers re-triage the
same junk every sweep. This section closes both gaps with one
review surface backed by the existing `decide()` path.

Definitions:

- **Path A (rejected-memory):** an analyzer-proposed statement
  byte-identical (after §11.3 normalization) to a `rejected`
  fact on the same `(authority_id, topic)` is never re-proposed.
  No staff action needed; the rejection itself is the rule.
- **Path B (explicit rules):** staff-authored `suppress_rule`
  rows that block future proposals matching a normalized
  statement, scoped globally or to one authority, optionally
  locked to a topic. Created from the reject control or
  standalone.

Invariants (non-negotiable):

1. The analyzer writes `proposed` facts only (§2). Suppression
   only *skips* inserts; it never deletes, never edits, never
   writes `accepted`.
2. The only write path to `accepted` is the existing `decide()`
   in `crates/web/src/approval.rs`, behind `AdminAuth`. The
   queue UI and accept-with-edit reuse it; no parallel path.
3. Every suppression skip is logged (row + audit). Suppression
   is observable, never silent.
4. No new binary, no new crates. Work lands in `crates/web`
   (routes), `handbook-store` (migration 011), `xtask analyze`
   + `handbook-agent` (consultation + citation/reasoning store),
   and the existing admin Svelte app.

### 11.1 SQL deltas (migration `011_review_queue.sql`)

```sql
-- Migration 011: review queue + rejection-feedback suppression.
-- Additive only. Timestamps INTEGER unix seconds. No FK enforcement
-- (001 convention). Rollback-safe: new tables ignore cleanly; new
-- fact columns are read-optional (see below).

-- Explicit staff-authored suppression rules (path B).
CREATE TABLE suppress_rule(
  id INTEGER PRIMARY KEY,
  authority_id INTEGER,           -- authority.id, NULL = global scope
  topic TEXT,                     -- NULL = any topic; else exact enum member
  statement_norm TEXT NOT NULL,   -- normalized per §11.3 (match key)
  statement TEXT NOT NULL,        -- original display text (first-seen)
  reason TEXT NOT NULL DEFAULT '',
  created_by TEXT NOT NULL DEFAULT '',
  created_at INTEGER NOT NULL,
  revoked_at INTEGER              -- NULL = live; non-NULL = revoked (kept)
);
CREATE INDEX idx_rule_match ON suppress_rule(authority_id, topic, statement_norm)
  WHERE revoked_at IS NULL;

-- Observable log of every analyzer skip (paths A+B). Not a queue:
-- staff never decide these; GET .../suppressions exposes them.
CREATE TABLE suppression(
  id INTEGER PRIMARY KEY,
  capture_id INTEGER NOT NULL,    -- capture.id being analyzed
  authority_id INTEGER NOT NULL,  -- authority.id
  topic TEXT NOT NULL,
  statement TEXT NOT NULL,        -- skipped statement (one short claim)
  statement_norm TEXT NOT NULL,
  rule_id INTEGER,                -- suppress_rule.id, NULL = path A
  path TEXT NOT NULL,             -- 'A' | 'B' (denormalized for listing)
  created_at INTEGER NOT NULL
);
CREATE INDEX idx_suppression_capture ON suppression(capture_id);
CREATE INDEX idx_suppression_at ON suppression(created_at);

-- Citation home (OQ-5, answered): the analyzer's byte offsets were
-- validated but never stored (`insert_proposed_fact` doc comment).
-- -1/-1 = uncited (pre-011 rows, or unbound single-shot runs whose
-- citation has no capture home). The doc is always the fact's own
-- capture sidecar (`text/{capture_id}.txt`), so no doc column.
ALTER TABLE fact ADD COLUMN citation_start INTEGER NOT NULL DEFAULT -1;
ALTER TABLE fact ADD COLUMN citation_end INTEGER NOT NULL DEFAULT -1;
-- Model's own per-response summary (quarantined in the UI, §11.4.4).
-- '' = predates capture. Populated from the new `summary` response
-- field (client truncates to 500 chars).
ALTER TABLE fact ADD COLUMN reasoning TEXT NOT NULL DEFAULT '';
```

Deliberately NOT included: Gmail capture columns and a `kv`
watermark table (an earlier draft's proposal — superseded by
Phase B: gmail ids live in `capture.authenticity` JSON, the
mailbox cursor in `mail_poll`).

Rollback: `DROP TABLE suppress_rule/suppression`; the `fact`
columns stay unread-optional (readers treat -1/'' as absent).
No data migration at upgrade: path A reads live `rejected`
rows; path B starts empty.

### 11.2 API deltas (all under `/apostille/api/v1/`)

Guards (unchanged from existing admin routes): `AdminAuth`
extractor; mutations additionally pass the Origin + double-submit
CSRF layers. Error baseline: `401 {"error":"unauthorized"}`
(session), `403` (role/cert), `503 {"error":"store
unavailable"}` (DB). Bodies use
`#[serde(deny_unknown_fields)]` like `DecisionBody`.

#### 11.2.1 `GET .../queue` — enriched review items

Staff list of proposed facts with everything needed to decide
inline. The existing `GET .../facts?status=proposed` stays
untouched (compat); the queue is a superset view, not a
replacement.

Query params: `authority` (slug, optional; unknown →
`404 {"error":"unknown authority"}`), `topic` (enum member,
optional; off-enum → `422 {"error":"unknown topic"}`),
`limit` (default 50, clamp 1–200 like `list_facts`),
`after_id` (cursor: `id > after_id`).

Response `200`:

```json
{
  "items": [
    {
      "id": 42,
      "authority_slug": "usa-wa-sos",
      "topic": "address",
      "statement": "po_box: PO Box 40234",
      "status": "proposed",
      "created_by": "analysis-agent/aa-prompt-v1",
      "created_at": 1761600000,
      "diff": {"base_statement": "po_box: PO Box 40220",
               "base_fact_id": 17, "base_status": "accepted"},
      "citations": [
        {"capture_id": 12, "start_byte": 200, "end_byte": 214,
         "quote": "PO Box 40234"}
      ],
      "reasoning_untrusted": "Sidecar bytes 200-214 differ from live fact 17.",
      "blast_radius": {
        "conflict": true,
        "conflicts": [{"id": 17, "statement": "po_box: PO Box 40220",
                       "status": "accepted"}],
        "open_tasks": 1
      },
      "provenance": {"capture_id": 12, "capture_kind": "warc",
                     "trust": "official",
                     "authenticity": "{\"tls\": true}",
                     "sidecar": "text/12.txt"}
    }
  ],
  "count": 1
}
```

Field rules: `diff` is null when no live sibling; else the
newest live sibling, accepted preferred (derived from the same
sibling query as the banner). `citations` holds zero or one
entry — the stored offsets plus the server-sliced sidecar quote
(`""` when the sidecar is missing/unreadable; offsets still
served). `reasoning_untrusted` is null for pre-011 rows, else
the stored model summary verbatim — always under that key name
so the UI quarantines it; never merged into a trusted field.
`blast_radius.conflicts` reuses `sibling_conflicts` verbatim
(same authority + topic, live statuses, cap 10);
`open_tasks` counts live (`open`/`batched`/`in_progress`) tasks
for the authority. (No `in_last_build`: the `build` table keeps
a hash, not a fact set — unimplementable without a set; cut.)

#### 11.2.2 `GET .../queue/{id}` — single enriched item

Same item shape for deep-linking. `404 {"error":"fact not
found"}` when missing; returns any status (inspect decided rows
too) with its current `status`.

#### 11.2.3 `POST .../facts/{id}/accept` — extended with edit

Same route, extended body (existing clients keep working):

```json
{"reason": "matches Sept recrawl", "statement": "po_box: PO Box 40234, Olympia WA"}
```

- `reason` optional, ≤1024 chars (existing
  `reason too long (max 1024 chars)` rule).
- `statement` optional: non-empty after trim, ≤2000 chars, else
  `422`. A changed statement must keep its text inside the cited
  quote (analysis-validator normalization): the value of a
  `field: value` statement, the whole text of an unkeyed one, else
  `422 {"error":"edited value not in citation quote"}`.
- `topic` optional (OQ-4, resolved by plan §5.5: re-topic at
  accept): one of the frozen topics (`Topic::parse`, else
  `422 {"error":"unknown handbook topic"}`); never into or out of
  the staff-only `phone_script` (`422`). `service_notice` moves
  like any topic (the validator drafts it like any topic). The
  response banner is computed for the new topic.
- Semantics: through the existing `decide()` transaction —
  `BEGIN IMMEDIATE`, re-check `status = 'proposed'`, run the
  value-in-quote and `phone_script` gates on the row it holds (a
  store error answers `503`, never an unchecked accept),
  `UPDATE fact SET status='accepted'` (plus `statement = ?` /
  `topic = ?` when changed), one `audit` row (§11.5), `COMMIT`,
  then the unchanged `fact_accepted` fan-out.
- Codes: `200` (`FactView`), `404` (unknown id),
  `409 {"error":"fact is not proposed"}` (already decided — the
  UI treats this as "someone else decided; refresh"), `422`,
  `401/403/503`.

#### 11.2.4 `POST .../facts/{id}/reject` — with optional rule

```json
{"reason": "PO box belongs to DOL, not SOS",
 "suppress": {"scope": "authority", "lock_topic": true}}
```

- `reason` required 1–1024 (existing
  `reason required (1-1024 chars)` rule, unchanged).
- `suppress` optional. When present, the same transaction that
  flips the fact to `rejected` also inserts a `suppress_rule`
  row: `statement_norm` = normalized rejected statement,
  `statement` = rejected statement, `authority_id` = fact's
  authority when `scope = "authority"`, NULL when `"global"`;
  `topic` = fact's topic when `lock_topic = true`, else NULL;
  `reason` = reject reason; `created_by` = staff user id.
  `scope` must be `authority|global`, else `422`.
- Codes: `200` (`FactView`), `404`, `409` (not proposed),
  `422`, `401/403/503`. If an identical live rule exists, the
  reject still succeeds (`200`) with
  `"rule": {"id": N, "deduped": true}` — the decision is the
  primary effect; rule creation is idempotent.

#### 11.2.5 `GET/POST .../suppress-rules` + revoke

Standalone rule management (rules not tied to one rejection):

- `GET .../suppress-rules?authority=SLUG&include_revoked=false`
  → `200 {"rules": [...], "count": n}`. Rule shape: `{id,
  authority_id, authority_slug|null, topic|null, statement,
  reason, created_by, created_at, revoked_at|null}`.
- `POST .../suppress-rules` body: `{"statement": "...",
  "authority": "slug"|null, "topic": "member"|null, "reason":
  "..."}`. `statement` required non-empty ≤2000; unknown slug
  → `404`; off-enum topic → `422`. Identical live rule →
  **`409 {"error":"duplicate rule","rule_id":N}`** (here the
  rule *is* the effect, so dup is a conflict). Success → `200`
  with the rule.
- `POST .../suppress-rules/{id}/revoke` body `{"reason": "..."}`
  (required 1–1024). Sets `revoked_at`; already-revoked →
  `409 {"error":"rule already revoked"}`; unknown → `404`.
  Append-only history (row kept); creation and revocation each
  write one `audit` row (`suppress_rule.create` /
  `suppress_rule.revoke`, entity `suppress_rule`).

#### 11.2.6 `GET .../suppressions` — skip log

`200 {"suppressions": [{"id","capture_id","authority_slug",
"topic","statement","path","rule_id","created_at"}], "count":
n}`. Params: `authority`, `limit` (default 50, max 200),
`after_id`. Append-only (retention: OQ-3).

#### 11.2.7 409 semantics summary

| Case | Code | Body |
|---|---|---|
| decide on non-`proposed` fact | 409 | `{"error":"fact is not proposed"}` (existing, unchanged) |
| standalone rule duplicating a live rule | 409 | `{"error":"duplicate rule","rule_id":N}` |
| revoke an already-revoked rule | 409 | `{"error":"rule already revoked"}` |
| reject-with-`suppress` when rule exists | 200, not 409 | decision wins; `{"rule":{"id":N,"deduped":true}}` |
| retire a non-`accepted` fact | 409 | `{"error":"fact is not accepted"}` |

#### 11.2.8 `POST .../admin/facts/{id}/retire` — undo an accept

`{"reason": "..."}` (required, 8-500 chars after trim, else `422`).
Staff session + CSRF, like accept/reject. One `BEGIN IMMEDIATE`:
`accepted` → `superseded`, one `audit` row (`fact_retire`,
`before_ref='accepted'`, `after_ref='superseded'`, the reason),
`COMMIT`, then `fact_retired {fact_id}` on `authority:{slug}`.
Every public read serves `accepted` rows only, so the fact leaves
`/facts`, the index count and `verified_at`, translations, the
crawler head, order destinations and the MCP lookup together;
`/plan` drops it with every block, path and accepted office that
quotes it (an office is served only while all its `fact_ids` and
piece `fact_id`s are accepted). Follower emails already queued for
it drop it too: `notify::load_fact` loads only accepted, non-
`phone_script` facts. Not yet covered: no client acts on
`fact_retired`, so an open dossier shows the fact until it reloads.
Codes: `200
{id, authority_slug, status}`, `404`, `409`, `422`, `401/403/503`.

### 11.3 Analyzer consultation rule (suppression check)

Runs inside `xtask analyze` per candidate record, after citation
validation (§5) and before the per-capture transaction inserts
anything. Pure reads + skip; no new deps (std-only).

Normalization (one shared helper, used identically by the
analyzer, reject-with-suppress, and standalone rule create):

```rust
pub fn normalize_statement(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}
```

Trim + collapse whitespace + lowercase. No NFKC (no dep), no
punctuation stripping (keeps `PO Box 40220.` distinct — safer to
over-propose than over-suppress).

Check order per `(authority_id, topic, statement)` candidate:

1. Compute `statement_norm`.
2. **Path A:** load `rejected` statements on
   `(authority_id, topic)` (covered by `idx_fact_lookup`),
   compare normalized in code (rejected sets are small). Hit →
   skip.
3. **Path B:** `SELECT id FROM suppress_rule WHERE revoked_at
   IS NULL AND statement_norm = ? AND (authority_id IS NULL OR
   authority_id = ?) AND (topic IS NULL OR topic = ?)` (covered
   by `idx_rule_match`). Hit → skip with the lowest `rule_id`.
4. Miss → normal §2 insert (with citation offsets + reasoning).

On skip: one `suppression` row (`path='A'|'B'`, `rule_id` only
for B) + one `audit` row in the same per-capture transaction;
the watermark advances exactly as a restatement no-op (§3).
Zero fact/task rows. Audit shape: `actor=
'analysis-agent/<prompt-version>'` (§8 interpolation),
`action='analyze.suppressed'` (new; `analyze.skipped` stays for
restatements/unknown-language), `entity='capture'`,
`entity_id=<capture_id>`, `before_ref=<input capture.sha256>`,
`after_ref='suppression:<row-id>'`, `reason` = JSON
`{"path","rule_id","topic","statement_norm","usage"}` (§7 usage
object). Repair-retry interaction: suppression runs *after* a
record passes validation (a suppressed record never consumes
the repair re-prompt). Determinism (§3): reproducibility is
evaluated on model output *before* suppression filtering.

### 11.4 Review-queue UI page contract (admin Svelte app)

Route `/review`: `web/src/routes/(admin)/review/+page.svelte`
(sibling to `calls/`, `workers/`) + client
`web/src/lib/admin/review-client.js` (sibling to
`calls-client.js`). Data sources are exactly the §11.2
endpoints; the page never invents unreturned state.

Per-item card (one proposed fact), fixed field order:

1. **Header:** `#id`, authority slug, topic chip, `created_by`,
   relative age. Deep-link to `GET .../queue/{id}`.
2. **Diff:** side-by-side (stacked on narrow) `base_statement`
   → `statement`, word-level highlight; "no live sibling —
   first claim on this topic" when `diff` is null.
3. **Citations:** `capture_id` + byte range + verbatim `quote`
   in `<code>` (truncation marked + expandable), linked to the
   sidecar (`provenance.sidecar`).
4. **Reasoning (quarantined):** `reasoning_untrusted` in a
   visually distinct box (amber border, `Untrusted model
   output — verify against citations, not this box` caption);
   null renders "model gave no summary — review citations".
   Never normal prose; never prefills the accept reason.
5. **Blast radius:** conflict banner (reuse the public banner
   component: sibling statements + statuses), `open_tasks`
   count linking to the task view.
6. **Provenance:** capture kind/trust chip (`official` green,
   `staff_verified` blue, `community` grey; rejected captures
   never yield facts per §1 eligibility), `authenticity`
   summary, sidecar excerpt link.

Controls per item (existing CSRF double-submit):

- **Accept** → §11.2.3 without `statement` (one click; the
  audit trail is the undo story).
- **Accept with edit** → textarea prefilled with `statement`,
  live diff preview; submit sends §11.2.3 with `statement`
  (omit the key when unchanged).
- **Reject** → reason textarea (required, 1–1024 counter,
  submit disabled while empty) + collapsed "Also suppress
  future copies": `scope` select (authority default / global)
  + `lock_topic` checkbox (checked default). Submit → §11.2.4.
- After `200`: card collapses to a decided one-liner; after
  `409`: "Already decided — refreshing" + refetch (another
  staffer won; `decide()` is the arbiter).
- Chrome: authority/topic filters, `limit` select, "load more"
  (`after_id`), empty state ("Queue clear"), error state with
  retry, link to the suppressions log view (read-only table).

No optimistic flips (server `decide()` is truth); ≤200 cards,
no virtualization; bookmarkable filter params.

### 11.5 Approve-with-edit audit shape

Exactly one `audit` row per decision (no second row for the
edit — keeps `decide()` single-write):

- `at`, `actor` (staff `user_id`), `action='fact_accept'`
  (unchanged — an edit-accept is still an accept; banner and
  fan-out identical), `entity='fact'`, `before_ref='proposed'`,
  `after_ref='accepted'` (existing pair, unchanged).
- `reason`: unedited → staff reason verbatim (existing).
  Edited → JSON envelope `{"note":"<reason or
  \"\">","edit_before":"<previous statement>"}`; a re-topic adds
  `"topic_before"`/`"topic_after"` (alone when the text is
  unchanged). The ≤1024
  check applies to the final stored value; on overflow,
  `edit_before` truncates with a `…` marker (statements are one
  short claim — never triggers in practice). Consumers tolerate
  both forms (try JSON, fall back to plain note).

Reject-with-rule: one `fact_reject` row + one
`suppress_rule.create` row (two rows, one transaction; the rule
row's `reason` echoes the reject reason).

### 11.6 Open questions

| # | Question | Owner | Blocks |
|---|---|---|---|
| OQ-1 | B8 remainder: named triage owner/rotation + SLO (daily clear?)? | author | staffing only; code can land |
| OQ-2 | Is normalized-exact matching enough for v1, or prefix/contains/regex (phone variants)? Spec leans exact-only. | author | §11.3 scope |
| OQ-3 | `suppression` retention: forever (audit-like) or expire? Default: forever until measured. | author | rollout notes |
| OQ-4 | Topic edits: statement-only editing acceptable, or topic-change needed day 1? | author | §11.2.3 scope |
| OQ-5 | `summary` response field: worth the client+schema addition for the reasoning box, or cut to citations+diff only? Spec leans keep (the author's HITL ask names reasoning). | author | client schema |

Resolved while writing (parent-verified, author can reopen):
OQ-5-orig (citation home) → 011 columns + sidecar slicing;
OQ-7 (admin route homes) → `(admin)/review/` +
`lib/review-client.js`; gmail/kv proposals → superseded
by Phase B (`authenticity` JSON + `mail_poll`);
`in_last_build` → cut (no fact set in `build`).

### 11.7 Rollout (no-flag, ordered)

1. Land migration 011 (additive; safe under the running server).
2. Land analyzer consultation (§11.3) + citation/reasoning
   store (client `summary` field, runner insert). Path A live
   immediately (reads existing `rejected` rows); path B inert
   until rules exist.
3. Land API deltas (§11.2) + `cargo xtask ci` gate.
4. Land `/review` UI (§11.4); drill on staging: propose →
   edit-accept → reject-with-rule → recrawl → confirm
   `analyze.suppressed` + suppression row, `409` on double
   decide.
5. Name the OQ-1 owner.
