# Instruction worker (per-authority instructions + live fact updates)

Atlas researches; this worker operates. It turns arriving evidence
— email captures, customer `comment` captures, call transcripts —
into per-state/county **instructions** and, where the evidence
moves a fact, **proposed fact updates** with before/after values
and reasons. Humans still decide everything: proposals flow
through the existing review queue + `decide()` path (§11); the
worker only proposes, never accepts.

## 1. Non-goals

- No new tables, no migration: triggers derive from `capture`
  rows, cursors live in `watermark` under `instruct:<slug>`
  keys, proposals are `fact(status='proposed')` rows, review
  items are `task(kind='correction')` rows.
- No topic changes: the 25 frozen topics stand. Instruction
  sections map onto topics (see §3); anything unmappable is
  rejected, never shoehorned.
- No auto-accept, no conflict auto-resolve (§11 invariants hold).

## 2. Pipeline (all in code, server-side)

```text
timer (5 min) ──▶ triggers ──▶ findings ──▶ draft ──▶ refine ──▶ diff ──▶ propose
                    (new         (summarize   (template+   (special    (vs accepted  (facts+audit
                    captures     long ones)   findings→    findings    facts →      +task+cursor,
                    > cursor)                 sections)    override)   changes)     one txn)
```

1. **Triggers.** Captures with `id > cursor`, `kind IN
   (email, transcript, comment)`, `trust != rejected`, oldest
   first, per authority. Text comes from `text/<id>.txt`
   sidecars (missing sidecar → skip + audit, analysis
   precedent).
2. **Findings.** Short evidence passes through; long evidence
   is condensed with the Atlas map-reduce summarizer (file
   cache under `<store>/instruct/cache`, keyed by content
   sha). Verbatim names/dates/numbers are preserved by
   contract; the cache makes reruns free.
3. **Draft.** One structured call: general template + the
   authority's accepted facts + findings brief → sections
   (`[{id, topic, heading, body, citations[]}]`). Context is
   budgeted like Atlas (estimate, then live `input_tokens`
   when not dry); over budget fails with a hint, never
   truncates silently.
4. **Refine.** A second pass applies special findings as
   explicit overrides (`[{section, override, reason}]`) —
   even single-authority quirks. Overrides are part of the
   proposal reasons, not hidden edits.
5. **Diff.** Each section body becomes `field: value`
   statements per its topic; statements identical to an
   accepted fact are dropped; the rest become changes with
   BEFORE (accepted statement or ∅), AFTER, and REASON.
6. **Propose.** Validate (topic ∈ frozen enum, citations
   inside sidecar bounds, suppression consult), then one
   transaction: proposed facts (reasoning carries the
   machine-readable header below) + `instruct_proposed`
   audit rows (`before_ref`/`after_ref`/`reason` columns) +
   one `correction` task (dedup key per trigger set,
   instruction markdown in `detail`) + cursor advance.
   Whole-trigger failure writes nothing.

## 3. Template pack (repo data, versioned)

`crates/instruct/packs/general.toml`: `version`, ordered
`[[sections]]` (`id`, `title`, `topic`, `guidance`). The
template states *general* requirements ("every office lists
walk-in hours or says none"); findings supply the
*particular*. Section→topic mapping is fixed in the pack; a
draft section whose topic is not in the frozen enum is a
validation error. Instruction rendering order = pack order.

## 4. Change header (in `fact.reasoning`, round-trip tested)

```text
SUPERSEDES #<accepted fact id | 0 when new>
BEFORE: <old statement | (none)>
AFTER: <new statement>
REASON: <why, citing trigger capture ids>
```

The queue already renders `reasoning` as `reasoning_untrusted`,
so reviewers see before/after inline with zero UI work. A
future column migration can adopt this grammar verbatim.

## 5. CLI + timer surface

```text
cargo xtask instruct-sweep [--store …] [--slug …] [--limit N] [--live]
cargo xtask instruct-demo   # frozen WA drill on a temp store (never prod)
```

Stub port by default (offline-safe, like `analyze`); `--live`
uses the sealed `meta.api_key`. Exit 0/1/2 match the Atlas
conventions. `docs/ops/apostille-instruct.{timer,service}`
fire the sweep every 5 minutes (Persistent); installing them
is operator/root work, like the mail timer.

## 6. Testing

- Unit: header format/parse round-trip, template load +
  topic validation, budget refusal, suppression consult.
- Fixture (Washington state, synthetic): authority + accepted
  facts + three triggers (email fee change, comment hours
  note, transcript Q&A) → instructions markdown golden +
  proposed rows with parsed headers + one task row. Runs
  offline via `StubPort`; `instruct-demo` prints the same
  drill for humans.
- Live API only via explicit `--live`, never in `cargo test`.
