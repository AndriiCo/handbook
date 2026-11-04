# Jurisdiction packs (F13: data, not code)

One JSON file per jurisdiction. Adding a country is a data task:
add a pack here, run `cargo xtask packs-validate`, then
`cargo xtask packs-load --store <path>`. Zero code changes.

## Trust rules (non-negotiable)

- Every `chain_steps[]` entry and every `observations[]` entry has
  `"status": "needs_review"`. No other value validates. A step flips
  to usable only via a staff-verified capture + the F4 approval path
  in the store — never by editing a pack.
- `packs-load` writes `authority`, `domain`, `source`, and `task`
  rows only. It NEVER writes `fact` or `capture` rows. Fees and
  addresses in packs are therefore never presentable as handbook
  facts; each one carries its `source_uri` and a `validation`
  backlog ref (V-1…V-19, P3-a/b, F2).
- Canonical PO-Box fixtures (`40220`/`40234`) must never appear in
  any pack. `PO Box 40228` (usa-wa) is a cited observation from the
  superseded WA form (V-5), not a fixture. A cited, staff-accepted
  fact may still carry WA's real PO Box 40234 (specs/README.md
  resolution 9, amended 2026-11-03); packs never do.
- Research traffic (fetching any `watch[].uri`) originates ONLY from
  the prod IPs per `docs/ops/deploy.md` — never from dev machines.

## Schema (version 1)

```jsonc
{
  "pack": "usa-wa",            // unique id, must equal the filename stem
  "version": 1,                // must be 1
  "title": "…",
  "authorities": [             // → authority rows (kind/country/region/name/slug)
    {"kind": "state", "country": "USA", "region": "Washington",
     "name": "…", "slug": "usa-wa-sos"}
  ],
  "domains": [                 // → domain rows (anti-spoofing verdicts)
    {"name": "sos.wa.gov", "authority": "usa-wa-sos",
     "status": "unverified", "notes": "…"}
  ],
  "watch": [                   // → source rows (thin-watch seeds)
    {"authority": "usa-wa-sos", "kind": "website_official",
     "uri": "https://…", "cadence": "daily", "notes": "…"}
  ],
  "chain_steps": [             // → task(kind=validate_fact) rows, status open
    {"id": "wa-sos-auth", "authority": "usa-wa-sos", "order": 1,
     "title": "…", "detail": "…",
     "status": "needs_review", "validation": "V-5"}
  ],
  "observations": [            // stay in the pack; become tasks, never facts
    {"authority": "usa-wa-sos", "topic": "fees",
     "statement": "…", "source_uri": "https://…",
     "status": "needs_review", "validation": "V-5"}
  ]
}
```

## Schema additions (v2, all optional; still `"version": 1`)

Pack level: `"iso2": "DE"` (ISO 3166-1) applies to every authority.
Per authority, beside `kind/country/region/name/slug`:

```jsonc
{
  "hidden": true,                 // hide from every public read; false unhides; absent leaves it
  "merges_into": "usa-wa-sos",    // this entry IS another pack's office: no row of its own,
                                  // its links/tasks/routes attach there (replaces slug sharing)
  "iso2": "US", "subdivision": "US-WA",   // ISO 3166-1 / 3166-2
  "level": "state",               // federal | state | regional | local | consular
  "name_en": "…", "aliases": ["Bayern"],  // searched beside `name`
  "paths": ["usa/new_york/chemung"],      // readable addresses, primary first
  "scope": [                      // document routes → authority_scope rows + one review task
    {"doc_class": "vital_record", "issued_in": "US-WA", "prereq": "…",
     "source_uri": "https://…"}   // doc_class: vital_record | state_court | federal_court |
  ],                              // notarized_private | company | state_agency |
                                  // federal_agency | education | fbi_check
  "source_name": "Land"           // the extracted text a curated `name` replaced
}
```

A load brings an existing slug in line with its pack (name, kind,
place, v2 fields) with one audited `authority_update` row, so a
corrected pack fixes production. A route stays unverified on the
place pages (`/in/usa/washington`) until staff accept a `jurisdiction`
fact `doc_rule: <doc_class> …` for that office; packs never write facts.

`packs-validate` walks the whole tree (`packs/`, `packs/world/`,
`packs/world-nonparty/`): a slug has one owning pack, every
`merges_into` target exists, and every public name passes the name
lint (no lowercase start, no trailing `:`/`,`, no phone, fax, e-mail,
postcode or street line, more than 6 characters). A fragment that
names no office is kept with `"hidden": true` so the load hides the
row it once created.

Vocabularies: `kind` per migration 005 (`authority.kind`,
`source.kind`); `topic` must be one of the 25 `contracts::Topic`
strings; `cadence` is `hourly|daily|weekly`; every `uri`/`source_uri`
is `http(s)`; every watched host must be declared in the same pack's
`domains` (so each seed has a verdict row).

## Thin-watch seam (contract for the fetch-url owner)

`packs-load` stores each watch seed as one `source` row with
`robots=''` (verdict pending) and `notes='cadence:<v>; <notes>'`.
The future `fetch-url` task needs no pack knowledge: it enumerates
`source` rows, parses the `cadence:` prefix from `notes`, and resolves
`domain_id` through the `domain` table. See the parent report for the
suggested patch text.

## Acceptance (end-to-end, admin-track consumable)

```sh
cargo xtask packs-validate          # all packs parse + invariants hold
rm -rf /tmp/pack-accept && cargo xtask packs-load --store /tmp/pack-accept
# then assert: authorities/sources/tasks present, zero fact rows:
# sqlite3 /tmp/pack-accept/handbook.sqlite \
#   "SELECT (SELECT COUNT(*) FROM authority), (SELECT COUNT(*) FROM source),
#           (SELECT COUNT(*) FROM task), (SELECT COUNT(*) FROM fact);"
```

The admin UI config-acceptance (admin track, G4) runs the same two
commands against its staging store and asserts the same shape before
any pack-derived value is displayable.

## North-America slice (SA-17: 106 issuing offices)

`packs/world/US.json` (59), `packs/world/MX.json` (33) and
`packs/world/CA.json` (14) hold **106 authorities, one per issuing
office**: US 50 states + DC + 5 territories + 3 federal units (DOS
Authentications, DOS Vital Records, federal court clerks); Mexico 32
federal entities + SEGOB/DICOPPU federal; Canada 13
provinces/territories + Global Affairs Canada federal. Every
authority cites its official site in `watch[]` + `observations[]`;
every `chain_steps[]`/`observations[]` entry is `needs_review`
(staff capture pending). Validate with `cargo xtask packs-validate` (the whole tree). These
three are hand-built: `world-index --live` never rewrites them.
