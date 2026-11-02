# agent-cites: a third-party agent cites the handbook

SA-13 demo surface (idea 11, Meta tooling breadth). One command lets an
outside agent ask the handbook a question; Muse Spark consults the
public facts endpoint through exactly one tool and answers with
verbatim quotes cited to `source_url` + `captured_at` — or refuses
when no fact covers the question.

## Live (staging; key via env only, never a file)

```sh
META_API_KEY=<staging key> cargo xtask ask \
  "What is the per-document authentication fee in Washington State?" \
  --slug usa-wa-sos
```

`--base` picks the handbook to cite (default
`http://127.0.0.1:18318/apostille`; production and localhost bases
work via the flag). `--meta-base` overrides the Meta API root.

## Offline replay (zero network; what CI runs)

```sh
cargo xtask ask "?" --slug fixture \
  --replay examples/agent-cites/fixtures/ask-usa-wa-sos-fee.json
```

`"?"` / `fixture` reuse the recorded question and slug.

## Refusal

A question no fact covers exits 1 with a clear message:

```sh
cargo xtask ask "What are the fees on another planet?" --slug usa-wa-sos \
  --replay examples/agent-cites/fixtures/ask-usa-wa-sos-fee.json
# ask: refused — no fact covers the question
```

(The replay fixture only records the fee exchange, so any other
question replays to the same refusal. Live, the model reads the real
facts first and refuses only when nothing covers the question.)

## Files

- `fixtures/ask-usa-wa-sos-fee.json` — recorded-shape replay fixture:
  question, slug, facts payload in the exact endpoint envelope, the
  two Spark exchanges (tool call, cited final), run metadata.
- `demo-beat.md` — the 10-second demo beat with expected output.
