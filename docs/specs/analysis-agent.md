# Analysis Agent Job Spec (draft for review)

- Status: draft (prompt 007, for author review; no implementation yet)
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
2. `task` rows with `dedup_key = sha256(authority_id + normalized
   topic)` per §4. Kinds: `validate_fact` (new/changed claim),
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
test` and `dev-seed`). Runner code depends only on `ModelClient`.
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
