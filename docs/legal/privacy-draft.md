# DRAFT PRIVACY POLICY — Andrii Co. Apostille Handbook

> DRAFT — NOT LEGAL ADVICE — REQUIRES HUMAN LAWYER REVIEW.
> Do not publish. Reviewer: Washington attorney (EN + ES pass).
> Source: researcher draft (prompt 010), recovered verbatim from
> child session log. Date: 2026-10-26.

*Andrii Co., Bothell, Washington. Effective date: [TBD].
Placeholder law: Washington / applicable U.S. law.*

## 1. Scope

Covers handbook browsing, accounts, form/label/kit/full-service
orders, uploads (including Instruction Media and document pre-check
photos), walk-in reports, support (including Spanish phone
interpreter line), and validation-call transcripts. Handbook
evidence store holds code/schema only; customer materials live in
the content/store layer, not the code repo. [LAWYER: add WA My
Health My Data / CCPA / COPPA applicability analysis; this draft
assumes no health-data or under-13 targeting — confirm.]

## 2. Data we collect

(a) **Account/identity:** name, email, phone, login identifiers
(passkey credential IDs; Apple/Google/Meta subject IDs;
magic-link tokens), linked-identity records, role, audit events
(login/logout/link/unlink). (b) **Order/submission data:**
authority, destination use, form values, signature mode,
typed/drawn signature PNG where enabled, rendered PDF + sha256,
from/to addresses, parcel weight/dims, carrier/service, postage
cost, tracking numbers, label payloads. (c) **Uploads/media:**
document scans/photos, Instruction photos/videos,
comments/corrections, support attachments; EXIF preserved in
originals, stripped in derivatives. (d) **Transcripts/audio:**
validation-call audio (our staff calls to authorities,
consent-first) and transcripts + human annotations; support-call
notes where recorded. (e) **Payments:** processed by existing
booking/billing provider(s); we store order totals, `paid_by`, and
provider references, not full card/bank numbers. [AUTHOR: name
processor(s).] (f) **Logs/telemetry:** structured JSON logs
(actor/action/entity), capture IDs/hashes/offsets, IP, user-agent,
rate-limit and denied-subscription events. We do not log
session/CSRF tokens, cookies, Authorization headers,
document-photo bytes, full email bodies, or verbatim transcripts
in logs.

## 3. Purposes and legal bases — [LAWYER: conform to chosen framework]

Provide handbook + fulfillment (render forms/PDFs, buy/paste
labels, ship kits, pick up/file/track in WA); verify facts and
reconcile conflicts; pre-check document photos; transcribe and
annotate validation calls; mask faces/plates before publication;
prevent fraud, spoof domains, and abuse; operate auth, rate
limits, audit, and retention sweeps; communicate order/tracking/
support messages (EN/ES). We do not train ML models on your data.
[Add consent/contract/legitimate-interest mapping if required.]

## 4. Retention — [DECISIONS REQUIRED]

Voice audio: **365 days** from capture, then deleted by
`retention-sweep` (blob + row removed, session link nulled, audit
row written). Transcripts, transcript annotations, and consent
records: kept indefinitely as evidence. Photo/PII retention:
**TBD — AUTHOR DECISION** (proposal in spec: submitted orders 2
years, drafts 90 days, drawn-signature PNGs 1 year; customer
route/document photos TBD; label PDFs/tracking TBD).
Logs/audit: [TBD — propose audit append-only, retained [N] years;
security logs [M] days]. Until decided, do not promise any
photo/PII period publicly.

## 5. Sharing and processors

(a) **Meta Model API (Meta Models API)** — approved processor for
transcription and face/plate redaction (SAM 3.1 path);
media/transcripts sent only as needed for those jobs. [LAWYER:
review Meta data-handling/retention/subprocessor terms before real
media flows; add DPA reference.] (b) **Carriers** (FedEx/USPS +
staff-manual portal buys): names, addresses, phones, parcel data,
service codes needed to issue and track labels. (c)
**Payment/booking provider(s):** order/billing data per §2(e).
(d) **Authorities/consulates you designate:** only the mailed/
emailed filing contents. (e) **No other third party receives
customer media.** No sale of personal information; no
cross-context behavioral advertising. Legal-compliance disclosures
only as required by law with narrowing where permitted.

## 6. Your rights; deletion; correction

Request access, correction, export, or deletion of your
account/order data via [contact TBD]. Deletion removes PII from
service stores per §4 and revokes the §3.4 license prospectively,
but cannot recall mailed filings, carrier records, government
records, append-only audit rows, retained transcripts where you
were the called authority representative (consent record retained),
or assigned Instruction Media already published as redacted
derivatives (unredacted originals in vault are deleted/purged per
retention once decided). Correction reports feed the handbook task
queue; promotion requires corroboration or staff approval.
[LAWYER: add WA/CA request timelines, appeal, and authorized-agent
handling.]

## 7. Cookies, sessions, and security

Opaque 256-bit session cookies (`__Host-hb_pub` customer /
`__Host-hb_adm` admin where applicable; Secure/HttpOnly/
SameSite=Lax, Path=/), CSRF double-submit token + Origin checks,
WS upgrade auth + per-topic authorization. Customer idle timeout
~24h; admin shorter + mTLS on admin host. Security program per
SECURITY.md baseline: allowlisted uploads with magic-byte checks
and size caps (photo ≤25 MB, video ≤500 MB), content-addressed
storage outside web root, quarantine-until-promoted community
trust, no PII in code repo, structured allowlisted logging,
per-IP + per-account rate limits, robots/SSRF-gated fetching. No
method is 100% secure; report incidents to [security contact TBD].

## 8. Children; international; changes; contact

Service is for adults ordering their own or authorized
business/family filings; do not submit children's data except a
parent/guardian's required filing (e.g., J1 birth certificate) and
only the minimum needed. Data is processed in the U.S.;
cross-border label/translation coordination (Canada/Mexico)
transmits only addressing/shipment data the destination requires.
We will post changes with a new effective date; material changes
get prominent notice. Controller: Andrii Co., Bothell, WA —
mailing address, email, and phone: [TBD]. Privacy/security
requests: [TBD].

*End of DRAFT Privacy Policy — DRAFT — NOT LEGAL ADVICE —
REQUIRES ATTORNEY REVIEW.*
