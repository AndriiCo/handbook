# DS-4194 — Request for Authentications Service (US DOS)

Per-form project for the form-resemblance pilot (goal-025,
[form-framework.md](../../docs/specs/form-framework.md)). This file
is the human-owned source; `v1.json` is the machine-readable
`FormTemplate` compiled from it.

- Authority: US Department of State, Office of Authentications
  (`authority: us-dos`, `code: ds-4194`)
- Official edition: title "REQUEST FOR AUTHENTICATIONS SERVICE".
  OMB control number 1405-0254 appears on third-party copies —
  **verify against eforms.state.gov before production**.
- Source status: built from corroborating public copies only
  (≥2 independent snippets agreeing on section structure). No
  official PDF has been measured yet. Every unverifiable item
  below is marked TBD and stays out of `v1.json`.

## Section map (corroborated)

1. Requester contact — name, formal mailing address (Line 1,
   Line 2, City, State, ZIP, Country), daytime phone, email.
2. Courier/representative — "Are you submitting/retrieving this
   request on behalf of another individual?"
3. Return mailing — shipping address + "Same address as above".
4. Document information — country (or countries) of use, number
   of documents, document type.
5. Projected cost — TBD (fee display additionally blocked by
   author decision 10: no prices until the numbered table is
   supplied).
6. Signature + date — wet ink only (P2 wet-ink-only rule).

## Field table

Status: `verified` = seen on the official PDF; `corroborated`
= ≥2 independent public copies agree; `TBD` = unverifiable.

| Key | Label | Type | Required | Status | Notes |
|---|---|---|---|---|---|
| `requester_name` | Full name | text | yes (TBD) | corroborated | Exact paper label TBD |
| `requester_address` | Formal mailing address | address_block | yes (TBD) | corroborated | Paper splits street into Line 1/Line 2; framework `line2` covers it |
| `requester_phone` | Daytime phone | tel | yes (TBD) | corroborated | |
| `requester_email` | Email | email | no (TBD) | corroborated | Required-ness TBD; conservative `false` |
| `on_behalf` | Submitting/retrieving on behalf of another individual? | radio yes/no | yes (TBD) | corroborated | Question text corroborated verbatim |
| `rep_name` | Representative name | text | no | corroborated | Conditional-required logic TBD (see below) |
| `rep_phone` | Representative phone | tel | no | corroborated | Conditional-required logic TBD |
| `same_as_above` | Same address as above | checkbox | no | corroborated | Paper wording corroborated |
| `return_address` | Return mailing address | address_block | no (TBD) | corroborated | Required-iff-not-same is TBD; conservative `false` + help text |
| `countries_of_use` | Country (or countries) of use | textarea | yes (TBD) | corroborated | Paper wording corroborated (Section 4) |
| `number_of_documents` | Number of documents | number | yes (TBD) | corroborated | 15-document walk-in cap seen in one copy — TBD, NOT encoded |
| `document_type` | Document type | text | no (TBD) | corroborated | Paper may use checkboxes; options TBD, free text meanwhile |

All `required` flags are TBD: the pilot has no submission path,
so they drive inline guidance only. Re-verify each flag against
the official PDF before any submission wiring.

## TBD (not in v1.json, not on the route)

- Section 5 cost computation and any fee figure (also blocked
  by decision 10).
- Document-type checkbox options (if the paper uses them).
- Conditional requirements (representative fields when
  `on_behalf = yes`; return address when `same_as_above` is
  unchecked).
- Exact paper labels, help microcopy, and field order within
  sections.
- `PdfRect` coordinates: `v1.json` carries stacked
  placeholders on page 1 (US Letter). Measure from an official
  scan before printpdf rendering (open question OQ-F1).
- Submission semantics: mail/walk-in/courier acceptance, SLA,
  replica acceptance — nothing asserted, nothing wired.

## Resemblance notes

- Route: `/(customer)/forms/ds-4194`. Fill + review + US-Letter
  print CSS; replica-unconfirmed banner (template
  `accepts_replica: unknown`); no POST anywhere.
- Synthetic fixtures only in tests and demos (e.g. Ann Lee,
  1 Main St, Olympia WA).
