# Prompt 049 — live-model plan: Telnyx call, Meta reasoning app, RAG/KG, HITL pipeline

- Prompt timestamp (UTC): 2026-10-28T20:49:17Z
- Response timestamp (UTC): 2026-10-28T21:30:00Z
- Author: Andrii
- Agent/model: Muse Code (Muse Spark)

## Prompt

```text
Move with a plan. What about tools that will be analysing the data collected? Can we place a call today via Telnyx? Did we build a tool that will propose authorities based on convention website? Do not build a list yourself, build me a Rust app that will reason using Meta Model APIs and keep it update. We do not need build once and get stale in 2 months data, we need to be able keep our data fresh. Is there any industry accepted practices to buuild this RAG / knowdledge graph / etc in order to use it efficiently with Meta Models? Any code example to follow (maybe from others models, but apply it now to Meta Models one)? We need to have efficient process to act on any new details - from getting a email with office closure tommorow to publication on website it must take one click for human operator to review the changes, their reasoning and approve it. No YOLO mode due to risks to hallucinations. In case if change is not approved - operator can provide feedback so similar changes will not proposed again and again.
```

(Verbatim, typos preserved.)

## Response

Answered all six questions from repo evidence, fanned out three
read-only researchers (full texts recovered from child session logs
after truncated delivery), committed pinned Meta API shapes, and
returned a build plan with four author decisions. No code changed;
no implementation started (spec-before-surface: review-queue UI +
schema deltas need a reviewed spec first).

Direct answers given:

1. Analysis tools: exist as a stub-driven pipeline — `xtask analyze`
   (Option-B watermark scan, `--scan-captures` closes old B6) +
   `crates/handbook-agent` (runner, byte-exact citation validator,
   PII redaction, all tested) + `review-conflicts` CLI + staff
   approve/reject API + accepted-only publish. Gaps: live model
   unwired (`MetaRestClient::draft_records` = honest "not
   implemented", readiness B1), key/terms gate open (B2), no sweep
   scheduler (B5), no HCCH poll writer (B10), no review UI (B8).
2. Telnyx call today: YES if the author provisions now — dial path is
   complete (`POST .../admin/calls/dial` → `TelnyxClient::dial` →
   `POST /v2/calls`, consent-gated, 503-unconfigured, tested vs
   stub) and the admin calls UI exists (`web/src/routes/(admin)/
   calls/`). Checklist returned: portal Voice app + outbound
   profile/number, sealed `telnyx.api_key`/`connection_id`/
   `from_number`/`webhook_public_key` via admin config UI, staff
   session, dial with consent from the calls page.
3. Authorities-proposal tool: NO — only static packs seed,
   `fetch-slug` manual sweep, and the pure `derive_routing` fn.
   Folded into the plan as HCCH poll + discovery (never a
   hand-built list, per the author's constraint).
4. "Rust app" shape: NOT a new binary — `xtask analyze` +
   `handbook-agent` + systemd timers, per design §8 and
   analysis-agent §3 Option B. Stated explicitly for author veto.
5. RAG/KG (researcher R2): our FTS5 + frozen topics + byte-exact
   validator already IS the accepted keep-fresh extraction pattern
   (ALCE-stricter citations; RAGAS/DeepEval provably never check
   citations); conflict query is a SQL self-join, not a graph
   traversal. ADOPT: conditional GET + fetch state, excerpt offsets
   (text-splitter), readability pilot, golden eval, SimHash.
   SKIP with triggers: tantivy, sqlite-vec (conditional: discovery
   only), qdrant, SurrealDB, spider, rig dep (copy its OpenAI-
   compat call shape, not the crate), LLM-judge frameworks.
   Refs: 0xPlaygrounds/rig (call shape), spider-rs politeness
   skills, fastembed-rs + sqlite-vec (conditional).
6. HITL pipeline (researcher R3): queue-not-pause (we match —
   `fact(proposed)` IS the queue); review screen = diff + verbatim
   citations + subordinate reasoning + blast radius + provenance;
   pessimistic apply + "approve with edit"; suppression = A
   (auto dedup-key on every reject) + B (operator negative rules,
   scoped + 90d TTL), defer embedding dedup (could silently eat a
   real fee change); Gmail historyId-watermark polling via `xtask
   mail-poll` on a timer; trust mapping requires dkim/spf pass and
   DMARC for `official`. Full schema/API sketch in the child log.

Meta API verification (researcher R1, committed): base URL + Bearer
confirmed; upstream env `MODEL_API_KEY` (ours is a local alias);
structured output + prompt caching confirmed; NO embeddings
endpoint (404 — kills any Meta-hosted vector path); limits 3000
RPM/4M TPM (irrelevant); ~$22/mo Standard, Contributor trains on
content (no PII). Shapes pinned in `docs/meta-api-shapes.md`
(new), digest amended.

Build plan returned (work order): keys/terms (author) → B1 live
client + `--check` probe → sweep + HCCH poll + timers → mail-poll
→ review spec (for author review) → review API/UI + suppression →
eval throughout. No-YOLO invariant preserved: only `decide()`
under AdminAuth writes `accepted`; suppression skips proposals,
never approves.

Author decisions requested: D1 provision META key + read terms;
D2 Telnyx creds for a live call; D3 Gmail OAuth for mail-poll;
D4 approve plan + "no new binary" framing (else veto).

## Commits

- `97b4081` docs: pin Meta API JSON shapes, verify digest
- (this log entry commits separately, per convention)

## Verification

- Docs-only turn; code tree untouched, no `cargo xtask ci` needed.
  Staged-hook secrets-scan clean (2 files).
- All researcher claims used above were recovered in full from
  child session logs (`.../subagent/01a18442-c57b-*/session.jsonl`,
  `-c8c1-*`, `-cb5c-*`), per the truncation-proof handoff rule —
  never from truncated delivery summaries.
- Repo-grounded answers cite file + line (Telnyx dial, agent
  model.rs:92, xtask analyze/scan-captures, readiness B1–B10).

## Follow-ups

- Author: D1–D4 (keys, terms, Telnyx, Gmail, plan approval).
- Then: B1 implementation + review-queue spec draft for review.
- OPEN-32 + U1–U9 still await the earlier approval (prompt 048).
