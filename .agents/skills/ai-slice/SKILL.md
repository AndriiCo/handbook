---
name: ai-slice
description: Make a Meta model (muse-spark-1.3, sam-3.1) do visible, measured work in the Apostille Handbook - scoreboard rows GP2-data, A1 quote-verified citations, A2 SAM photo precheck, A5 eval/quality numbers, plus the frozen demo world and the agent trace console. Use for any change to crates/handbook-agent, crates/media, crates/atlas, evals or xtask eval/world commands.
---

# ai-slice

Judges score Meta AI tooling (reported 25%) and technical depth (20%). A
model call that nobody sees and nobody measured scores zero. Every AI slice
ends with a live run, replay fixtures and a number.

## Patterns to reuse (read the source, port to Rust)

- Quote verification:
  `/Users/andrii/Code/meta-models/legal-agent-workshop/agent/agent_kit/document_text.py:46-52`
  (`normalize` + `quote_on_page`, minimum 8 characters) and `case_record.py:243`
  (`record_fact` refuses a quote that is not on the page and returns the
  error to the model). Replace the offset-only check in
  `crates/handbook-agent/src/analyze.rs:61-72`; compute offsets server-side;
  check the value appears inside the quote; one retry with the refusal as a
  tool error. Emit `Topic::ALL` from `crates/contracts/src/topics.rs` as a
  JSON-schema enum in `crates/handbook-agent/src/model.rs` (it is a free
  string today), with a `reasoning` field before `records`.
- SAM 3.1:
  `legal-agent-workshop/agent/agent_kit/tools/check_signatures.py:117-160`:
  `POST /v1/responses` with `model: "sam-3.1"`, `input_text` + `input_image`
  data URL; boxes parsed from `<|box;…|>` tokens in `output_text`.
  `crates/media/src/redact.rs:19,24,168` builds `/v1/v1/…` today; the test
  at `:346` asserts the doubled path. Prompts that work: "ink scribble",
  "official seal", "embossed stamp", "face".
- Frozen world: `legal-agent-workshop/world/release.py` (sha256-checked
  captures). Name the command `cargo xtask world load <name>`, never
  "release" (xtask release already means deploy).
- Evaluation: `building-agentic-ai` structured grader. Deterministic checks
  first (quote found, value in quote, topic match); an LLM judge only for
  "same meaning" and only after it agrees with hand labels. Gold facts in
  `evals/authorities/*.json`, outside the store and the prompt.

## Done means

1. The contract in `.agents/goal.md` for the row holds (field names exact).
2. A live run on production inputs: model, p50/p95 latency, input/output
   tokens, refusal-then-fixed count. Spark vision goes through the atlas
   Responses client, not the text-only chat client.
3. The recorded responses are committed as replay fixtures, and the
   crate's tests replay them (`cargo test -p <crate>`; the crate's clippy).
4. For a quality change: the eval before → after.
5. `node .agents/scoreboard/scoreboard.mjs --only <row>` against
   production after the deploy, or against local with the reason it is not
   deployed yet.

Synthetic documents only: Contributor-tier keys may train on inputs. No key
values in fixtures, logs or messages. A public AI endpoint is rate-limited
and cost-capped before it ships.

Facts become public only after the operator approves them in `/review`.
Queue the proposals, add "approve N proposals for <slug>" to
`docs/ops/operator-today.md`, and continue with the next row.
