---
name: ai-slice
description: Make a Meta model (muse-spark-1.3, sam-3.1, muse-voice-transcribe-1.0) do visible, measured work in the Apostille Handbook - scoreboard rows GP2-data, A1 quote-verified citations, A2 SAM photo precheck, A5 eval/quality numbers, the trace console - with every call shaped as Meta documents it. Use for any change to crates/handbook-agent, crates/media, crates/atlas, crates/instruct, evals, xtask eval or xtask/ops/a2-record.py. While a lane owns some of these paths (goal.md "Lanes"), only that lane edits them; others send it the request.
---

# ai-slice

Judges score Meta AI tooling (reported 25%) and technical depth (20%). A
model call that nobody sees and nobody measured scores zero, and a call
that ignores Meta's documented shape is a bug. Every AI slice ends with a
live run, replay fixtures and a number.

## Rules for every Meta call

`.agents/meta-api-findings.md` is the checklist: Meta's documented request
shapes (chat `response_format.json_schema` vs Responses flat
`text.format`), no `temperature` (Spark is tuned at the default), token caps
that leave room for reasoning, explicit `reasoning_effort`, stable prompt
prefix first for caching, refusal and truncation handling, retries with
jitter honouring `Retry-After`, SAM with one bare noun phrase per request,
speech-to-text as multipart `request` JSON + `audio` WAV, and the data
rules. Cite the official page (`https://dev.meta.ai/docs/<page>.md`) for any
shape you change.

## Patterns already in the code (reuse them)

- Quote verification: done. `crates/handbook-agent/src/analyze.rs` checks
  the quote byte-exact against the capture with server-side offsets, and
  `crates/handbook-agent/src/runner.rs` retries once with the refusal. The
  legal-agent-workshop's `quote_on_page`
  (`/Users/andrii/Code/meta-models/legal-agent-workshop/solutions/02_session2/agent/agent_kit/document_text.py`)
  matches normalized text instead (whitespace, quote marks, case). The
  topic is a JSON-schema enum from `Topic::ALL`.
- SAM 3.1: `POST /v1/responses`, `model: "sam-3.1"`, one `input_text`
  noun phrase + one `input_image` data URL; parse the
  `<|box;x1;y1;x2;y2;w;h|>` tokens in `output_text` by object id; empty
  output means zero matches (workshop `agent_kit/tools/check_signatures.py`).
- Evaluation: deterministic checks first (quote found, value in quote,
  topic match), each computed independently; gold facts in
  `evals/authorities/*.json`, outside the store and the prompt;
  `cargo xtask eval --live`.

## Done means

1. The contract in `.agents/goal.md` for the row holds (field names exact).
2. A live run on public inputs or synthetic documents, on plant with the
   staging key (`~/.config/apostille/meta-staging.env`, sourced in the
   remote shell only): model, p50/p95 latency, input/output/reasoning/
   cached tokens, refusal-then-fixed count.
3. The recorded responses are committed as replay fixtures, and the
   crate's tests replay them (`cargo test -p <crate>`; the crate's clippy).
4. For a quality change: the eval before → after on all three rates
   (production's 11-01 04:29Z run, `muse-spark-1.3`: quote_found 1.0,
   value_in_quote 0.969, topic_accuracy 0.641); none may drop by more than
   one gold case.
5. `node .agents/scoreboard/scoreboard.mjs --only <row>` against staging
   (AGENTS.md §8, `--base http://127.0.0.1:18318/apostille`), then against
   production after the next deploy.

Synthetic documents or public pages only. Nothing but synthetic or public
text ever goes to a `-contributor` model, which trains on its inputs. No
key values in fixtures, logs or messages. A public AI endpoint is
rate-limited and cost-capped before it ships.

Facts become public only after the operator approves them in `/review`.
Queue the proposals, add "approve N proposals for <slug>" to
`docs/ops/operator-today.md`, and continue with the next item.
