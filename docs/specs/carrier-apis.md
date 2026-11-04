# Carrier APIs: USPS + FedEx Labels (research draft for review)

- Status: draft (prompt 010, for author review; no implementation yet)
- Date: 2026-10-26
- Source: parallel researcher draft (full text recovered from child
  session log). All endpoint/claim URLs below were fetched or
  search-verified by the researcher; items marked unverifiable need
  portal/spec or carrier-rep confirmation.
- Decides the price of P2 path (A) carrier-API server buy vs the
  recommended (B) staff-manual (see
  [p2-forms-labels.md](p2-forms-labels.md) §7, OQ-3).

Scope: domestic USA first (P2-U1 happy path, pilot Washington state).
Cross-border/customs is P3 (P2 §11).

## 1. USPS — current platform ("USPS APIs", REST+JSON+OAuth)

Old Web Tools XML APIs retired 25 Jan 2026; replacement is the USPS
APIs platform ([migration note](https://dev.to/jeffreyinman/live-usps-rates-after-web-tools-the-60-calls-an-hour-limit-1ppg),
[catalog](https://developers.usps.com/apis)). On-prem
`developer.usps.com` creds were migrated to cloud
`developers.usps.com`, usable at `apis.usps.com` ([USPS transition
notice](https://postalpro.sequirly.co/mnt/glusterfs/2025-07/INFORMATIONAL%20USPS%20Transitions%20from%20an%20On-prem%20Environment%20to%20a%20Cloud-based%20Platform.pdf)).

- Base URLs: prod `https://apis.usps.com`, test (TEM — Testing
  Environment for Mailers) `https://apis-tem.usps.com` — test by
  swapping the base URL with prod credentials ([getting
  started](https://developers.usps.com/getting-started), [AbleCommerce
  doc](http://help.ablecommerce.com/mergedProjects/ablecommerce/Configure/Shipping/Carriers/USPS_OAuth.htm)).
  Older examples use `https://api.usps.com` (e.g. [StackOverflow label
  thread](https://sm-stackoverflow.azurefd.net/questions/79029934/usps-api-the-response-of-creating-label-is-not-json-xml-anyone-know-how-to-dese));
  treat `apis.usps.com` as current and verify at integration time.
- Auth: OAuth 2.0 client-credentials. Consumer Key/Secret from a COP
  app act as `client_id`/`client_secret`; token endpoint `POST
  https://api.usps.com/oauth2/v3/token` (verify host:
  `apis.usps.com` equivalent) ([getting
  started](https://developers.usps.com/getting-started), [Postman
  docs](https://www.postman.com/avionics-participant-52825978/work/documentation/j85updc/usps?entity=request-31212650-18dfb53b-4d79-4056-b38b-e0b00a7cbd7d)).
  All calls send `Authorization: Bearer`
  ([catalog](https://developers.usps.com/apis)).
- Account setup: USPS Business Account in the [Customer Onboarding
  Portal (COP)](https://cop.usps.com/cop-navigator?wf=API&showCC=false)
  → yields CRID, MID, Enterprise Payment Account (EPA); create app,
  take credentials ([getting
  started](https://developers.usps.com/getting-started), [PostalPro
  tech guides](https://postalpro.usps.com/taxonomy/term/35)).
- Labels require extra approval: "If you want to use the Labels APIs,
  it requires additional approval and configuration… enrolled in
  USPS Ship for both outbound and return labels and have an
  Enterprise Payment Account" ([getting
  started](https://developers.usps.com/getting-started)). Default
  product already includes OAuth/Addresses/Pricing/Tracking; quota
  increases and non-default APIs go through a [USPS API service
  request](https://emailus.usps.com/s/usps-APIs). Lead time for
  Labels approval is **unverifiable from public docs — open
  question**.
- Label endpoints ([migration
  guide](http://usps.com/business/web-tools-apis/web-tools-label-api-migration-guide.pdf)):
  `POST /labels/v3/label` (domestic label), `POST
  /labels/v3/returnlabel` (domestic return), `DELETE
  /labels/v3/label/{trackingNumber}` (cancel). Label creation also
  validates the address, confirms availability, computes postage,
  and generates the Shipping Services File manifest
  ([catalog](https://developers.usps.com/apis)). Tip: request
  `Accept: application/vnd.usps.labels+json` to get JSON instead of
  multipart
  ([StackOverflow](https://sm-stackoverflow.azurefd.net/questions/79029934/usps-api-the-response-of-creating-label-is-not-json-xml-anyone-know-how-to-dese)).
- Postage payment: `POST /payments/v3/payment-authorization` mints
  a payment-authorization token from OAuth token +
  roles/account numbers/CRIDs/MIDs ([PlugInHive USPS
  doc](https://www.pluginhive.com/usps-label-printing-shipment-issues-multi-carrier-shipping-label-app-for-shopify-faqs/));
  the Label API authorizes an Enterprise Payment (EPS) or PERMIT
  account via that token ([migration
  guide](http://usps.com/business/web-tools-apis/web-tools-label-api-migration-guide.pdf)).
  Postage is charged to the EPA; if we buy labels for customers on
  our EPA we are the merchant of record (see also the two-step
  merchant→label-provider authorization in the [Merchant Onboarding
  Tech Guide](https://postalpro.usps.com/taxonomy/term/35)). EPA
  funding mechanics (ACH prefund vs postpay) **unverifiable — open
  question**.
- Services (domestic): USPS Ground Advantage, Priority Mail,
  Priority Mail Express (mail-class names on labels; STC-based)
  ([Nov 2024 release
  notes](https://github.com/USPS/api-examples/releases/download/4.9.6/USPS.APIs.2024.November.Release.Notes.pdf));
  returns have matching Ground/Priority/Express Return services
  (older [Returns Label
  doc](https://stage-www.usps.com/business/web-tools-apis/uspsreturnslabel-api.pdf)
  — verify current enum values against the v3 spec). Covers P2-L2's
  `PRIORITY|EXPRESS|GROUND_ADVANTAGE`.
- Label formats: PDF, TIFF, SVG, JPG, ZPL203DPI, ZPL300DPI
  ([catalog](https://developers.usps.com/apis)); EPL203DPI/EPL300DPI
  added Sept 2026 ([Sept 2026 release
  notes](https://postalpro.usps.com/mnt/glusterfs/2026-09/USPS%20APIs%202026%20September%20Release%20Notes.pdf)).
  P2 needs PDF — supported.
- Tracking: Tracking API v3 ([Postman thread, token URL + tracking
  ID](https://community.postman.com/t/usps-tracking-api-v3/58593),
  spec hub
  [trackingv3r2](https://postalpro.sequirly.co/mnt/glusterfs/2026-05/PTR%20Tracking%20API%20Specifications%20and%20Integration.pdf));
  exact REST path **resolved prompt 080** against the official
  [USPS/api-examples
  README](https://github.com/USPS/api-examples): `GET
  https://apis.usps.com/tracking/v3/tracking/{trackingNumber}?expand=DETAIL`
  (single; `?expand=SUMMARY` for list views), batch variant `GET
  .../tracking/v3r2/tracking/` with trackingNumber body; TEM base
  swap for test. Labels API also returns a tracking-URL link ([Aug
  2025 release
  notes](https://github.com/USPS/api-examples/releases/download/4.9.20/USPS.APIs.2025.August.Release.Notes_v2.pdf)).
- Void/reissue/refund: cancel via `DELETE
  /labels/v3/label/{trackingNumber}` ([migration
  guide](http://usps.com/business/web-tools-apis/web-tools-label-api-migration-guide.pdf));
  IMB cancel-or-refund-request flow exists ([June 2025 release
  notes](https://github.com/USPS/api-examples/releases/download/4.9.15/USPS.APIs.2025.June.Release.Notes.pdf));
  retail Click-N-Ship refund window is 30 days ([USPS
  FAQ](https://faq.usps.com/articles/Knowledge/USPS-Label-Delivery-Service)).
  Whether API-cancelled-but-manifested labels auto-refund to EPA
  **unverifiable — open question**.
- Rate shopping: Domestic Prices API `POST
  /prices/v3/total-rates/search` (base + extras) and
  `/extraservicesrates/search` ([June 2025 release
  notes](https://github.com/USPS/api-examples/releases/download/4.9.17/USPS.APIs.2025.June.Release.Notes.pdf));
  Shipping Options `/options/search`. P2 §11 excludes rate
  shopping, but the endpoint exists for P3.
- Address validation: Addresses API v3 — Address Standardization
  (ZIP+4/delivery point), City/State and ZIP lookups
  ([catalog](https://developers.usps.com/apis)). Also note the
  separate licensed NCOA+ API (needs API license via COP, [getting
  started](https://developers.usps.com/getting-started)) — not
  needed for P2.
- Return-label pairing: return labels are a separate endpoint
  (`/returnlabel`); "return-label pairing id" (P2-L2) would be our
  own `shipment` row linking two `shipping_label` rows (P2-D3/D4)
  — no USPS-side pairing construct found in public docs.
- Code samples: [USPS/api-examples on
  GitHub](https://github.com/USPS/api-examples) (Postman/curl).

## 2. FedEx — REST APIs

Portal [developer.fedex.com](https://developer.fedex.com/api/en-us/catalog/ship/v1/docs.html);
legacy SOAP/WebServices superseded by REST (`apis.fedex.com`).

- Base URLs: prod `https://apis.fedex.com`, sandbox
  `https://apis-sandbox.fedex.com` ([Acumatica setup
  doc](https://help.acumatica.com/(W(89))/Wiki/ShowWiki.aspx?wikiname=HelpRoot_OrderMgmt&PageID=3facd1eb-c78d-4582-9d2b-18522ddb4d4d),
  [Tawridy guide](https://help.tawridy.com/integrations/fedex.html)).
- Auth: OAuth 2.0 client-credentials, `POST
  https://apis.fedex.com/oauth/token`
  (`grant_type=client_credentials&client_id=API_KEY&client_secret=SECRET`),
  1-hour Bearer [REDACTED] ([copyprogramming
  guide](https://copyprogramming.com/howto/fedex-tracking-in-google-sheets),
  [H3XED PHP
  guide](http://www.h3xed.com/programming/how-to-use-fedex-api-with-php-examples),
  [browse.sh probe](https://browse.sh/skills/fedex.com/track-package-7fxxv5)).
  Sandbox and prod credentials are separate; sandbox works
  immediately, prod after approval
  ([RapidDev](https://www.rapidevelopers.com/bubble-integrations/fedex-api)).
- Account setup: fedex.com account → developer portal → Create API
  Project → pick Ship, Rate, Address Validation, Track ([Cavallo
  portal
  guide](https://support.cavallo.com/kb/articles/pdf/fedex-developer-portal),
  [Tawridy](https://help.tawridy.com/integrations/fedex.html)).
  Ship calls additionally require the 9-digit FedEx shipping
  account number in the request ([Ship
  docs](https://developer.fedex.com/api/en-us/catalog/ship/v1/docs.html),
  [Cavallo](https://support.cavallo.com/kb/articles/pdf/fedex-address-validation-rest-api-configuration)).
- Sandbox/certification: sandbox open immediately with simulated
  tracking numbers; **production Ship requires label
  certification** — submit scanned test labels + cover sheet to
  the Bar Code Analysis Group; FedEx temporarily enables prod
  keys (~1 week) to print the samples
  ([SupportSync](https://supportsync.zendesk.com/hc/en-us/articles/9914511339789-Setting-up-FedEx-Shipping),
  [AutoPrint
  KB](https://autoprint-software.atlassian.net/wiki/spaces/KB/pages/4507041832/How+to+complete+FedEx+API+Certification+for+shipping+labels?atl_f=content-tree),
  [cover sheet
  PDF](https://developer.fedex.com/content/dam/fedex-com/irc/common/Label-Cover-Sheet-form.pdf),
  submit to `label@fedex.com` / Collierville TN ([cover sheet
  v7.6](https://support.cityhive.net/hubfs/Label-Cover-Sheet-form.pdf))).
  Calendar lead time **unverifiable — open question** (third
  parties imply days-to-weeks; do not plan the 10-day build
  around it).
- Create/validate/cancel: `POST
  https://apis.fedex.com/ship/v1/shipments` (1–40 packages;
  returns tracking + label as URL or base64); Validate Shipment
  endpoint for pre-check; cancel at
  `https://apis.fedex.com/ship/v1/shipments/cancel` ([Ship
  docs](https://developer.fedex.com/api/en-af/catalog/ship/docs.html),
  [CodeFixup](https://www.codefixup.com/how-to-create-fedex-shipping-label-via-api-call-in-php/),
  [Acumatica](https://help.acumatica.com/(W(5))/Wiki/Print.aspx?pageid=3facd1eb-c78d-4582-9d2b-18522ddb4d4d)).
  Cancel HTTP method (PUT vs POST) **unverifiable from sources
  read — confirm in spec**.
- Services: `serviceType` enum, e.g. `STANDARD_OVERNIGHT`,
  `FEDEX_GROUND` ([CodeFixup](https://www.codefixup.com/?p=96));
  family includes First/Priority/Standard Overnight, 2Day/2Day AM,
  Express Saver, Ground, Home Delivery, Ground Economy (per [cert
  cover sheet services
  list](https://developer.fedex.com/content/dam/fedex-com/irc/common/Label-Cover-Sheet-form.pdf)
  and Adobe guide excerpt). P2-L2's
  `FEDEX_OVERNIGHT|FEDEX_2_DAY|FEDEX_GROUND` are placeholders —
  real enums are finer (pick e.g. STANDARD_OVERNIGHT / FEDEX_2_DAY
  / FEDEX_GROUND at implementation).
- Packaging: `packagingType` + per-package weights required ([Ship
  docs](https://developer.fedex.com/api/en-us/catalog/ship/v1/docs.html));
  envelope preset = `FEDEX_ENVELOPE` (standard value; confirm enum
  in spec).
- Pickup: `pickupType` required for Express/Ground — use drop-off
  value for P2 (no pickup scheduling, P2-L2/§11) ([Ship
  docs](https://developer.fedex.com/api/en-us/catalog/ship/v1/docs.html)).
- Label formats: ZPL, DPL, EPL, PDF, PNG per the [cert cover
  sheet](https://support.cityhive.net/hubfs/Label-Cover-Sheet-form.pdf);
  Ship response label options URL-or-encoded ([Ship docs
  AF](https://developer.fedex.com/api/en-af/catalog/ship/docs.html));
  stock types incl. `STOCK_4X6`
  ([RocketShipIt](https://docs.rocketshipit.com/rs/docs/fedex-freight.html)).
  P2 PDF — supported. Certify the exact image type + printer we
  ship.
- Billing: `shippingChargesPayment` (SENDER/RECIPIENT/THIRD_PARTY)
  + account number; no charge until a label is scanned
  ([parcelpath](https://parcelpath.com/shipping-companies/fedex/fedex-create-label/));
  one-off portal buys can go on credit card without an account
  ([parcelpath](https://parcelpath.com/shipping-companies/fedex/fedex-create-label/))
  — relevant to staff-manual (B). Invoice/billing-account setup
  for API volume **not researched — open question**.
- Rate shopping: `POST
  https://apis.fedex.com/rate/v1/rates/quotes`
  ([Acumatica](https://help.acumatica.com/(W(5))/Wiki/Print.aspx?pageid=3facd1eb-c78d-4582-9d2b-18522ddb4d4d)).
  Excluded from P2 (§11), available for P3.
- Address validation: Address Validation API ([docs
  hub](https://help.kwi.com/knowledge-base/fedex-oauth-2-0-migration/?print=print),
  [Cavallo
  config](https://support.cavallo.com/kb/articles/pdf/fedex-address-validation-rest-api-configuration)).
  Exact REST path **unverifiable from sources read — confirm in
  spec** (commonly `/address/v1/addresses/resolve`; do not
  hard-code).
- Tracking: `POST
  https://apis.fedex.com/track/v1/trackingnumbers`
  ([copyprogramming](https://copyprogramming.com/howto/fedex-tracking-in-google-sheets),
  [PyPI daylily](https://pypi.org/project/daylily-carrier-tracking/)).
- Returns: two native modes via
  `returnShipmentDetail.returnType`: `PRINT_RETURN_LABEL` (we
  print, include in outbound box or send separately) and `PENDING`
  + `pendingShipmentDetail` (FedEx emails the label to the
  customer; pending email returns expire ~1 month; no emails sent
  in test env) ([RocketShipIt
  params](https://docs.rocketshipit.com/rs/docs/fedex-api-parameters.html),
  [RocketShipIt
  returns](https://docs.rocketshipit.com/rs/docs/return-labels.html),
  [Ship docs
  AF](https://developer.fedex.com/api/en-af/catalog/ship/docs.html)).
  Either satisfies P2-L1's return label; PRINT_RETURN_LABEL fits
  "print both PDFs" UX best.

## 3. Mapping to the P2 payload schema (P2-L2 + §8–9)

Staff-manual (B) payloads (`shipment.from_json/to_json/parcel_json`,
`shipping_label.payload_json`, `POST /shipments` preview) should
collect exactly the API-buy superset so P3 reuses them:

| P2 payload field | USPS Labels v3 | FedEx Ship v1 |
|---|---|---|
| sender/recipient name, street, city, state, postal, country, phone | from/to address objects (address validated inside label call) | `shipper`/`recipient` contact+address; validate via Address Validation API |
| parcel weight + dims (letter/flat preset + override) | package description: weight/dims; cubic inputs where applicable | per-package weights + `packagingType` (`FEDEX_ENVELOPE` preset) |
| service code | mail class: Ground Advantage / Priority / Express | `serviceType`: STANDARD_OVERNIGHT / FEDEX_2_DAY / FEDEX_GROUND (finalize enums) |
| label format | `imageType`/label option = PDF | `labelSpecification` imageType PDF |
| pickup | n/a (drop-off/collection) | `pickupType` = drop-off value |
| return pairing id | our `shipment` row links outbound `/label` + `/returnlabel` | our row links two shipments, or one outbound + `PRINT_RETURN_LABEL` return |
| tracking (post-purchase) | response tracking no. + tracking URL | response tracking no.; Track API for status |
| payment ref | EPA account + payment-auth token (server-side, never customer PII) | account no. + SENDER payment (server-side) |
| cost record | response postage → `postage_cost_cents` | rate/ship response charges → `postage_cost_cents` |

P2 records `postage_cost_cents` + `paid_by` without charging (L4)
— both APIs return the chargeable amount at buy time, so the
record-first design ports cleanly. WS events
(`label.pending/ready/voided`, §9) map to: buy response →
`label.ready`; cancel endpoint → `label.voided`; staff-manual
differs only in who clicks buy.

## 4. What API-buy (A) additionally requires over staff-manual (B)

1. Accounts: USPS COP business account + CRID/MID/EPA + USPS Ship
   enrollment + Labels approval ([getting
   started](https://developers.usps.com/getting-started)); FedEx
   account + developer project
   ([Cavallo](https://support.cavallo.com/kb/articles/pdf/fedex-developer-portal)).
   B needs the same portal logins for staff but no API approvals,
   no EPA/API billing linkage. OQ-3 (whose accounts, who pays)
   applies to both; A additionally puts our binary in the money
   path.
2. Secrets: USPS consumer key/secret + payment-auth token flow;
   FedEx API key/secret + account number. B stores zero carrier
   secrets.
3. Certification + lead time: FedEx label certification via Bar
   Code Analysis Group ([cover
   sheet](https://developer.fedex.com/content/dam/fedex-com/irc/common/Label-Cover-Sheet-form.pdf));
   USPS Labels approval + Ship enrollment. Both have unverifiable
   calendar lead times — the core risk to a 10-day build,
   supporting L4's (B)-for-P2 recommendation.
4. Edge-case code P2 §11 explicitly excludes: rate-shop,
   address-validation UX, void/refund automation, reissue, billing
   disputes. Under (B) these are staff clicks +
   `shipping_label.status` transitions (L5).
5. Printer/cert coupling: FedEx certifies exact image type +
   printer ([cert
   guide](https://support.cityhive.net/hubfs/FedEx%20-%20CertificationGuidelines.pdf?hsLang=en));
   changing label stock/format later re-opens certification. USPS
   label is "USPS certified" output from the API
   ([catalog](https://developers.usps.com/apis)) — no per-printer
   cert found.
6. Keep payloads carrier-neutral per L4; the table above is the P3
   shopping list.

## 5. Open questions

1. USPS Labels approval + USPS Ship/EPA enrollment lead time
   (days? business verification?) — needed to schedule P3.
2. FedEx label-certification turnaround + whether one cert covers
   Express and Ground with PDF-only.
3. Exact FedEx cancel method and Address Validation REST path —
   confirm in spec before coding.
4. USPS Tracking v3 REST path — resolved prompt 080 (§1; see
   also shipping-design §8.2). Remaining: webhook/polling story for
   `label.*` WS parity (push = Subscriptions-Tracking API 3.2;
   receiver IP-whitelist steps still to confirm in portal).
5. USPS void→EPA refund semantics (auto-refund on DELETE? window?)
   and EPA funding mechanics.
6. FedEx billing-account/invoicing setup for API volume vs
   credit-card portal buys.
7. Final service-type enums to offer (STANDARD vs PRIORITY
   overnight? Express Saver? Ground Economy?) — product + cost
   decision for OQ-8 demo slice.
8. Who holds carrier accounts and pays postage (OQ-3 passthrough):
   customer-prepaid at checkout vs absorbed — blocks both (A) and
   (B) checkout copy.

## 6. Fake carrier (W-3, `labels.mode = fake`)

Loopback and staging boots only (`docs/specs/config-db.md` §9). Today
`labels.live = false` means the carriers' sandbox; that stays the
`sandbox` mode. `fake` swaps the network for `labels::demo::DemoTransport`,
an `HttpTransport` that answers the FedEx Rate/Ship/cancel and USPS
Prices/Labels/cancel calls in their documented shapes, so every buy
path runs its real code — caps, the idempotency ledger, audit rows,
blob storage, the customer's cabinet:

- staff `/labels` (`fedex_live`): no credentials or account needed,
  generous demo caps (50 a day), the requested ZIP allowed;
- customer label orders (`customer_labels`): the switch reads on and
  the demo link is used — but only for test-mode orders: a live-mode
  (real money) order is never quoted or bought a demo label;
- P2 prepaid USPS (`p2_labels`): no sandbox or certification gate;
  a demo label voids locally.

Every label is a 4x6 PDF stamped **"DEMO — NOT POSTAGE"** (plus a
diagonal watermark and "No postage. Do not mail.") with a tracking
number from the reserved fake range `DEMO` + 12 digits, which no
carrier issues; `labels::demo::is_demo_tracking` keeps the tracking
poller from ever sending one to a carrier. Quotes are flat demo
prices (FedEx $12.50, USPS $9.85); postage is zero.
