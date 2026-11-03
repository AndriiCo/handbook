# Form Resemblance Framework + DS-4194 Pilot (draft for review)

- Status: draft (goal-025 stream 3 of 5; for author review; pilot
  implemented, not production-wired)
- Date: 2026-10-27
- Decides: goal-025 decision 26 (common framework + per-form
  projects compiled into releases now, dynamic sandboxed hosting
  later; DS-4194 pilot)

## 0. Terms

- **Resemblance form**: an in-browser fill-in page that mirrors an
  authority paper/PDF form's sections, field order, and print
  layout closely enough that a customer can cross-check the screen
  against the official edition. It is NOT the official form and
  never claims to be.
- **Framework**: the shared code in `web/src/lib/forms/` (this
  spec §2). One copy, every resemblance form reuses it.
- **Per-form project**: one directory per authority form edition
  under `forms/<authority>/<code>/` (§3): human-owned
  `REQUIREMENTS.md` plus the machine-readable `v<n>.json`
  template. The `forms/` tree is the content-repo source of
  truth; it extends the `forms/<authority>/<code>/v<n>.json`
  layout that `crates/forms` `TemplateRegistry::load_dir`
  already reads (P2-F4/F5).
- **Generated copy**: the `web/src/lib/forms/generated/` JSON
  bundled into the web release (§5). Never hand-edited; the
  `form-scaffold --check` gate (§4) keeps it byte-identical to
  the `forms/` source.

## 1. Goals and non-goals

- G1: One shared framework for all PDF-like webforms:
  validation, address input/validation, phone formatting,
  country dropdown/autocomplete. Per-form code holds only what
  differs (sections, order, print CSS, help text).
- G2: An iterative, documented path from an official form PDF
  to a shippable resemblance page: PDF → `REQUIREMENTS.md` →
  `v1.json` → scaffolded route → visual compare → publish.
- G3: Pilot the path end-to-end on US DOS DS-4194 ("Request
  for Authentications Service", OMB 1405-0254 per third-party
  copies — verify against eforms.state.gov before production).
- Non-goals: server-rendered replica PDFs (P2 printpdf track
  owns that); drawn signatures (P2 wet-ink-only, decision
  record §14–15 open); submission/payment semantics for any
  form (never invented — §6.3); dynamic per-tenant form
  hosting (future, §5).

## 2. Component catalog (`web/src/lib/forms/`)

All modules are dependency-free JS + Svelte 5 runes components.
JS helpers carry JSDoc types and node:test coverage in
`web/test/forms-framework.test.mjs`. Visual components rely on
`svelte-check`, `vite build`, and the route smoke.

| Module | Exports | Contract |
|---|---|---|
| `validation.js` | re-exports `isBlank`, `parseMoneyCents`, `validateField`, `validateSubmission` from `../customer/forms-validate.js`; adds `errorsForKey`, `isValid` | Error codes AND messages stay identical to the Rust `crates/forms` validator; the vectors file pins both. Framework code never invents a code. |
| `phone.js` | `formatPhone`, `normalizePhoneDigits` | NANP display format `(555) 123-4567` / `+1 (555) 123-4567`; charset-compatible with the `tel` rule; non-NANP input passes through unmangled. |
| `countries.js` | `COUNTRIES`, `searchCountries`, `isCountryName`, `countryNameForCode` | Static ISO 3166-1 list (code + short name); prefix matches rank before substring matches; pure functions, no locale data fetched. |
| `address.js` | `US_STATES`, `isUsStateCode`, `stateNameForCode`, `missingAddressParts`, `formatAddressOneLine` | Parts are exactly `name/street/city/state/zip` (the `address_block` contract); messages mirror the `address`/`required` codes. |
| `CountrySelect.svelte` | autocomplete combobox | Binds the full country name; filters via `searchCountries`; keyboard-operable listbox; free text allowed (validation decides). |
| `PhoneInput.svelte` | formatting text input | Binds raw text; formats NANP on blur; never blocks typing. |
| `AddressBlock.svelte` | five-part address editor | Binds `{name,street,city,state,zip}` (+ optional `line2` display-only); state is a `CountrySelect`-style datalist-free select. |

Rules for framework code:

- F1: Framework modules never import per-form code or routes.
  Dependency direction is route → framework → `forms-validate`.
- F2: No PII-bearing defaults, no network, no storage keys.
  Draft persistence (if a route wants it) lives in the route.
- F3: New shared behavior needs a node:test case before the
  route uses it.
- F4: Visual style follows the anti-slop preflight
  (bundled:taste): government-form aesthetic is black on
  white, system fonts, no accent gradients, real hierarchy.

## 3. Per-form project layout

```text
forms/<authority>/<code>/
  REQUIREMENTS.md   # human-owned: source PDF identity, section map,
                    # field table (each row: key, label, type, required?,
                    #   verification status), TBD list, print notes
  v1.json           # FormTemplate JSON (P2-F1/F2 schema), status draft
  # v2.json …       # later editions; publish chains via the registry
```

Conventions:

- `<authority>` is the store slug (`us-dos`); `<code>` is the
  authority's form code lowercased (`ds-4194`).
- `REQUIREMENTS.md` is written BEFORE `v<n>.json` and reviewed
  against the source PDF page by page. Every field row carries
  one of: `verified` (seen on the official PDF), `corroborated`
  (≥2 independent public copies agree), `TBD` (unverifiable —
  never shipped as fact, never given invented semantics).
- `v<n>.json` must deserialize as `crates/forms::FormTemplate`
  and pass `validate_schema()` (unique keys, compilable
  patterns, sane `PdfRect`s, options on choice types).
  `PdfRect` coordinates start as measured-from-scan
  placeholders and say so in `REQUIREMENTS.md` until measured.
- `authority_revision` is the authority's edition mark (e.g.
  OMB control number + expiry) or `TBD` — never our version.
- `accepts_replica` starts `unknown` (P2-A1): routes render
  the replica notice until staff research lands.

## 4. xtask scaffold/compare workflow

```text
cargo xtask form-scaffold --authority us-dos --code ds-4194 \
    --title "Request for Authentications Service"
```

- Creates `forms/<authority>/<code>/REQUIREMENTS.md` (field-table
  template with the verification-status column) and `v1.json`
  (minimal valid `FormTemplate`, `status: draft`, no fields).
- Copies `v1.json` to the generated web copy
  `web/src/lib/forms/generated/<authority>__<code>.json`
  (double underscore: slugs/codes never contain `_`).
- Refuses to overwrite without `--force`; validates the
  authority/code charset (`[a-z0-9-]+`).
- `--out <dir>` roots the tree elsewhere (tests use tempdirs).

```text
cargo xtask form-scaffold --check --authority us-dos --code ds-4194
```

- Byte-compares `forms/.../v1.json` against the generated copy
  (latest `v<n>.json` wins when several exist) and fails with a
  field-key diff when they diverge. Run in CI once a second
  form lands (today: manual + documented).

Iterative resemblance loop (per form, all human-driven):

1. `form-scaffold` the skeleton.
2. Fill `REQUIREMENTS.md` from the official PDF (§3 table).
3. Author `v<n>.json`; validate with
   `cargo test -p handbook-forms` round-trip + the registry
   loader (a second form adds a `forms/` tree fixture test).
4. Re-run scaffold (or copy) to refresh the generated JSON;
   `--check` passes.
5. Build the route from the framework catalog (§2) + §6
   template; print-compare against the PDF; record deltas in
   `REQUIREMENTS.md` §"Resemblance notes".

## 5. Hosting: compile-into-release now, sandboxed dynamic later

**Now (this spec):** per-form JSON + route code compile into
the web release. The generated JSON is a static import, so the
route works offline, adds no API surface, and cannot leak one
tenant's draft to another. Cost: every form edition is a code
release (acceptable for the pilot phase; editions change
rarely and each needs review anyway).

**Later (not this spec):** dynamic sandboxed hosting — staff
publish `v<n>.json` to the store, the P2 template API serves
it, and the generic guided-fill route renders it without a
release. Requires, before it ships: JSON-schema validation at
publish, CSP + sandboxed iframe or strict allowlist rendering
(staff-authored layout must not become stored XSS), edition
pinning per in-flight submission (P2-U2 already specs the
diff), and the SEC lane's sign-off. No code in this spec may
assume that future: routes import generated JSON statically
and never `fetch()` a template by user-supplied path.

## 6. Pilot: US DOS DS-4194

- Source: `forms/us-dos/ds-4194/` (`REQUIREMENTS.md` + `v1.json`).
- Route: `/(customer)/forms/ds-4194` — fill with inline
  framework validation, review step, US-Letter print CSS,
  replica-unconfirmed banner. No submit button wires to any
  API: printing/saving the filled page is the only output
  until DOS submission semantics are verified (§6.3).
- Sections (per corroborating public copies; granular fields
  TBD in `REQUIREMENTS.md`): 1 requester contact, 2
  courier/representative, 3 return mailing address, 4 document
  information (countries of use, count, type), 5 projected
  cost (TBD — and fee display is blocked by decision 10
  regardless), wet-ink signature + date lines.

### 6.1 Coverage bar for the pilot

Every `verified`/`corroborated` field in `REQUIREMENTS.md`
appears on the route with the framework component matching
its type; every `TBD` field is absent from `v1.json` and
listed in `REQUIREMENTS.md` §"TBD". The route's section map
is checked at load against the template keys (unknown keys
warn in console, never crash).

### 6.2 Budgets

`cargo xtask web-budget` reports the built size with the pilot
built (advisory since 2026-10-28 — the Day-1 gate was removed; no
budget to raise). The framework + pilot stay code-split per route
regardless.

### 6.3 Submission semantics: never invented

The pilot asserts nothing about how DOS accepts DS-4194
(mail/walk-in/courier), what it costs, how long it takes, or
whether a printed replica is accepted. The route copy says
"draft resemblance — verify against the official form" and
links nothing it cannot verify. Fee, SLA, and acceptance
claims need staff research + numbered-table supply per
decisions 10/14–16.

## 7. Acceptance criteria

- A1: `docs/specs/form-framework.md` (this file) reviewed;
  component catalog (§2) matches the shipped modules.
- A2: `web/src/lib/forms/*` ship with node:test coverage for
  every JS export's contract; `npm run check` passes.
- A3: `cargo xtask form-scaffold` creates the §3 skeleton and
  `--check` catches drift (rust tests, tempdirs only).
- A4: DS-4194 pilot route renders all non-TBD fields, prints
  on US Letter, shows the replica banner, and posts nothing.
- A5: `cargo xtask ci` + `npm run build && build:admin &&
  smoke` green (`cargo xtask web-budget` reports size, advisory).
- A6: Synthetic fixtures only; no PII, no invented DOS
  semantics; TBDs explicit in `REQUIREMENTS.md`.

## 8. Open questions

- OQ-F1: Who measures `PdfRect`s from the official DS-4194
  scan, and where does the scan live (content-repo TBD,
  decision 2)? Blocks printpdf rendering, not the pilot.
- OQ-F2: CI wiring for `form-scaffold --check` — now or with
  the second form? (Spec says second form; cheap to move up.)
- OQ-F3: Country-name localization (EN-only slice ships
  English names; ES scope is decision 17–18).
- OQ-F4: `forms/` tree ownership when the content repo splits
  off (decision 2): move or git-subtree? Non-blocking.
