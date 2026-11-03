# Operator: open steps

Production runs `0.1.0+20abd03` (deployed Tue 11-03 ~15:45 PST; store
schema v12; 510 authorities loaded; scoreboard 8/10, GP3 and S1 red). The rollback point for the v12 migration is
`/var/lib/apostille/pre-v12-20261103T230443Z.sqlite`: restore it with
the service stopped; `xtask rollback` refuses to cross v12. Procedures
live in [deploy.md](deploy.md) (§ Rails has the migration path; §
Rebuild from scratch has `cargo xtask setup`). No deploy freeze on
11-03 and 11-04 (operator decision).

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
   - As root on plant, once:
     ```bash
     sudo mkdir -p /srv/scratch/apostille/{models,meta-cache,fetch-cache,work} /srv/backup/apostille && sudo chown -R chandra: /srv/scratch/apostille /srv/backup/apostille
     ```
   - In `/config`, mint `worker.token` and put the same value plus
     `META_API_KEY` in plant's `~/.config/apostille/worker.env`. The
     cron watchdog line is in `ops/plant-pipeline/README.md`.
   - On admin **Work queue**, enqueue `research.nonparty`. When it is
     done, press "Stage as packs", then load them from **Packs**.
3. **Glimmer weights:** name the Hugging Face repo (and the `.gguf` plus
   vision projector files) and put an HF token on plant. Then run
   `ops/open-weights/glimmer-serve.sh download` and `serve`, and set
   `GLIMMER_MODEL` in `worker.env`.
4. **Washington SOS notice:** the apostille hub now carries "Now open in
   Eastern Washington… Cheney… same-day… $150 same-day service fee".
   Once the daily sweep captures the change, the change drafter
   proposes it, and you approve the `service_notice` in `/review`. It
   then shows as the dossier banner and goes out in the follower
   digest.
5. **S1 (submission, before 11-04 18:00 PT):**
   - Swap in the ready licence. If it is not MIT or Apache-2.0, change
     the licence check in `xtask/ops/scoreboard.mjs` `s1()`.
   - Put the video URL on README line 6.
   - Run a full-history `gitleaks` scan, then
     `gh repo edit AndriiCo/handbook --visibility public`. All history
     becomes public.
6. **Brand logos (GP3):** use the setup wizard's brand step (`cargo xtask
   setup wizard`). Only accepted, hash-matched files from
   `private/brand/` ship.

## Deferred

- A "fake FedEx label" judge mode (follow-up).
- `ops-deploy.sh` on production prints `/srv/apostille/public-host: No
  such file` at step 3. It is harmless (the stamp check passes), and
  the next provision writes that file.
- A quote paid up to ~1 h after its window inside an open Checkout
  session (P379 low finding).
