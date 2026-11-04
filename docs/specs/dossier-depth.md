# Dossier depth: what it takes to publish a New York-grade page

Status: plan, 2026-11-03. Source: a read-only map of the pipeline
against the operator's target New York page (who issues, document
rules, three certification chains, the form, five walk-in offices,
fee, payee, USPS vs courier addresses, timing, verify link, remote
online notarization). Nothing here is built yet. Official URLs were
fetched or searched on 2026-11-03.

## Short answer

Most of the pipeline exists. Packs seed an authority and its watched
official pages. Fetch, sweep or staff upload turns them into captures.
Muse Spark drafts cited facts in 26 frozen topics, Muse Wonder cross-
checks them, and staff accept them in /review. Spark then composes
offices (OfficeSetV1), form maps (FormMapV1) and a page plan
(RenderPlanV1), and staff accept each.

Four gaps keep the app from producing the New York page:

1. **Vocabulary.** `fact.topic` is fixed by a DB CHECK, so a new topic
   is a migration (forbidden). New concepts (HCCH signer, verify URL,
   remote notarization, per-document rules, skip conditions) become
   *field keys* under existing topics. The live drafter still runs
   aa-prompt-v1, which has no field list.
2. **Page plan.** Only 7 block kinds, no links (URLs are stripped), at
   most 9 blocks in the prompt (12 in the validator) and 5 paths. It
   cannot express an issuer card, a per-document matrix, three chains
   with skip conditions, a verify link or a RON card.
3. **Offices.** OfficeSetV1 has no same-day, document-limit, drop-off,
   courier-address, carrier allow/deny, hours-label or line-2 fields.
4. **Chains.** ChainView renders every `legalization_chain` fact as one
   flat list. Compile (discover → capture → draft → check → compose) is
   a stub, so each step is a manual admin call.

All four are additive code: no schema migration.

## Gap table

| Target section | Covered today by | Change |
|---|---|---|
| Who issues (office, official URL, HCCH signer) | authority name; `website`, `jurisdiction` facts | fields `website:official_url`, `jurisdiction:competent_authority`, `jurisdiction:hcch_signer`; HCCH details3 page as a source; plan kind `authority` with a `link_fact_id`; AuthorityCard |
| Which documents it accepts | plan `paths` (max 5) + step/callout blocks | fields `jurisdiction:doc_rule`, `rejections:not_accepted`, `rejections:route_elsewhere`; paths cap 8; kind `documents`; DocumentsMatrix |
| Certifications first (3 chains, skip-if, scope) | flat ChainView | `chain_record` table (CREATE TABLE IF NOT EXISTS) + Spark ChainSetV1 with a guard (actor, county, scope words and numbers written in the cited text); fields `legalization_chain:hop`, `:skip_if`, `:scope`; kind `chain`; ChainSteps; WS `chain_accepted` on `authority:{slug}` |
| The request form (DOS-1917-f) | forms topic + FormMapV1 | PDF text layer for fetched PDFs (sweep.rs routes `%PDF` through lopdf) or the staff upload UI; the form-map guard must drop card, CVV and signature fields from "Fill it online" |
| Requesting online (none) | nothing | v3 prompt rule to extract stated negatives ("by mail and walk-in"); field `forms:online`; callout |
| Where to go in person (5 offices) | OfficeSetV1 (address, walk-in, split hours) | OfficeSetV1 v2: `line2`, hours `label`, `services {same_day, doc_limit, drop_off}`, phone from a cited contacts fact, `mon–fri` with an en dash; timezone stays staff-confirmed (no NY page says "Eastern") |
| What it costs ($10) | fees block | none; fields `fees:per_document`, `fees:county_clerk` |
| How to pay (payee, no cash) | payment facts as step text | fields `payment:payee`, `:cash`, `:card_authorization`; FeeSummary shows the payee (the payee is only in the DOS-1917-f PDF, so that capture is needed) |
| Where to send (USPS PO Box vs courier street) | one `mail` address, `usps_only` | OfficeSetV1 `courier` address + `carriers {accepted, refused}` + return rules; MailBlock tabs; Egypt's "USPS only" reuses `carriers.refused` |
| How long it takes | timing block | fields `processing_time:standard`, `:walk_in`, `:drop_off`; "no commitment" is an absence and is never published as a fact |
| Verify an apostille | nothing (no URLs in plans) | fields `website:verify_url`, `contacts:status_line`; kind `verify` with `link_fact_id` (https host in the domain table); VerifyBlock |
| Remote online notarization | `notarization:step` only | fields `notarization:remote_status`, `:legal_basis`, `:in_force`, `:registration`, `:id_proofing`, `:fee_cap`; kind `ron`; RonCard; dates copied as written ("February 1, 2023") so number grounding holds |

New plan kinds and the topics they may cite: `authority` → website,
jurisdiction, authentication; `documents` → jurisdiction, rejections,
notarization, authentication, id_requirements; `chain` →
legalization_chain, notarization, authentication, jurisdiction;
`verify` → website, contacts; `ron` → notarization. `parsePlan` already
drops unknown kinds, so older pages degrade safely.

## Build order (each slice gated as usual: eval delta for AI changes)

1. **aa-prompt-v3**: v2 plus the field keys above and the stated-
   negatives rule; topic-group passes (office, money/time,
   prepare/mail, routing) or 16 records per capture. Switch the live
   drafter only after `xtask eval` on the gold sets plus a new
   `evals/authorities/usa-ny-dos.json` (public dos.ny.gov quotes) shows
   no regression.
2. **PDF text for fetched captures** and an "Upload official document"
   control on the trace screen.
3. **OfficeSetV1 v2** (offices.rs schema, guards, OfficeCard, MailBlock).
4. **Chains**: chain.rs, ChainSetV1, admin compose/accept, ChainSteps.
5. **RenderPlanV1 v2**: the five kinds, link_fact_id, caps 8 paths and
   18 blocks, plan-v2 prompt, renderer components.
6. **Compile run** (optional, cuttable): automate the loop over
   `run:{id}`.

## Per-authority checklist (data, no code)

1. Pack: domains + watch URLs (lists below), `cargo xtask packs-validate`.
2. Staging: `packs-load`, `fetch-slug --slug <slug>`; upload PDFs and
   bot-blocked pages with their official URL.
3. Draft each capture, run the cross-check, accept facts in /review.
4. Compose and accept offices (confirm the timezone), form maps,
   chains, then the plan (the dossier updates live over WS).
5. Production: the operator runs the same data steps; facts reach
   production only through the operator's /review approvals.

Slugs: use `usa-ny-dos`, `usa-or-sos`, `usa-ca-sos`, `usa-nv-sos` (the
`packs/usa-*.json` convention). `packs/world/US.json` carries the
curated slug for every curated state (and `usdos-authentications`), so
loading curated packs first and then the world set merges its rows
into the curated dossiers (P397; pinned by
`world_rows_merge_into_curated_dossiers`). A new curated state pack
must rename its `us-*` row in `packs/world/US.json` the same way. With
P362 each of these also answers at `usa/<state>` and `us/<code>`.

## Official sources (government domains; fetched 2026-11-03)

**New York (usa-ny-dos)**
- https://dos.ny.gov/apostille-or-certificate-authentication (hub)
- https://dos.ny.gov/obtain-apostillecertificate-authentication (steps by document type, fee, PO Box, courier address)
- https://dos.ny.gov/walk-service (five offices, split hours, same-day, 10-document limit, drop-off)
- https://dos.ny.gov/verifying-apostillecertificate-authentication (links the verify tool; link it, do not crawl it)
- https://dos.ny.gov/additional-information-apostillecertificate-authentication
- https://dos.ny.gov/apostillecertificate-authentication-request-forms (DOS-1917-f, AcroForm, Rev. 02/24: payee, no cash, PO Box 22001, One Commerce Plaza)
- https://dos.ny.gov/notary-public-frequently-asked-questions (remote notarization, Executive Law §135-c)
- https://dos.ny.gov/public-notice-adoption-remote-notary
- https://www.nyc.gov/site/doh/services/birth-death-records-birth.page (Letter of Exemplification, New York County Clerk)
- https://ww2.nycourts.gov/courts/1jd/supctmanh/county_clerk_operations.shtml
- https://www.health.ny.gov/vital_records/
- https://www.criminaljustice.ny.gov/ojis/recordreview.htm
- https://www.hcch.net/en/states/authorities/details3/?aid=353 (signers; slow crawl)
- Drop from the pack: the businessexpress.ny.gov answer page (robots disallow).

**Washington (usa-wa-sos)**
- https://www.sos.wa.gov/corporations-charities/apostilles-services-information-resource-page (an alert posted 11/02/2026 says apostille services are unavailable)
- https://www.sos.wa.gov/corporations-charities/apostilles-program/apostille-frequently-asked-questions
- https://www.sos.wa.gov/sites/default/files/2025-06/6.2025%20-%20Apostilles%20request%20form.pdf (current form; the pack still watches a removed 2024 PDF)
- https://www.sos.wa.gov/corporations-charities/apostilles-program/apostille-and-certificate-authenticity-search
- https://www.sos.wa.gov/corporations-charities/frequently-asked-questions-faqs/fee-schedules-priority-services
- https://doh.wa.gov/licenses-permits-and-certificates/vital-records/apostilles
- https://dol.wa.gov/guides/washington-state-notary-public-guide/electronic-notarial-acts/how-notarize-electronic-documents (notaries are DOL, not SOS)
- sos.wa.gov blocks fetchers intermittently: keep staff upload ready.

**Oregon (usa-or-sos)**
- https://sos.oregon.gov/business/Pages/how-to-get-authentication-apostille.aspx
- https://sos.oregon.gov/business/Documents/notary-forms/notary-request-for-authentication.pdf
- https://sos.oregon.gov/business/pages/authenticating-documents-faq.aspx
- https://sos.oregon.gov/business/Pages/delivery-options.aspx
- https://www.oregon.gov/oha/PH/BIRTHDEATHCERTIFICATES/Pages/index.aspx
- https://sos.oregon.gov/business/Pages/remote-online-notarization.aspx

**California (usa-ca-sos)**
- https://www.sos.ca.gov/notary/request-apostille (USPS PO Box vs courier street, Los Angeles office)
- https://www.sos.ca.gov/notary/apostille-faqs
- https://www.sos.ca.gov/notary/processing-times
- https://www.sos.ca.gov/notary/forms and the cover sheets under https://notary.cdn.sos.ca.gov/forms/
- https://www.sos.ca.gov/business-programs/regional-office/
- The verify search is http only: link it, never cite it (citations need https).
- Remote online notarization is not yet operative in California (2025 notary handbook).

**Nevada (usa-nv-sos)**
- https://www.nvsos.gov/businesses/apostille and /fees return empty bodies to fetchers: use staff upload.
- The request form PDF on bizhub.nv.gov (revised 07/01/2026; address, phone, expedite tiers).
- https://www.nvsos.gov/sos/businesses/processing-dates
- https://www.nvsos.gov/licensing/notary/enotary-faqs

**Egypt (egypt and the consulate slugs)**
- Egypt is not a party to the Apostille Convention (no Egypt row in the HCCH status table, https://www.hcch.net/en/instruments/conventions/status-table/?cid=41, fetched 2026-11-03), so its page is a consular-legalization dossier, not an apostille page.
- Ministry of Foreign Affairs (Arabic): https://www.mfa.gov.eg/ar/ConsularServicesAndTransactions/Details/77 and https://www.mfa.gov.eg/ar/RatificationOffices/AuthenticationProcedure
- US-side chain: the embassy's legalization pages on egyptembassy.net (official embassy site; notary → state authentication → USDOS where required → consulate; USPS only).
