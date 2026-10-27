# Inputs-readiness report — 2026-10-27 (parent-filed)

- Base: 7535194. Agent: read-only child. Parent annotations: B7 partially stale — packs-load HAS run on staging (15 authorities, 57 tasks, 14 captures) but domain verdicts remain unverified pending staff review; ci green at parent HEAD 206/206 (child did not execute).

---

# Inputs-readiness report — handbook @ 7535194 (note: actual HEAD is `0f7c193`, one commit ahead; 7535194 is its parent)

Scope verified by reading bodies: `docs/specs/{roadmap,analysis-agent,app-features,TOPICS,README}.md`, `docs/{meta-api,auth-playbook}.md`, `packs/*`, `crates/{handbook-agent,ingest,auth,web,media}/src/*`, `xtask/src/*`. No files written; nothing executed.

## Blocker table (fetch → analyze → approve → publish)

| # | Item | Owner | Unblock action | Timebox |
|---|---|---|---|---|
| B1 | Live model unwired: `MetaRestClient::draft_records` returns honest `Transport("live calls not implemented")` (`crates/handbook-agent/src/model.rs:92`); `xtask analyze` is Stub-only and nothing reads `META_API_KEY` into it (`xtask/src/main.rs:293`) | S2 / CODE | Implement REST POST per `docs/meta-api.md` (base `https://api.meta.ai/v1`, Bearer key), key via sealed config or env, plus `--check` probe; pin JSON shapes | 0.5–1 d |
| B2 | Key + terms gate open: `META_API_KEY` not provisioned anywhere in repo; `MEDIA_TERMS_READ` unset so real uploads fail closed (`crates/web/src/main.rs:445`, `admin.rs:524`); Meta data-handling terms unread (roadmap §5, G3) | author (key) + SEC (terms) | Author supplies key into sealed config; SEC reads terms, then sets gate | same-day author action |
| B3 | No staff login: only customer demo login exists (`POST /api/v1/demo/login`, no password, `main.rs:211`); no staff/admin login endpoints — "sessions are programmatic" (prompt 022 follow-ups); staff method still open in `docs/auth-playbook.md:9` | author (decide) + admin-track CODE | Decide staff method (playbook recommends passkey or password+TOTP + mTLS); interim: ship a programmatic staff-session mint (xtask) so approval can be worked | decision 2 h; interim 0.5 d |
| B4 | Strict admin mTLS fails closed in prod: no layer inserts `VerifiedClientCert`, so strict mode 403s every admin call (`crates/web/src/admin.rs:27`); approval API sits behind `AdminAuth` = session + cert | admin-track CODE | Land rustls client-cert verifier, or document `ADMIN_MTLS=off` + compensating controls as the slice posture | 0.5–1 d |
| B5 | Watch has no sweep: `fetch-url` takes one manual `--url` (required, `xtask/src/main.rs:447`); no enumerate-`source`-by-cadence mode, no HCCH poll task, no timer — the packs-README "thin-watch seam" is a contract for code not yet written | S2/S6 CODE or operator | Add cadence-enumeration mode + HCCH poll writing status/routing tasks; cron/systemd timer | 0.5 d |
| B6 | fetch→analyze seam broken: `analyze` takes `--input` text, not a capture; every drafted fact lands with `capture_id = 0` (`UNATTRIBUTED_CAPTURE_ID`, `runner.rs:35,136`); no sidecar→analyze wiring | S2 CODE | Add `--capture-id`/`--from-watch` mode reading `text/<id>.txt` | 0.5 d |
| B7 | Seed unverified: all 12 pack domain verdicts are `unverified`; HCCH URI is researcher-observed, pack notes "OUR staff capture pending" (`packs/hcch.json:16`); pack README mandates research traffic from prod IPs only | author/operator + SPEC | `packs-load` on staging/prod store, staff fetch from prod IPs, flip verdicts with evidence captures | 0.5 d |
| B8 | Conflict-triage staffing path missing: `task` rows exist with dedup, but no task list/work API or UI in `crates/web` (zero task routes); `review-conflicts` only prints; no named triage owner/rotation | SPEC + admin-track CODE | Min: `xtask task-list`/`task-close` or admin API; name the human triage owner | 0.5 d min |
| B9 | WS `submission:*`/`shipment:*` ownership seam: `ws_bus.rs:88` passes `NoOwners`, so customers fail closed and only staff subscribe; S6 P2 tables + real `OwnerLookup` missing | S6 CODE | P2 store + lookup impl, or accept staff-only for the slice | 1 d (or slice-accept) |
| B10 | HCCH flip has no writer: `derive_routing` is a pure fn (`hcch.rs:43`); nothing creates the spec'd high-priority F4 records/tasks on member/effective-date change | S2 CODE | Fold into B5's HCCH poll | incl. in B5 |

Slice-accepted (not blockers, per roadmap): WARC plain-blob first (marked in `capture.notes`, `capture.rs:16`); naive tag-strip extraction (F5 later); EN-only + unknown-skip (`runner.rs:92`); banner-not-block conflicts.

## Already green

- **Topics frozen**: 25-enum `TOPICS.md` + `contracts::topics::Topic` + migration CHECK; `po_box` own topic decided.
- **Agent core**: Stub deterministic, byte-exact citation validator, per-record reject counting, watermark + audit atomic with facts, skip-path task dedup — all with unit tests (`runner.rs`, `analyze.rs`).
- **Ingest**: sha256 dedup, blob store, sidecar temp+rename, FTS row, audit row, fetch-failure → `validate_fact` task (never silent), robots gate for third-party hosts (`fetch.rs`, `capture.rs`, `run_fetch_url`).
- **Approve + conflicts**: staff list/accept/reject with audit, conflict banner on API views, `review-conflicts` CLI, banner policy in code (`approval.rs`, `publish::conflicts`).
- **Publish**: accepted-only `build-handbook`, atomic swap, `build` rows with `build_no`/`facts_hash`, sitemap/robots merge.
- **Packs (F13)**: validate/load with `needs_review`-only invariant, never-writes-facts guarantee, `40220`/`40234` fixture ban; `hcch.json` carries the ONE HCCH URI with daily cadence; Viet Nam routing regression test green (`hcch.rs` tests).
- **Auth skeleton**: roles/sessions/CSRF/Origin/WS-topic authz tested (`auth` crate); admin listener split with host guard; mTLS cert registry xtask; encrypted config envelope; secrets-scan first in `ci`.

## Gaps (could not verify, read-only)

- `cargo xtask ci` green at 7535194 (did not execute; roadmap Day-1 note claims green on an earlier tree).
- Prompt 022's staging-deploy/smoke claims and current live server state.
- Whether `packs-load` has been run against any real store.
- Whether `META_API_KEY` exists out-of-band (operator-held).
- Exact Meta REST JSON shapes — `docs/meta-api.md` defers pinning to implementation; unpinned.

**Top finding:** the loop's code skeleton is complete but no real input can flow yet — the three hard gates are B1+B2 (live model + key/terms), B3+B4 (staff identity to work approvals), and B5+B6 (watch sweep + capture-attributed analysis).