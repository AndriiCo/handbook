# S1 scrub list — 2026-11-01 (builder P198; delete nothing)

Method: P197 `gitleaks detect --log-opts="--all"` over full history was
clean (385 commits). This list adds a signal scan over the S1 scrub scope:
public IPv4, IPv6, email addresses, phone numbers, PEM blocks, API-key
patterns (`AKIA`, `ghp_`, `sk-live-`, `AIza`), sudoers paths, cloud refs,
FQDNs and `ssh user@host` targets. Proposals are keep / redact / move;
the operator makes the final call before the repo goes public.

Global findings (whole scope):
- No emails, phone numbers, PEM blocks, or key-like tokens anywhere.
- Hostnames are only `andrii.ai`, `admin.andrii.ai`, `www.andrii.ai`
  (all public) plus the alias `plant` (no DNS name, IP or key in repo).
- One public server IP (`<server IPv4>`) in two prompt files, plus one
  public IPv6 in one of them — redact (rows below).
- `169.254.169.254` in `security-review-2026-10-27.md` is the well-known
  link-local metadata address inside an exploit description — keep.
- `sudoers` mentions (~10 prompt files) are procedural, no secret
  content — keep. `hal`/`root` usernames and `/srv/apostille`,
  `~/apostille`, `/var/lib/apostille` paths disclose ops layout but no
  secrets — keep (operator may still move ops docs private).
- Telnyx/GCS refs are config-field names and bucket project names, never
  values — keep.

## `docs/prompts/` — 95 numbered files + LOG.md + README.md + `sessions/` (15 files)

Battery-clean except:
| File | Signal | Proposal |
|---|---|---|
| `2026-10-27-013-deploy-secrets.md` | public IPv4 + IPv6 | redact the two addresses, keep rest |
| `2026-10-27-016-root-install.md` | public IPv4 in ACME line | redact the address, keep rest |

All other prompt files, `LOG.md`, `README.md`, `sessions/*`: keep
(signal scan clean; session log regenerated 11-01 via
`node .muse/hooks/session-log.mjs`).

## `docs/ops/` — signal scan clean throughout

| File | Notes | Proposal |
|---|---|---|
| `acme.md` | loopback only | keep |
| `apostille*.service`, `apostille*.timer` (10) | unit files, paths only | keep |
| `apostille-ship.sh`, `apostille.service` | loopback only | keep |
| `drill.md`, `staging.md`, `watch-load-check.md` | 127.0.0.1 / `plant` alias only | keep |
| `inputs-readiness-2026-10-27.md` | readiness notes | keep |
| `operator-today.md` | script sha256s, `hal` paths; live ops page | keep until public, then operator decides (move private is fine) |
| `plant-inference.md` | pilot design, no values | keep |
| `security-review-2026-10-27.md` | H2/H3/H5/H6, M3-M6, L1-L4 + deferred M1/M2 parts still open | move out of public tree unless findings are fixed first (ship-it step 5) |
| `staff-enrolment.md` | procedure, no person names | keep |
| `threat-model.md` | model only | keep |
| `validation-calls-01.md` | Telnyx field names, no values | keep |

## `xtask/ops/` — 15 `ops-*.sh`, `a2-record.py`, `ship-release.sh`, `README.md`

Runbooks with server paths and `ssh plant` / `ssh andrii.ai` targets,
no secrets (gitleaks clean): keep all. Operator call whether they ship
public.

## `ops/plant-pipeline/` — `bootstrap.sh`, `status.sh`, `submit.sh`, `worker.sh`, `README.md`

Build-box scripts, home-directory only, no prod credentials: keep all.

## Still operator-owned (not this list)

LICENSE switch, repo visibility, video link, `/review` approvals, brand
SVGs, A2/A5 re-runs, terms opt-in — see `docs/ops/operator-today.md`.
