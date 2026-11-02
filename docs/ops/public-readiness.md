# Public readiness (builder-owned, updated 2026-11-02 ~03:35Z)

Production: `0.1.0+3836bfb`, `/healthz` ok, scoreboard **5/10 stable**
(`prod-after.json` 02:38Z → `prod-now.json` 03:32Z, no row changed
state, no rollback). Staging on `1992959`: **9/10 + demo 3/3**,
stamped, awaiting a deploy order (Run 13 in `staging.md`).

## What is red on prod, and who owns it

- GP2-data / A1 / GP2-page — prod has no accepted facts to show
  (vietnam 0/8, usa-wa-sos 1/8). Code path proven on staging
  (17 facts, all Spark-drafted, live WS). Owner: operator
  `/review` approvals.
- GP3 — Google/Apple buttons have no logos. Owner: operator
  brand SVGs.
- S1 — LICENSE is not MIT/Apache-2.0, README has no demo video
  link, repo visibility PRIVATE. Owner: operator (licence,
  video, visibility — never the agent).

## Gates before Wed 11-04 submission

- Operator: approvals, `admin.allow_ips`, brand SVGs, S1 items,
  N6 YES|NO (SA-6 runs only on YES), unit-file root step.
- Builder: second Monday deploy of stamped 1992959 only on an
  explicit order, ready by 22:00 PST, never after Mon 23:59 PST.
- Freeze from Tue 11-03 00:00 PST: no deploys; rollback only on
  operator order or three failed `/healthz` over 5 minutes.
