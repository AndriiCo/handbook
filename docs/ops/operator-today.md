# Operator: open steps

Production runs `0.1.0+83602a0` (store schema 13; scoreboard 8/10,
GP3 and S1 red). The rollback point for the v13 migration is
`/var/lib/apostille/pre-v13-20261104T0737Z.sqlite`: restore it with
the service stopped; `xtask rollback` refuses to cross v13. Procedures
live in [deploy.md](deploy.md): § Rails has the migration path, and
`cargo xtask setup --target production` ships a release. This file is
the one list of open operator steps.

## Open, in order

1. **First real FedEx label:**
   - In `/config`, set `labels.fedex_client_id` and
     `labels.fedex_client_secret` (production key and secret) and
     `labels.fedex_account` (9 digits). Turn `labels.live` On. Set
     `labels.live_ship_to_allow` to the destination ZIP. Leave
     `labels.fedex_base_url` unset (unset means production).
   - Press **Restart service** on `/config`.
   - On `/labels`, fill both addresses and both phones, then press
     Quote.
   - The purchase is confirmed in chat (item and total) before Buy.
   - The caps are 2 a day and $60 a label.
2. **Plant worker (non-party research and Glimmer):**
   - Create the scratch and backup dirs once, as plant's admin user
     (the worker itself never runs as root):
     ```bash
     sudo mkdir -p /srv/scratch/apostille/{models,meta-cache,fetch-cache,work} /srv/backup/apostille && sudo chown -R chandra: /srv/scratch/apostille /srv/backup/apostille
     ```
   - Run `ops/plant-pipeline/worker-install.sh` from the Mac (it
     installs the cron watchdog; `ops/plant-pipeline/README.md`
     § Work-queue worker). Nothing to mint and no `worker.env`: within
     a minute, `ssh plant 'tail -n 20 /srv/scratch/apostille/work/worker.log'`
     shows `enrol code XXXX-XX for chandra@plant`. On admin **Work
     queue** → **Workers**, approve it by typing that code. The Meta
     key comes from production's `meta.api_key`.
   - On admin **Work queue**, enqueue `research.nonparty`. When it is
     done, press "Stage as packs", then load them from **Packs**.
3. **Glimmer weights:** name the Hugging Face repo (and the `.gguf` plus
   vision projector files) and put an HF token on plant. Then run
   `ops/open-weights/glimmer-serve.sh download` and `serve`, and set
   `worker.glimmer_model` (and `worker.glimmer_url` if it is not the
   default) in `/config`. Connected workers pick it up on their next
   connect.
4. **Washington SOS notices (pairs only):** outages come from the
   division's public-notice log
   (`/corporations-charities/public-notices/online-filing-system-outage`,
   which P450 adds to the daily watches). In `/review`, accept a
   `service_notice` only as a pair, keyed with accept-with-edit: the
   outage line as `notice__<id>` and the office's restored line as
   `restored__<id>`, with the same `<id>` (the outage date, e.g.
   `notice__2026_11_02` and `restored__2026_11_02`). Accept pairs only
   after the new guide page is live: the old page shows every accepted
   notice as an active banner. An outage that is still active may be
   accepted on its own at any time, because a banner is then correct.
   Accepted notices go out in the follower digest only after the
   digest is switched on (`notify.enabled`, SMTP and the postal address
   in `/config`); production answers `notify/status`
   `{"enabled":false}` today. The hub's "Now open
   in Eastern Washington… Cheney… $150" alert is a standing
   announcement, not a notice: its values are `walkin` and `fees`
   facts.
5. **Washington payee (with the R2 deploy):** production's payee fact
   (`#13`) is the whole sentence "Make check or money order payable in
   U.S. dollars (USD) to: Secretary of State", so the guide's "Payable
   to" row and its Copy button carry the sentence (D2 wants the payee
   alone). The guide shows values verbatim, so this is a data fix: in
   `/review`, accept-with-edit `#13` as `payee: Secretary of State`
   (the value stays inside the quote), before or together with the R2
   deploy.
6. **S1 (submission):**
   - Licence: done (`LICENSE.md`, `LICENSE-CONTENT.md`).
   - Put the demo video URL on README line 6 (script:
     [demo-script.md](../demo-script.md)).
   - Run a full-history `gitleaks` scan, then
     `gh repo edit AndriiCo/handbook --visibility public`. All history
     becomes public.
7. **Brand logos (GP3):** use the setup wizard's brand step (`cargo xtask
   setup wizard`). Only accepted, hash-matched files from
   `private/brand/` ship.
8. **Production mail (E-3):** seal the Gmail credentials (the mail and
   instruct timers are already provisioned), set the postal address and
   SMTP, turn on `notify.enabled`, then run one real notice through
   Gmail → capture → proposal → `/review` → dossier → digest and record
   it. The sending domain needs SPF, DKIM and DMARC.
9. **Product depth (after submission):** envelope kits; Egypt chain step
   validation V-1…V-18 ([egypt-consular.md](../specs/egypt-consular.md));
   grow the gold set past 30 cases.

## Decisions waiting on you

- **Future Hague parties (D-1).** `research.nonparty` subtracts every
  party on the HCCH status table, including twelve whose entry into
  force is still ahead (Thailand: 2027-02-28), though they still need
  legalization until then. Treating them as non-parties until their
  entry into force means relaxing the P436 test on purpose.
- **Retention timer.** `apostille-retention` (voice audio older than
  365 days) is installed but no target enables it; add `retention` to
  the production target's timers to turn it on.

## Deferred

- A "fake FedEx label" judge mode (follow-up).
- `ops-deploy.sh` on production prints `/srv/apostille/public-host: No
  such file` at step 3. It is harmless (the stamp check passes), and
  the next provision writes that file.
- A quote paid up to ~1 h after its window inside an open Checkout
  session (P379 low finding).
