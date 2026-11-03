# Operator: open steps

Production runs `0.1.0+9460b26` (deployed Tue 11-03 ~07:56 PST).
Procedures live in [deploy.md](deploy.md); rebuilding from scratch uses
`cargo xtask setup` (see deploy.md § Rebuild from scratch).

## Open, in order

1. **Approve in admin `/review`.** Review → "By authority" → "Accept all
   verified" for usa-wa-sos and vietnam (at least 8 accepted each, keep
   2 proposed per slug for the video). Flips GP2-data, A1 and GP2-page.
2. **Viet Nam MOFA capture:** in admin Workers press Retry on it (its old
   text is the portal's script blob).
3. **Before sending outreach email:** set `mailer.provider` in `/config`.
4. **Washington SOS alert:** sos.wa.gov posted on 11/02 that apostille
   services are unavailable; check it before recording the WA scenes.
5. **S1 (submission).** Source code is PolyForm Noncommercial 1.0.0,
   content all rights reserved. The row stays red by design:
   - Licence check: it accepts only MIT or Apache-2.0. If the hackathon
     rules accept a source-available licence, change the licence line in
     `xtask/ops/scoreboard.mjs` (`s1()`), e.g. to
     `/MIT License|Apache License|PolyForm Noncommercial License 1\.0\.0/.test(read('LICENSE').slice(0, 2000))`.
     If the rules require an OSI licence, this licence fails the
     submission whatever the row says.
   - Video: replace README line 6 with the YouTube, Vimeo or Loom URL.
   - Repo stays private (decided 11-03); the visibility check stays red.
6. **Deploy decision for candidate `a2abb38`** (P373-P379, freeze in
   force): plant stamp green, staging-green 9/10 (only S1) + demo 3/3.
   It removes the dead config keys and makes every formerly Reserved key
   work (auto-compose, Compile, customer-paid FedEx and USPS labels, the
   label fee, kit ship-from, the USPS v3 client, the tracking poller);
   every new switch is off by default. After a deploy, in order:
   `orders.ship_from` and `orders.fee_label_cents`; FedEx sandbox keys
   and `labels.fedex_base_url`; USPS keys on TEM and `usps.certified`;
   `labels.customer_live` and one Stripe test-mode order per carrier;
   `tracking.poll_enabled` last.

## Deferred

- Brand logos (GP3): the setup wizard's brand step records the licence
  terms and places `google.svg`, `apple.svg`, `meta.svg` in `private/brand/`.
- Re-run `ops-a5-eval.sh` before the A5 72-hour window ends.
- Low findings left open in P379: FedEx cancel method (POST vs the
  documented but unconfirmed PUT), a quote paid up to ~1 h after its
  window inside an open Checkout session.
