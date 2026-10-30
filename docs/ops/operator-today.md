# Operator — today (2026-10-30, GP2-data slice)

Order matters (P101 changed the draft attribution; the proposals must be
queued by the NEW binary):

1. Deploy main (P100–P112: proposals support, `drafted_by`, quote-verified citations, `/me` 200, `/auth/providers`, `/quality`, precheck replay, doc_ids schema, taste fixes) via `docs/ops/scripts/ops-deploy.sh`.
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

A5 (deploy first so `eval` exists): `bash /home/hal/apostille/scripts/ops-a5-eval.sh`
— one live draft per gold case, writes quality.json the endpoint serves.

S1: switch LICENSE to MIT (text ready at opensource.org/licenses/MIT),
make the repo public, and link the demo video in README (video lands 11-03).

GP3 (scoreboard demands a logo inside every configured provider's button):
drop the official google.svg / apple.svg / meta.svg (one per provider prod
configures, each owner's current artwork per its guidelines) into
web/src/lib/assets/brand/, then redeploy — the login page bundles them at
build time via import.meta.glob and renders text-only buttons until they land.
