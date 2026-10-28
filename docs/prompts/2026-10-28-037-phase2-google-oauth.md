# 2026-10-28-037-phase2-google-oauth

- Prompt (UTC): 2026-10-28T03:10:00Z (approx)
- Response (UTC): 2026-10-28T04:25:00Z (approx)
- Author: Andrii
- Agent: Muse Code

## Prompt

```text
Resume work towards the goal. Use sub-agents as approoptiate (add this instruction to AGENTS.md).
```

## Response

Two commits. First: standing "Subagents as appropriate" rule
added to AGENTS.md §6 (`6ce9ad3`, separate commit per the
living-document rule). Second: Phase 2 (Google OAuth) designed
via 4 read-only workflow streams (spec/backend/UI/guide), all
complete with zero unresolved; drafts arrived truncated
(`textChars: 0`) and were recovered in full from
`session.jsonl` `subagent_control` records to `/tmp/phase2-*.txt`
per the §6 truncation-proof rule. Parent reconciled conflicts
(flat `/auth/google/*` paths over the spec draft's `/oauth/`
segment, 3:1 + playbook-consistent; spec's split binding cookie
over the backend draft's state-echo; fresh-session unlink proof
for Google-only users; graceful-503 instead of a providers
endpoint) and wrote everything as single writer.

Implementation (`9c28dd9`, 21 files, +2484/-30): spec §9 +
decision 32 + OPEN-4..8; `google.client_id`/`google.client_secret`
keys (registry 34→36, boot tolerates absence); new
`auth::oauth` (state reuse of `webauthn_challenge`,
token-exchange + tokeninfo verify over a stubbable
`HttpTransport`, rule-5 resolve) + generic `disable_link`;
new `web::oauth` handlers (start JSON / callback 302s, ureq +
aws-lc-rs, `spawn_blocking`) + unlink extension + main wiring;
login/security UI (`google.js`, button, link/unlink,
`?error=` copy) + `google.test.mjs`; new `config-values.md`
with the Google section.

Verification performed (all observed):

- `cargo xtask ci` exit 0, 29 suites, zero warnings; web 118
  passed (+10 oauth incl. port-0 stub full-loop/login/link/
  rule-5/aud-mismatch, +1 google unlink), auth 39 (+11),
  config keys/overlays/counts; node 42/42; svelte-check 0/0;
  `check:imports`, both builds, smoke OK; packs clean;
  `cargo tree -i ring` still rcgen-only (lockfile edges only,
  zero new crates).
- `web-budget` green at 225234/225280 — 46 bytes of headroom
  left (trims: dropped `$app/state` for `window` in effects,
  merged busy flags, unified methods row, dropped the
  redundant `?linked=` notice + param, shortened copy).
- Fixes along the way: stub consumed request bodies (RST
  flake), `from_ref` clippy lints, while-let lint, TS index
  signature, `SecretText::from` constructor.
- Deploy `9c28dd9` as hal via `ops-deploy.sh --skip-accept`
  (announced: no packs/migration surface, prod still has no
  build row): FIRST fully-hal end-to-end ship. Live proof:
  `healthz` `0.1.0+9c28dd9`, POST start → 503
  `google login unavailable`, GET start → 405, GET callback →
  302 `.../login?error=oauth_unavailable`, journal clean.
- Environment notes: `zig` was missing (installed via brew
  mid-turn after finding a concurrent install holding the
  lock); `echo -n` wrote a literal `-n` into SHA (macOS) —
  fixed with `printf`, 40-hex verified; scp `-r` landed the
  dir as `staging-9c28dd9`, renamed server-side.

Staging: `/home/hal/apostille/staging/9c28dd9/` (ELF-verified
x86-64); prod `current` → `releases/9c28dd9`, f5d199f kept.

## Commits

- `6ce9ad3` — agents: standing rule — use subagents as
  appropriate
- `9c28dd9` — Phase 2: Google OAuth login + linking
  (this entry committed separately; see log commit)

## Follow-ups

- Budget nearly exhausted (46B): next UI addition needs a
  budget decision (raise gate or code-split).
- Legacy `(root) apostille-ship` grant still present — remove
  now that hal end-to-end is proven (needs root).
- Operator-side unchanged: Telnyx keys + 8am proof call, www
  3rd SAN script, Google console client for live-ceremony
  proof (OPEN-1 covers passkey; Google needs its own run).
- Next code: Phase 3 (Apple + Meta OAuth).
