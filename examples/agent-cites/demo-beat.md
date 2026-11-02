# Demo beat: agent cites us (10 seconds)

One question, one tool call, one verified quote. Replay runs below
are byte-identical to what the command prints (zero network); the
live staging take replaces `--replay …` with `META_API_KEY=<key>`
and the same question.

## Beat (say while typing)

1. "An outside agent asks the handbook a question." (2s)
2. "Spark looks up the authority through one tool — no browsing."
   (3s)
3. "The answer quotes the fact verbatim, with its source and
   capture time — or the command refuses." (5s)

## Take 1 — cited answer (replay; exit 0)

```sh
cargo xtask ask "?" --slug fixture \
  --replay examples/agent-cites/fixtures/ask-usa-wa-sos-fee.json
```

Expected output (stdout, then the stderr ledger line):

```text
The per-document authentication fee in Washington State is $15 per document.
[fact 2] "$15 per document" (source: https://www.sos.wa.gov/authentication/fees; captured: 2026-10-30T04:29:00Z)

Citations verified: 1
- [fact 2] source: https://www.sos.wa.gov/authentication/fees captured: 2026-10-30T04:29:00Z
ask: model=muse-spark-1.3 slug=usa-wa-sos latency_ms=8 input_tokens=2400 output_tokens=180 reasoning_tokens=120 cached_tokens=800
```

(`latency_ms` varies run to run; token counts are the recorded
exchange totals.)

## Take 2 — refusal (replay; exit 1)

```sh
cargo xtask ask "?" --slug fixture \
  --replay examples/agent-cites/fixtures/ask-usa-wa-sos-uncovered.json
```

Expected output (stderr):

```text
ask: refused — no fact covers the question
```

Exit code 1. No quote, no citation, no silent empty answer.

## Live staging take (pending — builder runs on plant)

```sh
META_API_KEY=<staging key> cargo xtask ask \
  "What is the per-document authentication fee in Washington State?" \
  --slug usa-wa-sos
```

Expect the same shape against real staging facts: the tool result
carries the live `source_url` + `captured_at`, the quote is checked
byte-verbatim against the fetched payload, and the stderr ledger
reports the real model, latency and token counts. Record that run
over `fixtures/ask-usa-wa-sos-fee.json` (keep `run.live: false`
until then).
