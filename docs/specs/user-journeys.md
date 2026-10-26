# Critical User Journeys (draft for review)

- Status: draft (prompt 007, for author review; no implementation yet)
- Date: 2026-10-26
- Source: parallel researcher draft (full text recovered from child
  session log).
- Tiers (c)/(d) (door-kits, full-service) live here as P3/ops — P2
  covers forms + labels only (see
  [p2-forms-labels.md](p2-forms-labels.md)).

**Business:** Andrii Co., mobile notary + apostille, Bothell WA.
**Scope:** 106 authorities in USA / Canada / Mexico. Handbook is
source of truth for addresses, forms, fees; service tiers fulfill
around it.
**Context from repo:** WA counter moved Olympia → Tumwater (June
2025); Viet Nam joined Apostille Convention 11 Sept 2026 (own site
misrouted to legalization as late as 13 Oct 2026); Washington's two PO
Boxes contradiction; Egypt-in-USA legalization chain is first
non-Hague chain; existing assets: online booking, MCP server, Gemini
Spanish phone interpreter +1(425)800-8688, $1.99 estate/inheritance
offer through June 2027.

**Four tiers referenced below:**

* **(a) Free self-service:** guided form-filling, label data
  (addresses), printable checklist. No fulfillment.
* **(b) Paid labels:** (a) + prepaid USPS/FedEx outbound + return
  labels (PDF/email).
* **(c) Envelopes to door:** (b) + printed forms, printed checklist,
  envelopes, FedEx delivery to customer's house (the hackathon
  success criterion).
* **(d) Full-service + pickup (WA only):** (c) + mobile notary pickup
  at door, document pre-check, notarization if needed, mail/walk-in
  submission, tracking to return.

---

## J1. US birth certificate for Mexican dual nationality

**Trigger:** US-born child of Mexican parent(s) needs apostilled birth
certificate for Registro Civil / SRE dual-nationality registration in
Mexico.

**Steps:** 1) Order certified (long-form, registrar-signed) birth
certificate from correct county/state vital records. 2) Identify
correct apostille authority (state of issuance, not state of residence
— e.g., WA-born → WA Secretary of State, Tumwater). 3) Complete state
apostille cover form, fee, mail to correct PO Box. 4) Receive
apostille, forward to family or Mexican consulate; arrange certified
Spanish translation if required by receiving office.

**Anxieties:** Is a photocopy or short-form abstract enough? (No.)
Which state apostilles it? Which Tumwater address / PO Box is
current? Will Mexico accept it without translation? How long will
vital-records + apostille take in series?

**Across tiers:**
* (a) Free: state selector ("where were you born?"), certified-copy
  ordering link, auto-filled WA cover form, Tumwater address + fee +
  current processing estimate, checklist distinguishing certified
  copy vs. photocopy, note on translation requirements in Mexico.
* (b) Labels: + outbound label to Tumwater + tracked return label;
  useful because vital-records offices and SOS both accept mail and
  customer already has printer.
* (c) Envelopes to door: + printed cover form, checklist, both
  envelopes via FedEx tomorrow. Best when customer has the certified
  copy in hand but no printer or fears addressing errors.
* (d) Full-service WA: + doorstep pickup of certified copy, visual
  pre-check (raised seal, registrar signature), submission, return
  tracking. Best when customer is in WA and the registration
  appointment in Mexico is already scheduled.

**Best-fit:** (a) for planners with printer; **(c) for most** (one
document, one destination, error cost is a 3-week re-mail); (d) when
date-driven.
**Honest upsell triggers:** (a)→(b): customer asks "what address do I
put on the return envelope?" (b)→(c): no printer, or "I already
mailed to the wrong PO Box once." (c)→(d): appointment in Mexico < 3
weeks away, or unsure the copy they hold is certifiable.

## J2. Diploma / transcripts for work abroad (incl. Viet Nam post-entry)

**Trigger:** Recent graduate (e.g., UW, Bothell) hired in Canada,
Mexico, or Viet Nam; employer/visa office demands apostilled diploma
and/or transcripts.

**Steps:** 1) Determine what the destination accepts: original
diploma vs. registrar-issued certified copy vs. notarized photocopy
(varies by state authority and destination). 2) Get notarization if
required (many states apostille only notarized copies of diplomas,
never originals directly). 3) Apostille in state of notarization. 4)
Ship abroad or to employer; translate if required.

**Anxieties:** "Do I mail my only original diploma?" (Usually no —
and losing it is catastrophic.) Does Viet Nam now take apostille or
still consular legalization? (Apostille since 11 Sept 2026 — handbook
must say so prominently with citation.) Will the employer accept a
notarized copy? Notary wording valid for the SOS?

**Across tiers:**
* (a) Free: destination check ("Viet Nam → apostille, since
  2026-09-11; do NOT go to the consulate"), state rule for diplomas
  (notarized copy vs. registrar original), notary-certificate wording
  template, cover form, checklist.
* (b) Labels: + labels to SOS; weak fit alone because notarization
  step precedes mailing.
* (c) Envelopes to door: + printed notary-certificate insert, cover
  form, envelopes. Customer still visits a notary themselves.
* (d) Full-service WA: mobile notary comes to the door, notarizes the
  copy correctly the first time, picks up, submits, tracks. Eliminates
  the #1 rejection cause (bad notarization).

**Best-fit:** **(d) in WA** (notarization + submission in one visit);
(a)+(local notary) outside WA, (c) if they want the paperwork kit.
**Honest upsell triggers:** (a)→(d): "my school only gave me one
diploma"; any mention of Viet Nam + consulate confusion; prior
rejection for notarization. (c)→(d): customer has no notary
relationship or no transport.

## J3. FBI background check / federal document for emigration or work (USA → Canada/Mexico)

**Trigger:** US citizen needs apostilled FBI Identity History Summary
(or federal document: naturalization certificate, USPTO, FDA) for
Canadian/Mexican immigration, work permit, or residency.

**Steps:** 1) Obtain correct federal document (FBI check via approved
channeler or direct; must be the official report). 2) Federal
apostille via **US Department of State, Authentications Office** —
NOT the state SOS (most common misroute). 3) DS-4194 form, fee, mail
per current DOS instructions. 4) Forward abroad; note FBI checks
often have 3–6 month freshness windows at destination.

**Anxieties:** State SOS vs. State Department — nearly everyone
guesses wrong first. DOS processing time vs. visa freshness window.
DS-4194 version currency. Whether a channeler printout is
apostillable.

**Across tiers:**
* (a) Free: document-type router ("FBI check → US Dept. of State,
  never WA SOS"), DS-4194 pre-fill, current DOS address/fee/
  processing estimate, freshness-window warning with back-planning
  calculator.
* (b) Labels: + DOS-compliant outbound/return labels; high value
  because DOS mail rules are strict and change.
* (c) Envelopes to door: + printed DS-4194, envelopes, checklist.
  Good for customers who distrust their printer for a federal form.
* (d) Full-service WA: + pickup, pre-check (is this the apostillable
  original?), DOS submission, expiry back-planning ("your Mexican
  consulate appointment must be before X").

**Best-fit:** **(b) or (c)** — the submission is federal-by-mail
regardless, so pickup adds less; (d) when freshness window is tight
or customer is unsure which paper they hold.
**Honest upsell triggers:** (a)→(b): any DOS mail-rule question.
(b)→(c): no printer. Any tier→(d): FBI check older than 60 days with
a visa deadline pending.

## J4. Corporate packet for expansion (WA LLC → Mexico operations)

**Trigger:** Small business opening a branch, signing a lease, or
granting power of attorney in Mexico needs apostilled certificate of
existence/good standing, articles, board resolution, and POA.

**Steps:** 1) Pull fresh certificate of existence from WA SOS
Corporations. 2) Draft/execute POA and resolutions; notarize
signatories. 3) Apostille each document (state of
notarization/issuance). 4) Certified Spanish translation in many
Mexican states; deliver to notario público in Mexico.

**Anxieties:** Which docs need fresh dates (certificates stale
quickly)? Does the Mexican notario want translation by a perito
traductor? Multi-document fee math and single vs. separate cover
forms. Signatory authority proof.

**Across tiers:**
* (a) Free: packet builder (per-document checklist, per-document fee
  tally, POA template pointers — not legal drafting), cover forms,
  translation-scope note.
* (b) Labels: + tracked labels sized for a multi-document packet with
  return tracking per document.
* (c) Envelopes to door: + printed cover forms per document, document
  separator sheets, envelopes. Reduces mix-ups in 4–6 document
  packets.
* (d) Full-service WA: + on-site notarization of signatories, packet
  assembly, submission, translation coordination, courier to Mexico
  contact.

**Best-fit:** **(d)** — multi-document + notarization + translation
coordination exceeds self-service reliability; (a) only for repeat
filers with counsel.
**Honest upsell triggers:** (a)→(c): more than 2 documents. Any→(d):
signatories in different locations, Mexican notario deadline, or "our
lawyer said get it apostilled but didn't say how."

## J5. Viet Nam case post-Convention-entry (the misroute regression test)

**Trigger:** Any US/Canada/Mexico document needed in Viet Nam after
11 Sept 2026 — family, marriage, study, work. Customer (and many
blogs, and previously our own site) still believes consular
legalization is required.

**Steps:** 1) Handbook asserts apostille route with effective date +
HCCH citation. 2) Standard per-document flow (J1/J2/J3 depending on
document). 3) Explicit "do NOT visit the Vietnamese consulate for
legalization" stop sign. 4) Monitor for Vietnamese receiving-office
stragglers that still ask for consular stamps during transition.

**Anxieties:** "Are you sure? Three other sites say consulate." Fear
that the apostille will be refused on arrival. Transition-period
inconsistency at receiving offices.

**Across tiers:**
* (a) Free: destination banner with effective date + citation + "last
  verified" timestamp; printable one-page explainer (EN/ES summary
  pointer; Vietnamese translation is a recorded exception to the
  EN/ES/FR scope, pending decision) the customer can show a
  skeptical receiving clerk.
* (b) Labels: same as underlying document journey; the value-add is
  confidence, not logistics.
* (c) Envelopes to door: + printed explainer sheet included in the
  kit.
* (d) Full-service WA: + human reassurance at pickup ("yes, apostille
  is correct as of [date], here is the citation"), plus
  walk-in-report feedback loop if a receiving office pushes back.

**Best-fit:** **(a) for the information job** (this journey is
primarily a trust/content job); fulfillment tier follows the
underlying document type.
**Honest upsell triggers:** Fulfillment upsell only per underlying
journey; never upsell on fear. The correct "upsell" here is the
walk-in/pushback report button ("was your apostille refused in Viet
Nam? Tell us") feeding handbook corrections.

## J6. Non-Hague destination: US document for Egypt (consular legalization chain)

**Trigger:** US document (e.g., commercial invoice, POA, degree)
needed in Egypt, a non-Hague country: full chain — notarization →
county/state certification → US State Department authentication
(conditional — mandatory for commercial/court/marriage instruments
per [egypt-consular.md](egypt-consular.md) V-6) → Egyptian consular
legalization.

**Steps (first chain: Egypt in USA):** 1) Notarize. 2) County clerk
certification if required by state. 3) State SOS certification (not
apostille — same office, different certificate). 4) DOS
authentication where required (see V-6). 5) Egyptian
consulate/embassy legalization (jurisdiction by **issuing state**,
not residence; fees/payment method vary). Each
step's output is the next step's input; one error restarts the chain.

**Anxieties:** Chain order, consulate jurisdiction ("which Egyptian
consulate takes a WA-issued document?"), payment method (money
order vs. card), total elapsed time (weeks), document aging
between steps.

**Across tiers:**
* (a) Free: chain visualizer with per-step forms, fees, addresses,
  jurisdiction map, back-planner from need-by date; pre-filled
  authority forms per step.
* (b) Labels: + per-hop labels; genuinely complex (up to 4 mail
  hops) — label kit must be hop-labeled (Step 2→3, 3→4…) to prevent
  mis-sends.
* (c) Envelopes to door: + hop-labeled printed forms + envelopes in
  chain order. High value: the chain is a sorting problem as much as
  a mailing problem.
* (d) Full-service WA: + pickup, chain management end-to-end,
  per-hop tracking, consulate submission. Only tier where Andrii Co.
  absorbs chain risk.

**Best-fit:** **(c) minimum; (d) recommended.** (a)/(b) serve only
experienced legalization repeaters.
**Honest upsell triggers:** (a)→(c): any question about hop order or
"which consulate?" (c)→(d): need-by date < 6 weeks, commercial
shipment blocked, or first-timer with 3+ documents.

## J7. Urgent travel deadline (death, marriage, custody — flight in 7–14 days)

**Trigger:** Family emergency or fixed-date event abroad
(funeral/inheritance in Mexico, wedding, custody hearing in Canada).
Documents must be apostilled and in hand before a flight.

**Steps:** 1) Triage: what must travel with the customer vs. what can
follow by courier. 2) Fastest path per document: walk-in (Tumwater
counter? Sacramento? county registrar?) vs. expedited mail vs. DOS
expedite. 3) Same-day notarization if needed. 4) Tracked return timed
to flight; backup plan (travel with certified copies + apostilles
ship separately).

**Anxieties:** Missing the flight or the funeral. Paying for speed
that doesn't materialize (stale walk-in hours, closed counters,
parking/access confusion). Note existing $1.99 estate/inheritance
offer (through June 2027 or $50k cap) — grieving customers should
see it, not be upsold past it.

**Across tiers:**
* (a) Free: deadline back-planner ("flight on X → last safe mail
  date Y, walk-in cutoff Z"), walk-in intel (Tumwater route photos,
  parking, counter hours, rejection notes), triage checklist.
* (b) Labels: + overnight/express labels both ways; only if the math
  closes before the flight.
* (c) Envelopes to door: FedEx-tomorrow kit; fits when documents are
  ready and the bottleneck is correct packaging, not procurement.
* (d) Full-service WA: + same/next-day pickup, mobile notarization,
  walk-in submission where available, hand/overnight return. Only
  tier that can credibly promise a date.

**Best-fit:** **(d) in WA when the flight is < 14 days out**;
(a)+walk-in intel for out-of-WA customers near their own SOS counter;
(c) when docs are ready and flight is 14+ days out.
**Honest upsell triggers:** Any tier→(d): back-planner shows mail
path misses the flight; grieving customer (offer $1.99 path first,
upsell only on speed, never on grief); walk-in required but customer
can't travel to Tumwater.

## J8. Spanish-speaking customer (ES-first, phone-centric)

**Trigger:** Same needs as J1–J7, but customer reads Spanish better
than English, may distrust English-only government sites, and
currently reaches Andrii Co. via the Gemini Spanish phone
interpreter.

**Steps:** Mirror the underlying journey, with: ES-first handbook
content for that authority, ES form-filling guidance (forms themselves
remain English — customer must know what they sign), ES checklist, ES
phone/chat support, ES tracking updates.

**Anxieties:** Signing an English form they don't fully understand.
Whether the translation they arrange in the US will be accepted in
Mexico. Phone-booking reliability. Fear of scams targeting Spanish
speakers (fake "apostilla" services).

**Across tiers:**
* (a) Free: ES UI + ES authority pages + ES field-by-field form guide
  with EN field names shown alongside; anti-scam notice (what Andrii
  Co. will never ask for).
* (b) Labels: + ES-labeled mailing instructions ("sobre de ida /
  sobre de regreso"), ES tracking links.
* (c) Envelopes to door: + ES printed checklist on top, EN forms
  beneath with ES field guides; reduces the "which paper goes where"
  failure mode.
* (d) Full-service WA: + Spanish-capable pickup visit (human or
  interpreter-assisted), in-person explanation before signing.

**Best-fit:** **(c) or (d)** — paper-in-hand with ES guidance beats
screen-only for this segment; (a) must still be fully usable, never a
degraded funnel into paid tiers.
**Honest upsell triggers:** (a)→(c): customer on phone says "no tengo
impresora." (c)→(d): multi-document packet or notarization needed and
customer prefers explaining in person. Never upsell on language need
alone.

## J9. Mobile-only customer (no printer, no desktop)

**Trigger:** Same needs as J1–J7; customer has only a phone, no
printer, possibly no stable mailing address (young, renter,
traveler).

**Steps:** Phone-complete flow: photo-based document pre-check
(Glimmer), mobile form fill, Apple Pay / passkey checkout (existing
patterns), kit or pickup arranged without ever touching a desktop.

**Anxieties:** "Can I really do this from my phone?" Photo quality
sufficiency. Where the kit ships if they're between addresses. Data
use and upload reliability.

**Across tiers:**
* (a) Free: mobile-first forms, photo pre-check with clear
  pass/fail/retake, checklist as shareable link (not just printable
  PDF), label data as QR + copy-paste (post-office counter
  scanning).
* (b) Labels: + carrier QR codes / post-office scan flows, not just
  PDFs — a PDF label is useless without a printer.
* (c) Envelopes to door: the natural tier — everything physical
  arrives ready; ships to home, work, or FedEx hold-for-pickup.
* (d) Full-service WA: + pickup at their location; highest
  convenience, WA-bounded.

**Best-fit:** **(c)** as default (printerlessness is the defining
constraint); (a)+QR labels when time is ample and a post office is
near.
**Honest upsell triggers:** (a)→(b): customer reaches the "print
your label" step on a phone (offer QR/scan alternative first, paid
label second). (b)→(c): no post-office access, unstable address
(offer FedEx hold), or multi-document confusion.

---

## Placeholder pricing questions (author decisions, not proposals)

Tier (a) is free by definition. Everything below needs an author
number before any page promises it:

**Tier (b) labels:**
* B1. Flat per-order label fee vs. pass-through postage + service
  fee? One price for USPS and FedEx, or carrier-specific?
* B2. Does (b) include the return label always, or is return
  optional? (Recommend always — missing return postage is a top
  rejection/delay cause.)
* B3. Reprint/re-route policy: free reprint window, who pays
  carrier-change difference?

**Tier (c) envelopes-to-door:**
* C1. Kit price bands: single-document vs. multi-document packet
  (J4/J6)? Per-page print cap?
* C2. FedEx service level promise: overnight by default (matches
  success criterion) or economy default with overnight upsell? Who
  pays if FedEx misses?
* C3. Geographic limits for "tomorrow" promise (contiguous US only?
  HI/AK/Canada/Mexico timelines stated separately?).
* C4. What exactly ships: printed forms + envelopes + checklist only,
  or also prepaid return packaging (padded mailer, document sleeve)?

**Tier (d) full-service WA:**
* D1. Pickup radius from Bothell (miles / named cities) and fee
  inside vs. outside radius; same-day vs. scheduled-day pricing.
* D2. SLA: pickup window promise, submission promise (same-day
  walk-in vs. next-business-day mail), return-handling promise;
  remedy if Andrii Co. misses (refund of service fee? expedite at our
  cost?).
* D3. Notarization bundled or separate (WA statutory notary fee +
  travel fee stated)? After-hours/weekend surcharge?
* D4. Walk-in submission scope: Tumwater only, or also county
  registrars / consulates (Seattle-area)? Mileage policy.
* D5. Interaction with the $1.99 estate/inheritance offer: which
  tiers does it cover, hard cap accounting ($50k), end date June 2027
  — handbook must state the boundary, not marketing copy.

**WA fee/SLA/pickup-radius (cross-cutting):** author to fix one table
— SOS fee passthrough (exact, cited), Andrii Co. service fee per
tier, notary/travel fees, pickup radius map, SLA per tier — before
any journey page shows a price or a date promise.

## ES-translation scope + legal review (flags, not decisions)

* **ES scope must be explicit:** (1) UI chrome, (2) handbook authority
  content (EN/ES/FR per design §11), (3) field-by-field form guides
  (ES explanation, EN field names preserved), (4) customer
  communications (tracking, support). Forms themselves stay in the
  authority's language. Customer's underlying documents are NOT
  translated by default — translation coordination (J4) is a stated
  add-on with perito-traductor guidance for Mexico.
* **Legal review required before launch:** POA/corporate templates
  are pointers and checklists, not drafting — needs a WA attorney's
  disclaimer pass ("general information, not legal advice," in EN +
  ES). Same pass for the Viet Nam explainer sheet (J5) and the Egypt
  chain guide (J6): a wrong instruction there costs customers weeks.
  Machine-translated ES legal-adjacent content must be
  human-reviewed; flag which pages are human- vs. machine-translated
  with dates.

## Open questions (for author)

1. Pickup radius + SLA for tier (d): which cities and what date
   promise can you actually keep from Bothell?
2. Tier (c) "FedEx tomorrow" promise: contiguous-US-only, and what is
   the remedy when FedEx misses?
3. Tier (b)/(c) pricing model: flat fee vs. postage pass-through, and
   single- vs. multi-document bands?
4. Which tier(s) does the $1.99 estate/inheritance offer cover, and
   how is the $50k cap tracked publicly?
5. ES scope for MVP: full ES authority pages for all 106 authorities,
   or ES-first only for top WA/CA/TX + Mexican-destination journeys?
6. Who performs legal review of templates, disclaimers (EN/ES), and
   the Viet Nam + Egypt guides — and blocks launch if missing?
7. Walk-in submission in tier (d): Tumwater counter only, or also
   county/consulate runs — and at what mileage fee?
8. Out-of-WA full-service demand (J7/J9 outside WA): refer out,
   mail-only tiers, or partner notaries — what does the handbook
   promise non-WA customers today?
