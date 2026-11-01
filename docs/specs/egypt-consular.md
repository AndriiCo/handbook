# Egypt Consular-Legalization Chain for US Documents, MVP (draft for review)

- Status: draft (prompt 007, for author review; no implementation yet)
- Date: 2026-10-26
- Source: parallel researcher draft (full text recovered from child
  session log). All findings from researcher reads on 2026-10-26.
  Per the evidence-store methodology ([design.md](../design.md)
  §3–§5), every factual claim carries its exact URL; anything
  uncertain or contradictory is labeled a **PHONE-VALIDATION TASK**,
  not a fact.
- Feeds [app-features.md](app-features.md) F7. MVP value in one
  sentence: a customer who buys a state *apostille* for Egypt gets
  rejected at the last hop — the state-level product must be a
  **Certificate of Authentication**, not an apostille.

## 0. Method note (trust tiers applied)

- **Official, fetched firsthand (tier: `official`):** `egyptembassy.net`
  DC pages (personal, commercial, signatures, translation, consulates
  list), `egyconsulatela.com` LA site + fee-schedule PDF (OCR'd from
  scan), HCCH status table. Intended `capture(kind=warc)` + `fact`
  rows with `capture_id` provenance.
- **Official, bot-blocked (tier: `official` claimed, authenticity
  pending):** `travel.state.gov` (HTTP 403 to the researcher's
  fetcher, 3 attempts) and `sos.wa.gov` (403/404). Facts about these
  two offices below come from official-domain search snippets and
  corroborating third parties — all cited — but **must not be
  published as handbook facts until a staff-verified capture exists**
  (headless-browser WARC or manual save). Logged as capture tasks
  V-1, V-2.
- **Arabic-only sources** (DC translation page, signatures page, LA
  fee PDF) were read by this researcher; per methodology they need an
  independent bilingual check before any derived fact is `accepted`
  (task V-3).

## 1. Why Egypt takes the consular route (no apostille)

- **Egypt is absent from the HCCH Apostille status table.** The
  researcher verified the fetched table is complete (spot-checked
  parties Albania→Vanuatu; the E-section contains only Ecuador, El
  Salvador, Estonia, Eswatini) — Egypt does not appear. Table last
  updated 30-VI-2026, 130 Contracting Parties. Source:
  https://www.hcch.net/en/instruments/conventions/status-table/?cid=41
- Corroborating (third-party): Egypt "not a contracting party," HCCH
  status table does not list it. Sources:
  https://medium.com/@apostillemypassport/egypt-apostille-services-for-international-document-certification-f6c005f6081a
  and
  https://www.apostille.us/news/indonesia-asks-egypt-to-join-apostille-c.shtml
  (Egypt is an HCCH *member organization* but not an Apostille
  Convention party — do not conflate the two).
- **Decisive customer-facing rule, in the embassy's own English:**
  *"the consulate only accepts authentication and certification from
  the Secretary of State. Apostilles are not accepted."* Source:
  https://egyptembassy.net/consular-services/legalization/legalization-of-signatures/
  — This single sentence is the MVP's value proposition.

## 2. The ordered MVP chain (USA → Egypt)

### Step 0 — Notarization (when the document needs it)

- DC requires a **Notary Public stamp** as chain step 1 for personal
  docs; for employment letters/medical reports the notary must
  witness the signer's signature. Commercial docs: *"Notary's dry
  seal must be next to the signature not on a separate paper."*
  Sources:
  https://egyptembassy.net/consular-services/legalization/personal-legalization-procedure/
  and
  https://egyptembassy.net/consular-services/legalization/commercial-legalization-procedure/
- POAs executed before a notary: the notary's signature must be **on
  the POA itself, over the principal's signature, not on a separate
  paper** (Arabic page; needs bilingual check). Source:
  https://egyptembassy.net/consular-services/legalization/legalization-of-signatures/
- **County pre-step variance (real, NY-confirmed):** NY requires the
  notary's signature to be certified by the **County Clerk of the
  county where the notary is qualified** before the Dept. of State
  will act — including NYC vital records and notarized school
  records. Official sources:
  https://dos.ny.gov/apostillecertificate-authentication-request-forms
  and https://www.businessexpress.ny.gov/app/answers/cms/a_id/3423 ;
  corroborated by https://www4.erie.gov/clerk/apostilles .
  **PHONE-VALIDATION TASK V-4:** enumerate every other state needing
  a county/clerk pre-certification for Egypt-bound authentications
  (known candidates from general practice, unverified: NY only is
  confirmed here).
- **Arabic-only documents — bilingual workaround is official embassy
  guidance:** the signatures page states (Arabic) that US notaries
  sometimes refuse to sign Arabic-language POAs/declarations they
  cannot understand; the remedy is either a notary who knows Arabic,
  **or drafting the POA/declaration in Arabic and English on one
  page** and having the notary certify that. Source:
  https://egyptembassy.net/consular-services/legalization/legalization-of-signatures/
  (needs bilingual check, V-3).

### Step 1 — Secretary of State: Certificate of AUTHENTICATION (not apostille)

- DC personal page step 2: *"Authentication by the Secretary of your
  Local State."* Commercial page step 2: *"Authentication of the
  Secretary of the Local State."* Same two URLs as above.
- The SOS authentication certificate must bear **the same notary
  name** as the one who took the signature (Arabic signatures page).
  Same source as above.
- Originals required; if submitting copies they must be certified by
  the issuing authority and carry the full chain up to the SOS. Same
  source.
- **Washington SOS post-Tumwater status (partially verified):**
  walk-in apostille/authentication moved to **Town Center 3, 243
  Israel Rd SE, Tumwater, WA 98501**, with an eastern office at
  Eastern Washington University, 960 Washington Street, Cheney, WA
  99004 (official contact page, search snippet):
  http://www.sos.wa.gov/about-office/contact-us . The official
  Dec-2024 request form (fee **$15/doc**, **$100 expedite per 10
  docs**, mail to **PO Box 40228, Olympia WA 98504-0234**, overnight
  by commercial carrier, 360-725-0377 opt 3) now returns **HTTP
  404** — i.e., search engines still surface a dead form, exactly
  the README's superseded-form hazard:
  https://www.sos.wa.gov/sites/default/files/2025-01/Apostilles.Request.Form_.12.2024.pdf
  . Third-party 2026 reports add: 7–10 business days by mail;
  same-day in-person **$150 for the first 10 docs + $15/doc**.
  Sources: https://www.globeia.com/blogs/how-to-get-washington-state-apostille
  and
  https://legalclarity.org/how-to-get-a-birth-certificate-in-washington/
  . **PHONE-VALIDATION TASKS V-5:** (a) current WA mail-to address —
  the README's "two PO Boxes" conflict is still unresolved; live
  6.2025 CORPS Records Request (verified 2026-11-01, PDF text
  extracted) names **PO Box 40234, Olympia WA 98504-0234**,
  **$15 per apostille**, plus a country-of-use field:
  https://www.sos.wa.gov/sites/default/files/2025-06/6.2025%20CORPS%20Records%20Request.pdf
  (vs PO Box 40228 in the dead Dec-2024 standalone form); (b)
  confirm WA issues a distinct "Certificate of Authentication"
  product for non-Hague countries and its exact request wording; (c)
  confirm same-day fee/hours.
  - Parent note: `PO Box 40228` above is a **cited observation from
    the superseded form**, pending validation (V-5) — it is not a
    fixture. Canonical *fixture* values remain `40220`/`40234`
    (see [README.md](README.md)) and must never leak into facts.

### Step 2 — U.S. Department of State, Office of Authentications (conditional hop)

- **When required (embassy's rule):** commercial documents,
  court-issued documents, and marriage/divorce judgments/decrees
  must carry USDOS authentication; the embassy then legalizes over
  the USDOS (or SOS) signature. For plain personal documents (e.g.,
  employment letters) the DC personal page lists USDOS as step 3 but
  its mandatory language covers only court/marriage/divorce
  instruments. Sources: personal + commercial + signatures pages
  above. **PHONE-VALIDATION TASK V-6:** confirm with DC whether a
  plain employment letter with only SOS authentication is accepted
  (no USDOS) — this decides whether MVP Step 2 can be skipped for
  personal docs.
- Fee **$20 per document**, form **DS-4194** (Request for
  Authentication Services), check/money order payable to "U.S.
  Department of State," prepaid return envelope included, country of
  use in Section 4. Sources:
  https://medium.com/@elsayedatef20202027/department-of-state-apostille-services-for-international-documents-636ba31be336
  ,
  https://www.lurodocs.com/guides/federal-apostille ,
  https://www.dcmobilenotary.com/dc-mobile-blog/how-to-get-an-apostille
- **Mail to:** Office of Authentications, U.S. Department of State,
  **44132 Mercure Circle, PO Box 1206, Sterling, VA 20166-1206**.
  Sources: https://www.lurodocs.com/guides/federal-apostille and
  https://legalclarity.org/how-to-get-an-apostille-in-tennessee/
- **Walk-in:** **600 19th St NW, Washington DC 20006** (corroborated
  by 22 CFR 5.3:
  https://casetext.com/regulation/code-of-federal-regulations/title-22-foreign-relations/chapter-i-department-of-state/subchapter-a-general/part-5-organization/section-53-rules-of-procedure-description-of-forms-available-or-the-places-at-which-forms-may-be-obtained
  ). Reported counter hours: open 8am–3pm Mon–Fri; walk-ins 8–9am
  Mon–Thu; appointments 10:30am–2:30pm Mon–Thu; **no counter service
  Friday** (mail processing day). Single third-party source — **V-7**:
  https://quartzmountain.org/article/is-state-department-travel-advisory
  . Status phone: **202-485-8000** (official travel.state.gov
  sidebar, search snippet):
  https://travel.state.gov/content/travel/en/passports/resources/authentications-sidebars.html
- **Processing time — sources disagree, all third-party:** mailed
  "within five weeks" / "up to 5 weeks"
  (https://medium.com/@elsayedatef20202027/department-of-state-apostille-services-for-international-documents-636ba31be336
  , https://www.lurodocs.com/guides/federal-apostille ) vs "6–8+
  weeks, Dept. estimate in 2026"
  (https://www.sespanish.com/how-to-get-an-apostille-for-an-fbi-background-check/amp/
  ); walk-in 7 business days (Medium, above) vs 7–9 business days ≈
  2 weeks as of Oct 2026
  (https://www.sespanish.com/product/federal-apostille-service/?v=0374e8de2d70
  ); same-day only for documented life-or-death emergency with
  advance approval
  (https://www.sespanish.com/how-to-get-an-apostille-for-an-fbi-background-check/amp/
  ). **PHONE-VALIDATION TASK V-8:** get the current posted mail and
  walk-in times from the Office directly; handbook must show a range
  with as-of date, never a point estimate.

### Step 3 — Egypt embassy/consulate legalization (strict jurisdiction routing)

Route by the **state that issued/authenticated the document**, not
where the customer lives. DC accepts ONLY: Delaware, Florida,
Georgia, Maryland, N. Carolina, S. Carolina, Virginia, DC, West
Virginia. Sources: personal + commercial DC pages. Full jurisdiction
map:
https://egyptembassy.net/consular-services/egyptian-consulate-in-us/

| Post | Address (official) | Jurisdiction (official) | Contact (official) |
|---|---|---|---|
| DC Embassy/Consulate | 3521 International Ct NW, Washington DC 20008 | DE, FL, GA, MD, NC, SC, VA, DC, WV | 202-966-6342, consulate@egyptembassy.net |
| New York | 866 UN Plaza, Suite 586, NY 10017 | CT, ME, MA, NH, NJ, OH, PA, PR, RI, VT, NY | 212-759-7120, egyptnyc@gmail.com |
| Chicago | 180 N Michigan Ave, Suite 1150, Chicago IL 60601 | IL, IN, IA, KY, MI, MN, NE, ND, SD, WI | 312-332-7210/7214, consulateofegyptchicago@gmail.com |
| Los Angeles | 6300 Wilshire Blvd, Suite 1890, LA CA 90048 | AK, AZ, CA, ID, HI, MT, NV, OR, WA, WY, UT | 323-933-9700, egyconsulatela@egyconsulatela.com, www.egyconsulatela.com |
| Houston | 5718 Westheimer St, Suite 1350, Houston TX 77057 | AL, AR, CO, KS, LA, MS, MO, NM, OK, TN, TX | 713-961-4915/4916, consulateofegyptinhouston@gmail.com |

All cells above from the official page:
https://egyptembassy.net/consular-services/egyptian-consulate-in-us/
. LA jurisdiction re-confirmed on the LA site:
https://www.egyconsulatela.com/en/home

**DC fees / payment / timing (official):**
- Personal docs **$47/doc**; commercial docs **$132/doc**; pet
  legalization **$132**; signature certification $47 non-commercial
  / $132 commercial content; translation **$47/doc** (but see
  contradiction C-2 below). Sources: personal, commercial,
  translation, signatures DC pages.
- **Cashier's check only** from a US bank, payable to "Consulate of
  Egypt" (mail); cash if in person. **No personal checks, no money
  orders.** Commercial page adds: the cashier's check must stay
  valid **>1 month** after receipt. Sources: personal + commercial
  DC pages.
- Cover form: "Request to conduct a consular service":
  https://egyptembassy.net/media/Cover-letter-for-mailed-in-services.docx-1.pdf
  plus one photocopy of each document, passport copy, and **2
  photocopies of ID** for any transaction (signatures page).
- Hours: walk-ins **Mon–Fri 9:30am–12:30pm**, pickup
  **3:00–4:00pm** (pet docs: same-day pickup). Mailed personal docs:
  **10 business days**; commercial (walked or mailed): **5 business
  days**. Sources: personal + commercial DC pages.
- Third-party 2025 report of 6–7 business days for personal docs
  (https://alzayatfirm.com/practice/egypt-document-attestation-authentication/
  ) contradicts the official 10 — treat official as fact, third
  party as stale; re-confirm by phone (V-9).

**Consulate differences found (all need phone confirmation before
publishing as fact):**
- **C-1 — LA address:** official embassy page says 6300 Wilshire
  #1890, 90048, but e-visa directories list 4929 Wilshire Suite
  300, 90010
  (https://egypt-eta.com/consulate/egyptian-consulate-general-in-chicago-united-states-of-america/
  ). Trust official; **V-10** confirm LA's current street address.
- **C-2 — Translation fee:** DC translation page says $47
  (https://egyptembassy.net/consular-services/civil-affairs/translation-of-documents/
  ) but the **"unified fee schedule for all missions in the USA,
  effective 1/3/2026"** hosted by LA lists translation/certification
  at **$52** (item 7), signature certification $47 (item 5),
  commercial invoices/origin certs $132 (item 6), unspecified-value
  commercial certs $72 (item 8), non-commercial certs **$42** (item
  9), marital POAs $55 (item 12). Source PDF:
  https://www.egyconsulatela.com/assets/pdfs/consular-fees-1-3-2026.pdf
  via https://www.egyconsulatela.com/en/consular-fees . The
  $42-non-commercial vs DC's $47-personal and $52 vs $47 translation
  mismatches are unresolved — **V-11** (see Open Q1).
- **C-3 — NY commercial fee $55:** a Google snippet of the NY
  consulate's own commercial-legalization page
  (https://egypt-nyc.com/commercial-legalization ) states
  **$55/doc**, payable by cash, **USPS money order**, or cashier's —
  contradicting both DC ($132, no money orders) and the unified
  schedule ($132). The page now **404s on all URL variants** (site
  redesigned; homepage https://www.egypt-nyc.com/ renders only a JS
  shell). **V-12:** confirm NY's current legalization fees, payment
  methods, and mail rules by phone; do not publish $55.
- **C-4 — Chicago address:** official 180 N Michigan #1150, 60601 vs
  directories' 500 N Michigan #1900, 60611
  (https://egypt-eta.com/consulate/egyptian-consulate-general-in-chicago-united-states-of-america/
  ). **V-13** confirm.
- **C-5 — Houston address:** official 5718 Westheimer #1350, 77057
  (corroborated by https://embassyworld.org/tag/egyptian-consulate/feed
  ) vs one directory's 1000 West Loop N #200, 77055
  (https://assendelft.co.za/posts/egypt-consulate-in-houston/ ).
  **V-14** confirm; also confirm Houston hours (directories claim
  9:30–1:30 or 9–2 and "appointment-only" — unverified).
- **LA hours (official, LA site):** Mon–Fri **9:30am–1:00pm**
  intake, **2:00–3:00pm** pickup; site notes it is under update with
  services being added. Source:
  https://www.egyconsulatela.com/en/home . **V-15:** LA's
  legalization-by-mail rules and whether LA accepts money orders
  like NY reportedly does.
- No official websites were found for Chicago or Houston (only Gmail
  addresses on the embassy page) — **V-16:** verify whether any
  Chicago/Houston consulate site exists; if not, phone is the only
  validation channel.

### Special tracks (official DC pages)

- **Educational certificates:** TWO hops on the Egypt side — first
  the **Egyptian Cultural & Educational Bureau, 1303 New Hampshire
  Ave NW, Washington DC 20036, 202-296-4042** (www.eecous.org), then
  final consulate legalization ($47). Academic-track docs follow the
  same Cultural Office rule (signatures page, item on US academic
  certificates). Sources: personal + signatures DC pages. **V-17:**
  confirm whether the Cultural Bureau step applies in all four
  consulate jurisdictions or DC-only.
- **Pets:** vet health letter → **USDA APHIS** authentication with
  **dry embossed seal**; certificate valid **only 14 days** from
  issuance, so consulate processing (10 business days by mail) must
  fit inside it; $132. Source: personal DC page. High
  time-sensitivity — handbook should show a countdown warning.
- **POAs/declarations (mail route):** sign before notary (ID data in
  the POA text) → SOS → USDOS if commercial → consulate with
  bank-accepted check + copy of the ID used + tracked self-addressed
  return envelope. **Foreigners' sale/bank POAs: NO mail — in-person
  only**, with notary seal + SOS. US-passport ID in a POA forces
  in-person submission. POA cancellation is never done at the
  embassy (courts in Egypt only; in-person declaration required).
  Source: signatures page. Template POAs (Arabic, "for guidance"):
  https://egyptembassy.net/consular-services/legalization/legalization-of-signatures-by-mail/
  (existence of link confirmed; content not reviewed — V-3 scope).

## 3. Consulate-mail hop constraints (verified, DC)

- **USPS ONLY — FedEx, UPS, and all other carriers are refused.**
  Stated identically on all three DC legalization pages (English
  twice + Arabic on the signatures page). Return envelope must be
  **pre-paid, pre-addressed/self-addressed, with tracking**;
  Express/Certified strongly advised.
- **Packaging wording varies by page:** personal/commercial say
  **"USPS Priority Mail envelope (approx. 11″×8.5″)"**;
  signatures page says **"letter-sized USPS envelope."**
  Non-compliant packaging sits **pending and unprocessed** until
  fixed. **V-18:** confirm one canonical mailer spec; handbook must
  not promise FedEx-based prepaid labels for this hop (directly
  constrains the "prepaid labels to your door" vision — USPS labels
  only for Egypt consulate mail).
- USDOS mail hop accepts USPS or UPS return envelopes per one
  third-party (https://www.lurodocs.com/guides/federal-apostille ) —
  official rule unconfirmed (fold into V-1).

## 4. Prioritized phone-validation backlog (dedup keys in parens)

Per design §4, one batch per authority: **Batch A** = DC consulate
(202-966-6342): V-6, V-9, V-11, V-18 + Open Q1–Q3. **Batch B** =
USDOS (202-485-8000): V-7, V-8. **Batch C** = LA (323-933-9700):
V-10, V-11, V-15. **Batch D** = NY (212-759-7120): V-12. **Batch
E** = Chicago (312-332-7210): V-13 + fee/mail rules. **Batch F** =
Houston (713-961-4915): V-14 + fee/mail rules. **Batch G** = WA SOS
(360-725-0377 opt 3): V-5. **Batch H** = Cultural Bureau
(202-296-4042): V-17. V-1/V-2 are capture-engineering tasks
(bot-blocked official domains); V-3 is the bilingual review; V-4 is
the 50-state county-step survey (start with NY-confirmed baseline).

## 5. BACKLOG: where skipping the apostille benefits customers next

**Rule applied:** no membership claim asserted without an HCCH-table
check. All "absent/present" verdicts below come from the
researcher's grep of the complete fetched table
(https://www.hcch.net/en/instruments/conventions/status-table/?cid=41
, 30-VI-2026, 130 parties). "Absent" = consular route
(apostille-skipping helps); "present" = apostille route — included
only to **retire** stale consular guidance.

**P1 — High-volume consular routes (verified ABSENT from HCCH table;
build next):**

1. **United Arab Emirates** — absent. Gulf business + diaspora
   volume. Verify: which UAE embassy/consulate legalizes US docs,
   invoice-percentage fees, Arabic translation rules.
2. **Qatar** — absent. Same Gulf profile. Verify: consulate
   jurisdictions/fees in the US.
3. **Kuwait** — absent. Same. Verify: US-side consular fees and mail
   acceptance.
4. **Malaysia** — absent. Student + business docs. Verify: embassy
   DC requirements.
5. **Nigeria / Ghana / Kenya / Ethiopia** — all absent. Large
   diasporas; birth/marriage/POA docs. Verify per-country embassy
   rules (each has own chain quirks).
6. **Middle East cluster (all absent): Jordan, Lebanon, Iraq,
   Syria, Yemen, Iran, Libya, Sudan** — verify individually; several
   have sanctions/special-handling considerations that change the
   chain. Verify: whether the mission in the US is even operating
   for legalization (Syria/Yemen/Libya/Sudan especially).
7. **Cuba** — absent. Verify: Cuban interests/embassy legalization
   path for US docs.
8. **South Asia (absent): Sri Lanka, Nepal, Afghanistan** — verify
   mission rules. (Note: Pakistan and Bangladesh are PRESENT —
   apostille route; do not build consular chains for them.)

**P2 — Retire stale consular guidance (verified PRESENT;
apostille-only):** Saudi Arabia, China, Indonesia, Thailand,
Philippines, Singapore, Israel, Morocco, South Africa, Canada,
Mexico, Viet Nam — all present in the table. Viet Nam (EIF
11-IX-2026 per README) is the live example of the cost of stale
data. **V-19:** regression-sweep any handbook copy still sending
these destinations down consular legalization.

**P3 — US-state "wrong certificate" traps (same failure mode as the
Egypt MVP):** in EVERY state, customers requesting Egypt-bound
authentication must get a Certificate of Authentication, not an
apostille — but state offices default to apostilles and some share
one form for both. Priority verification order by customer volume
and known quirks: (a) **New York** — county-clerk pre-step + NYC vs
rest-of-state vital-record paths (baseline done above; verify DOS
authentication product name/fee); (b) **California, Texas,
Florida** — highest volume; verify authentication-vs-apostille form
wording and fees; (c) **Washington** — pilot authority (V-5 above);
(d) remaining states by volume. Each state needs: authentication
product name, fee, form, county pre-step (yes/no), mail/in-person
addresses, turnaround.

**Explicitly NOT asserted:** Hong Kong/Macau (extensions under
China's row — extension columns didn't survive text extraction;
verify against the full HCCH report before any claim); Taiwan (not
an HCCH state party — out of Convention framework entirely); Türkiye
(present, spelled "Türkiye" — earlier "Turkey" grep miss is a
cautionary tale for automated table parsing: match on HCCH's
canonical names).

## 6. Open questions

1. Which fee governs DC personal legalization today — the DC page's
   $47 or the "unified" schedule's $42 non-commercial / $52
   translation (C-2)?
2. Does DC accept a plain personal document (employment letter) with
   SOS-only authentication, skipping USDOS (V-6)?
3. What is the single canonical DC mailer spec — Priority 11×8.5 or
   letter-size (packaging wording variance)?
4. What are NY's current legalization fees/payment/mail rules now
   that its legalization pages 404 (C-3)?
5. Do Chicago and Houston publish any legalization requirements
   online, or is phone the only channel (V-16)?
6. What are USDOS's current posted mail and walk-in processing times
   (V-8), and is Friday still counter-closed (V-7)?
7. What is WA SOS's current mail-to address and authentication
   product name/fee post-Tumwater (V-5)?
8. Does the Cultural Bureau pre-step for educational docs apply in
   all consulate jurisdictions or DC only (V-17)?
