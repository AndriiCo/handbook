# evals/world-nonparty — recorded UN member-state list (public text)

Input fixture for the `research.nonparty` work-queue job (P392; atlas
pack `crates/atlas/packs/nonparty-sources.toml`, worker
`xtask/src/worker/research_nonparty.rs`). The job's country list is
deterministic: UN member states in this file minus the HCCH 1961
parties in `evals/world/status-table-cid41.html`, plus the Holy See
and the State of Palestine. No live network in tests: every unit test
reads these files.

## Provenance

- `un-member-states.html` — the UN member-states page
  (`https://www.un.org/en/about-us/member-states`, English, 193
  members). Recorded 2026-11-03, 200 748 bytes, sha256
  `8842747ec8ac9b4c0ae0bb37d47b6877e3f5b23fca7bcbb5d9fbbbd32b0038c3`.
  The page lists Nauru as "Naoero"; the job prints "Nauru".

Recorded with the repo's own fetch code (`ingest::fetch::fetch_url`,
user agent `handbook-watch/0.1`), through the worker's `scrape.urls`
kind run locally:

```sh
cat > /tmp/un.json <<'JOB'
{"kind": "scrape.urls", "payload": {"urls": ["https://www.un.org/en/about-us/member-states"]}}
JOB
cargo xtask worker --run-local /tmp/un.json --workdir /tmp/un-record
# then copy /tmp/un-record/local-*/pages/<sha256> here as un-member-states.html
```

The file is byte-identical to the fetch (its sha256 is the `pages/`
name the job printed); nothing was edited by hand.

## Re-recording

Re-run the job above and replace the file. The parser
(`un_members`) reads every `<h2>` whose text is a member name it knows
(`un_iso2`); the test `country_list_is_un_members_minus_parties_plus_observers`
pins 193 members and the non-party count, so a site redesign or a new
member fails loudly instead of silently shrinking the list.
