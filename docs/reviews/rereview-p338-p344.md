# Re-review of the P338-P344 fixes (2026-11-02)

An adversarial workflow (4 auditors, 4 verifiers) re-read the review-fix
commits P338-P344. It confirmed 38 findings, none critical or high and none
that can abort the server; 3 claims were refuted. P349 fixes the 11 medium
findings and 6 low ones; the rest wait for T3. Three further verify
rounds checked the fixes themselves (P351-P354): #3's clock-time matcher
was withdrawn in P354 (numbers ground on whole tokens, plus ZIP+4 heads),
so equivalent time formats fail closed until a context-aware matcher
lands. Deployed in T2b, `0.1.0+78a9ca2`.

| # | Sev | Area | Where | Finding | Status |
|---|---|---|---|---|---|
| 0 | M | money | `web/src/orders.rs:1267` | The P342 quote TTL breaks payment: any order quoted more than 30 minutes ago, or returning after its session expired, gets 409 quote_expired on eve… | fixed (P349) |
| 1 | M | money | `web/src/orders.rs:1297` | orders.daily_cap counts unpaid 'open' sessions, so one account (or anonymous demo refs in test mode) can pause all checkouts for everyone | fixed (P349) |
| 2 | M | grounding | `web/src/forms_map.rs:71` | P343 form-map quote rule drops every field whose printed label is one word (Name, Address, Phone, Email, Signature, Date) | fixed (P349) |
| 3 | M | grounding | `web/src/translate.rs:216` | P340 whole-token number guard reads '8:00', '8:00-4:30', '9-5' and '98504-0228' as single numbers, so equivalent times and ZIPs fail and a plan can… | partly (P354: ZIP+4 only; times fail closed) |
| 4 | M | pdf | `forms/src/acro.rs:572` | 'Shrink to fit' uses a 0.52 em average width, so all-caps values still get cut off without any error (#9 not closed) | fixed (P349) |
| 5 | M | pdf | `forms/src/acro.rs:644` | Comb fields silently truncate to MaxLen and MaxLen is never enforced; an ISO date prints as '2026-11-' | fixed (P349) |
| 6 | M | pdf | `web/src/forms_map.rs:83` | Model may map read-only fields (e.g. calculated totals) as required; P341's read-only refusal then fails every fill of that form | fixed (P349) |
| 7 | M | pdf | `web/src/forms_map.rs:71` | P343 label-quote guard rejects single-word printed labels (Name, Address, City, Date, Signature), dropping the most common form fields | fixed (P349, as #2) |
| 8 | M | safety | `web/src/orders.rs:1297` | orders.daily_cap counts unpaid 'open' Checkout sessions, so one customer can pause payments for everyone | fixed (P349, as #1) |
| 9 | M | safety | `web/src/orders.rs:746` | #15 not closed: the per-customer bucket is checked (and recorded) before the network bucket, and the limiter still refuses every key once full | fixed (P349) |
| 10 | M | safety | `web/src/forms_map.rs:700` | Fill stores the raw posted map: false/null/blank entries skip the allowed-key and length checks and are written to p2.sqlite (new in P341) | fixed (P349) |
| 11 | L | money | `web/src/orders.rs:1415` | A Mismatch on a captured payment strands the money: the order stays awaiting_payment (it is not flagged as claimed), refund rejects the 'mismatch'… | deferred (T3) |
| 12 | L | money | `web/src/admin_orders.rs:251` | Staff advance now refuses every move from awaiting_payment, so an order wedged there has no way out (async-payment failure, a claim with no payment… | deferred (T3) |
| 13 | L | money | `web/src/orders.rs:1222` | The checkout reuse window ignores the bucket-aligned expiry: for up to 5 minutes it returns an already-expired URL, or answers 'still open' for a l… | deferred (T3) |
| 14 | L | grounding | `web/src/forms_map.rs:63` | #24 still open: the displayed label and help are never tied to label_quote | deferred (T3) |
| 15 | L | grounding | `web/src/offices.rs:222` | Offices: the clause splitter and the 'closed' substring drop real hours ranges | deferred (T3) |
| 16 | L | grounding | `web/src/offices.rs:170` | #21 still open for the 'N a'/'N p' time forms: '12 p' matches '12 pages' and '5 p' matches '$5 per page' | deferred (T3) |
| 17 | L | grounding | `web/src/offices.rs:335` | Offices mail: a PO box written 'P.O. Box' now drops the whole mailing address, and po_box_in sees only the first 'po box' | deferred (T3) |
| 18 | L | grounding | `web/src/factcheck.rs:58` | Cross-check 'carries' guard turns genuine supported verdicts into not_found | deferred (T3) |
| 19 | L | grounding | `web/src/plan.rs:531` | Fee 'per' check accepts 'a <unit>' anywhere in the fact, so a per-request fee can still be multiplied by the document count (#20) | deferred (T3) |
| 20 | L | grounding | `web/src/translate.rs:353` | Street-span 'never crosses punctuation' check is dead code, so sentence breaks create false street spans | deferred (T3) |
| 21 | L | grounding | `web/src/translate.rs:511` | Street spans are still compared as substrings, so a truncated house number passes | deferred (T3) |
| 22 | L | grounding | `web/src/translate.rs:500` | Plan block money is still compared as digit strings, so '$1,500' passes against '$15.00' | fixed (P349) |
| 23 | L | grounding | `web/src/plan.rs:313` | times_and_days_ok reads any 'am'/'pm'-prefixed word as a time marker, misses common weekday ranges, and grounds a day named only in a 'closed' clause | deferred (T3) |
| 24 | L | grounding | `web/src/plan.rs:815` | Entailment 'fail closed' lets a later duplicate verdict override an unsupported one | fixed (P349) |
| 25 | L | pdf | `forms/src/acro.rs:505` | /P fallback is not checked against the page tree: one stale widget with a dangling /P makes every fill of the form fail (#31 not closed) | deferred (T3) |
| 26 | L | pdf | `forms/src/acro.rs:518` | A ticked checkbox with no /AP state dictionary now prints nothing (regression from the old X) | deferred (T3) |
| 27 | L | pdf | `forms/src/acro.rs:523` | A checkbox/radio value that matches no on-state is silently drawn Off with status ready; maps accepted before P341 still send booleans for radios | deferred (T3) |
| 28 | L | pdf | `forms/src/acro.rs:427` | win_ansi refusal also applies to checkbox/radio export values, which are never printed, so non-ASCII export names cannot be selected | deferred (T3) |
| 29 | L | pdf | `forms/src/acro.rs:543` | Text fields with a value lose their burned-in border, background and comb grid (#3 only partly closed) | deferred (T3) |
| 30 | L | pdf | `forms/src/acro.rs:364` | bbox_to_rect ignores the appearance's /Matrix, so burned-in appearances on rotated widgets are drawn as squashed slivers off the page | deferred (T3) |
| 31 | L | pdf | `forms/src/acro.rs:610` | Burned-in appearance streams without /Resources lose the AcroForm /DR fonts once /AcroForm is removed | deferred (T3) |
| 32 | L | pdf | `web/src/forms_map.rs:555` | Unticking a checkbox the PDF ships checked is ignored: false is dropped, so the default /AS on-state is burned in | deferred (T3) |
| 33 | L | pdf | `forms/src/acro.rs:569` | Crafted-PDF DoS remains: multiline shrink loop never ends at huge DA sizes; widget repetition is unbounded and removal is O(annots x widgets) | fixed (P349) |
| 34 | L | safety | `web/src/passkey.rs:86` | check_read refuses callers already in the map and gives new keys no limit; /translations still has the original #14 lockout | fixed (P349) |
| 35 | L | safety | `handbook-store/src/db.rs:316` | Erasure deletes paid or open order_payment rows: a captured payment loses its order, fulfilment and refund path | fixed (P349) |
| 36 | L | safety | `web/src/orders.rs:1222` | #13 partly open: checkout can still hand out an expired session URL for up to ~5 minutes | same as #13 |
| 37 | L | safety | `web/src/orders.rs:1616` | #56 regression test passes even with the bug: it asserts only is_some() | fixed (P349) |
