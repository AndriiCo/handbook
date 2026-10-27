# Handbook fact topics (frozen vocabulary)

- Status: FROZEN 2026-10-26 (night run; enforced by `topics.rs` +
  migration 005 `fact.topic` CHECK). Changes need a new migration
  and author sign-off.
- Code: `crates/handbook/src/topics.rs` (`Topic`, closed enum —
  `parse` rejects unknown input, no `Other` variant).
- Every `fact.topic` value MUST be one of the 25 strings below.
- Owner column = the stream that validates and signs off facts of that
  topic. S2's analysis proposes facts for every topic from captures;
  the owner below has the final say before publish. S1 owns the enum
  itself, not any topic's content.

| Topic | Owner | Meaning | Example questions |
|---|---|---|---|
| `hours` | S6 | Regular counter/phone hours | "What are the walk-in hours?" "Is the phone line 9–5?" |
| `fees` | S6 | Official fee schedule | "What is the apostille fee per document?" "Is there a counter surcharge?" |
| `forms` | S6 | Request forms and where to get them | "Which form requests an authentication?" "Is there a fillable PDF?" |
| `address` | S6 | Physical office location | "What is the street address?" "Which floor / suite?" |
| `parking` | S6 | Visitor parking at the office | "Is there visitor parking?" "Validated or paid?" |
| `route_photos` | S6 | Wayfinding photos for the visit | "Which entrance do applicants use?" "Photo of the drop-off window?" |
| `rejections` | S2 | Known rejection reasons from captures | "Why was this filing rejected?" "Which defects recur?" |
| `contacts` | S6 | Named contacts / teams | "Who handles authentications?" "Direct line or main switchboard?" |
| `po_box` | S6 | Authority's postal box (own topic, see below) | "What PO box takes mail-in requests?" "Is the PO box monitored daily?" |
| `walkin` | S6 | Walk-in rules and limits | "Are walk-ins accepted?" "Daily quota / cutoff time?" |
| `mailing` | S6 | Mail-in submission address and rules | "Where do I mail the packet?" "Certified mail required?" |
| `appointment` | S6 | Appointment booking rules | "Is an appointment required?" "How far out do slots book?" |
| `payment` | S6 | Accepted payment methods | "Do you take cards?" "Check payable to whom?" |
| `processing_time` | S6 | Stated turnaround times | "How many business days?" "Is expedited service offered?" |
| `jurisdiction` | S6 | Which office handles which cases | "Does the county clerk or SoS sign this?" "Out-of-state documents?" |
| `legalization_chain` | S6 | Full step chain for a destination | "What are the steps for use in Viet Nam?" "Embassy legalization after SoS?" |
| `website` | S2 | Official web presence and URL verdicts | "What is the official site?" "Is this lookalike domain fake?" |
| `phone_script` | S5 | Prepared call-console questions | "What do we ask on the validation call?" "Which answers are still open?" |
| `holiday_hours` | S6 | Holiday closures and special hours | "Are you open the Monday after?" "Year-end closure dates?" |
| `id_requirements` | S6 | ID the applicant must show | "Is photo ID required at the counter?" "What counts as valid ID?" |
| `copies` | S6 | Copy rules (originals, counts) | "Original or copy?" "How many copies per request?" |
| `translation` | S6 | Translation requirements | "Must foreign documents be translated?" "Certified translation needed?" |
| `notarization` | S6 | Notarization prerequisites | "Must this be notarized first?" "Remote notarization accepted?" |
| `authentication` | S6 | Authentication vs apostille distinction | "Is this an apostille or an authentication?" "SoS authentication ≠ apostille?" |
| `dropbox` | S6 | Physical drop-box rules | "Is there an after-hours dropbox?" "When is it emptied?" |

## `po_box`: own topic, merge guidance, batching rule

- `po_box` is its OWN topic (roadmap proposed default, kept). It is
  also a valid mailing address, but recording it under `mailing`
  loses the distinction between "street address for couriers" and
  "postal box for USPS mail" — couriers cannot deliver to a PO box,
  so the distinction drives order-path decisions.
- Merge guidance (owner S1): do NOT merge `po_box` into `mailing`.
  When an authority's PO box is also its only mailing address, record
  TWO facts (one `po_box`, one `mailing`) citing the same capture
  instead of merging topics. A merge happens only on explicit author
  override, recorded in `docs/specs/README.md` the same turn.
- Batching rule: never spend one call on one question. Ask `po_box`
  together with `mailing`, `address`, `appointment`, and
  `processing_time` questions in a single call batch (the `batch`
  table holds many questions per outbound contact).
