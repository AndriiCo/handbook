# Operator — today (2026-10-30, GP2-data slice)

One root script (paste the whole output back to the builder session):

```sh
bash /home/hal/apostille/scripts/ops-gp2-proposals.sh
```

(`docs/ops/scripts/ops-gp2-proposals.sh` in the repo; sync it to the server
first. Seeds vietnam + usa-wa-sos authorities if missing, fetches their
watch URLs from prod IPs, drafts proposed facts with the live Meta model.)

Then: approve N proposals for vietnam and usa-wa-sos in `/review`
(GP2-data passes at >= 8 accepted facts each; the accept step is the demo).
