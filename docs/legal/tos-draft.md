# DRAFT TERMS OF SERVICE — Andrii Co. d/b/a Apostille Handbook

> DRAFT — NOT LEGAL ADVICE — REQUIRES HUMAN LAWYER REVIEW.
> Do not publish. Reviewer: Washington attorney (EN + ES pass).
> Source: researcher draft (prompt 010), recovered verbatim from
> child session log. Date: 2026-10-26.

*Business: Andrii Co., Bothell, Washington. Effective date: [TBD].
Jurisdiction placeholder: Washington law.*

## 1. What this service is

1.1 **Informational handbook.** Country/authority pages (addresses,
fees, forms, processing estimates, walk-in reports) for apostille,
authentication, and consular legalization, starting with 106
authorities in the U.S./Canada/Mexico. Handbook content is general
information only, not legal advice (see §7).

1.2 **Paid services.** (a) **Free self-service:** guided in-browser
form fill, prefilled addresses/fees, printable checklist. No
fulfillment. (b) **Paid labels:** (a) plus ready-to-use outbound +
return FedEx/USPS labels (PDF/tracking). (c) **Envelopes-to-door
kit:** (b) plus printed forms, checklist, envelopes, FedEx delivery
to customer. (d) **Full-service + pickup (Washington only):** (c)
plus mobile-notary doorstep pickup, document pre-check,
notarization if needed, mail/walk-in submission (e.g., WA Secretary
of State, Tumwater), tracking to return. Tiers (c)–(d) logistics,
SLAs, and geography are as quoted at checkout; P2 order path is
in-browser fill + server-rendered PDF + labels only.

1.3 **What we are not.** We are not a government office, not the WA
Secretary of State, not the U.S. Department of State, not a
consulate, not a law firm, and (except for WA-commissioned notarial
acts in tier (d)) not acting as your attorney or agent before any
authority unless a separate writing says so.

## 2. Accounts and authentication

2.1 Orders and uploads require an Andrii Co. account. Login methods
offered: passkey, Sign in with Apple/Google/Meta, magic-link
fallback; existing booking auth may be reused. You must keep
devices and email accounts secure and promptly report suspected
compromise.

2.2 Sessions use opaque session cookies (`__Host-` prefixed,
HttpOnly/Secure/SameSite) plus CSRF/Origin checks. We may
step-up, suspend, or require re-verification for risk,
account-linking, or revocation signals. One account may link
multiple verified identities; linking requires verified-email
match and explicit confirmation — no silent merges. [LAWYER:
confirm age/capacity clause needed; add business-account
authority rep for J4 corporate packets.]

## 3. Your submissions and uploads; instruction-photo grant; redaction consent

3.1 **What you submit.** Form values, addresses, parcel data,
document scans/photos, route/instruction photos and videos of
offices and counters, comments/corrections, and support messages.
You represent you own or have the right to share everything you
upload, that IDs and source documents are authentic and unaltered,
and that you will not upload another person's documents without
authority.

3.2 **Ownership grant for instruction photos/videos.** By uploading
any route, office, parking, signage, or walk-in instruction photo
or video ("Instruction Media"), you assign all right, title, and
interest in that Instruction Media to Andrii Co., including the
right to keep, copy, crop, annotate, redact, publish, and
sublicense redacted derivatives in the handbook and related
guides, without compensation or further approval. This grant does
not cover your identity documents or the private documents you ask
us to apostille; those remain yours, with only the limited license
in §3.4. [LAWYER: confirm assignment vs. perpetual license;
consider moral-rights waiver and minor-in-photo rule.]

3.3 **Face/plate redaction consent.** You consent to automated
detection and masking of faces and license plates in Instruction
Media (currently planned: SAM 3.1 via Meta Model API) and to human
review before publication. Unredacted originals are held in a
staff-only PII vault with audited access; only redacted
derivatives publish. You may also redact before uploading. Masking
is best-effort, not guaranteed to catch every face/plate. Do not
upload media where incidental capture creates risk you cannot
accept.

3.4 **Limited license for service materials.** For non-Instruction
Media (forms data, document pre-check photos, scans, signatures
you draw), you grant Andrii Co. a non-exclusive, revocable (by
deletion where feasible, see Privacy Policy), service-limited
license to store, process, render PDFs/labels, transmit to
carriers and authorities you designate, and retain per our
retention schedule, solely to provide the service.

3.5 **Drawn signatures; wet ink default.** Default signature mode
is wet-ink (you print and sign in ink). Where a template permits
canvas-drawn signatures, you represent the drawn mark is yours and
agree it is embedded only in that submission's PDF. We make no
claim any authority accepts drawn/e-signatures; `accepts_replica` /
drawn-signature flags are stated per template with citations or as
"unknown."

## 4. Fees, payments, postage, refunds — [AUTHOR: fill numbers before launch]

4.1 **Handbook tier (a)** is free. Tiers (b)–(d), postage, carrier
fees, government fees (SOS/DOS/consulate/county), notary and travel
fees, and expedite charges are as quoted at checkout or in the tier
table. Government fees are pass-through and non-refundable once
remitted. [AUTHOR DECISIONS required: B1–B3/C1–C4/D1–D5 in
user-journeys.md — flat vs. pass-through postage, return-label
inclusion, reprint/re-route, kit bands, FedEx service promise and
miss remedy, "tomorrow" geography, pickup radius/SLA, notary
bundling, walk-in scope, $1.99 estate offer boundaries/cap.]

4.2 **Labels.** Label purchase may be staff-manual from a server
payload during P2 (tracking + PDFs pasted back; customer
notified). Postage recorded as `postage_cost_cents`/`paid_by`;
charging runs through existing Andrii Co. booking/billing until a
P2 checkout ships. Voids/reissues are staff-only and audit-logged.

4.3 **Refunds.** [LAWYER TO DRAFT with author numbers: e.g.,
unfulfilled label/kit before carrier purchase = refund;
government/carrier charges after purchase = no refund; our SLA
miss remedy = service-fee refund or expedite at our cost;
wrong-address caused by our stale prefill vs. customer override —
different treatment.] Nothing here promises a government outcome
or date except an explicit tier-(d) SLA in writing.

## 5. Prohibited uses

No false, forged, or altered documents; no impersonation; no
submission on behalf of another without authority; no
scraping/crawling that ignores robots.txt or rate limits; no
probing WS topics, IDs, or admin routes; no uploading malware or
unlawful content; no misuse of labels/tracking; no resubmission of
rejected spoof domains or quarantined content to evade trust
review. We may refuse, quarantine, or report suspected fraud.

## 6. Prefill, conflicts, and corrections

Prefills trace to accepted facts + cited captures. If the authority
revised its form, or two accepted facts conflict (e.g., two PO
Boxes), we block, withhold, or flag the field and tell you why;
manual overrides are marked `customer_override`. Corrections become
community captures and staff-reviewed tasks; community input never
auto-publishes. Stale-template refusals and replica notices are
part of the service, not errors.

## 7. Accuracy disclaimers; NOT legal advice — [NEEDS WA ATTORNEY EN+ES PASS]

7.1 Handbook pages, form guides, checklists, the Viet Nam
explainer, the Egypt-chain guide, and POA/corporate pointers are
**general information, not legal advice and not a lawyer-client
relationship**. Forms stay in the authority's language; Spanish
guides explain but do not replace the English text you sign.
Machine-translated pages are labeled with date and review status;
legal-adjacent Spanish content requires human review before
reliance.

7.2 Government addresses, fees, editions, hours, replica
acceptance, and processing times change without notice. We cite
sources and "last verified" stamps but do not warrant currency,
completeness, or that any authority will accept any replica, copy,
translation, or notarization. Verify critical facts against the
cited official source before mailing or traveling.

## 8. Liability limits; indemnity — [LAWYER TO DRAFT FOR WA]

[Placeholder: to the fullest extent permitted by Washington law,
disclaim implied warranties; cap direct damages at fees paid for
the order at issue; exclude consequential/special/incidental
damages (missed flights, appointments, closings, expired FBI
checks); no cap on your fraud or IP grant breach; mutual/single
indemnity TBD. Confirm enforceability of consumer-protection
waiver limits and conspicuousness.]

## 9. Termination and suspension

We may suspend or close accounts or refuse orders for fraud,
abuse, chargeback risk, or violation of these terms, with refunds
only as §4 requires. You may stop using the service and request
deletion per the Privacy Policy; government filings already mailed
and published redacted derivatives of assigned Instruction Media
are not recalled.

## 10. Governing law; venue; changes

Washington law governs, without regard to conflicts rules. Venue:
[LAWYER: pick state/court and county; add jury-waiver/arbitration
decision and small-claims carve-out]. We will post amended terms
with an effective date; continued use after that date accepts
them. English controls; Spanish is a convenience translation except
where a reviewed Spanish disclaimer states otherwise. Contact:
Andrii Co., Bothell, WA — [address/email/phone TBD].

*End of DRAFT Terms — DRAFT — NOT LEGAL ADVICE — REQUIRES
ATTORNEY REVIEW.*
