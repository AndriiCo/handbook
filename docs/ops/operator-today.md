# Operator — today (2026-10-30, GP2-data slice)

Order matters (P101 changed the draft attribution; the proposals must be
queued by the NEW binary):

1. Deploy main (P100/P101/P102: proposals script support, `drafted_by`,
   quote-verified citations, `/me` 200) via `docs/ops/scripts/ops-deploy.sh`.
2. One root script (paste the whole output back to the builder session):

```sh
bash /home/hal/apostille/scripts/ops-gp2-proposals.sh
```

(`docs/ops/scripts/ops-gp2-proposals.sh` in the repo; sync it to the server
first. Seeds vietnam + usa-wa-sos authorities if missing, fetches their
watch URLs from prod IPs, drafts proposed facts with the live Meta model.)

Then: approve N proposals for vietnam and usa-wa-sos in `/review`
(GP2-data passes at >= 8 accepted facts each; the accept step is the demo).

A2 (needs META_API_KEY in the shell, deploy first so the endpoint exists):
`META_API_KEY=... bash /home/hal/apostille/scripts/ops-a2-record.sh`
— one live SAM + Spark call per synthetic sample, verdicts installed as replay.
