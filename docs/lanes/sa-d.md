# SA-D "release-train" — lane status

Scope: `xtask/ops/ship-release.sh`, `xtask/ops/ops-deploy.sh`,
`xtask/ops/promote.sh` (new), `xtask/ops/release-train.sh` (new),
`xtask/ops/staging-accept.sh` (new), `xtask/ops/ship-when-green.sh`
(new), `ops/plant-pipeline/worker.sh`, this file. Nothing else
touched. `xtask/src/release.rs` already exists, so per spec no Rust
was added or changed in this lane.

Spec: Appendix J D1–D5, base `9b23489` (P290, contains P287
root-probe fix; eb57cdc ship+deploy preserved — all ship rails kept:
stamp, schema/migrations, unit file, prod-before, root-process
probe, prune; only the day gate left).

## Done

- D1: day gate removed from `ship-release.sh` (`check_day` + both
  calls gone). `--allow-today-overlap` still parses and is announced
  as an accepted no-op, so old orders and wrappers keep working.
- D2: sleeps replaced with bounded `/healthz` polling with clear
  timeout messages everywhere this lane owns:
  - `ship-release.sh`: new `poll_healthz` helper; rails live-version
    read polls 12×5 s instead of a single curl.
  - `ops-deploy.sh` step 3: stamp verify polls up to 18×5 s
    (`VERIFY_TRIES`/`VERIFY_INTERVAL` override), prints the last
    body on timeout plus the orphan-proc remedy.
  - All four new scripts poll; none contain a bare `sleep`.
  - Remainder (builder-owned): three `SETTLE_SECS` sleeps in
    `xtask/src/release.rs` — exact replacement snippet below.
- D3: `promote.sh` (Mac-relayed; plant never touches production):
  scp artifact from plant → verify SHA256SUMS (`shasum -c`) →
  BUILD_SHA names version → max required GLIBC ≤ `--max-glibc`
  (required flag, prod value never guessed) → build-info identity →
  relay to `andrii.ai:apostille/staging/<v7>/` + SHA file. Ends by
  printing the train invocation; never deploys.
- D3 plant side: `worker.sh` package stage now writes `glibc.txt`
  (`ldd` first line, `no-ldd` fallback), adds `glibc` to
  `build-info.json`, and writes `SHA256SUMS` over
  `web xtask build-info.json glibc.txt`.
- D4: `release-train.sh`: reads PREV live version first (refuses if
  already live) → parallel prod-before (scoreboard `--out
  prod-before.json` + plant stamp + live pin via `&`/`wait`; any
  failure blocks the deploy) → `ops-deploy.sh` over ssh → post-verify
  (healthz pin + `prod-after.json`, regression = rows green before
  and red after, compared with node) → on regression AUTOMATIC
  `xtask rollback --to PREV` with the previous release's own xtask,
  then re-poll of the PREV pin. Exit 3 after a rollback; exit 4 +
  operator-needed ABORT if the rollback itself fails.
- D5: `staging-accept.sh` (tunnel check with the exact ssh command
  on failure, exact-SHA pin, full scoreboard `--out
  staging-accept.json`, live demo take `DEMO_BASE_URL=<base> npm run
  demo`, verdict `staging-green` iff 9/10 with only S1 red + demo
  3/3) and `ship-when-green.sh` (bounded plant-stamp poll, then
  ship-release.sh, then release-train.sh; `--skip-accept` correctly
  routed to the train only).

## Verification

`bash -n` clean on all 7 scripts (shellcheck not installed on this
box). No Rust changed → no cargo gates apply. Dry runs below use
`DRY_RUN=1` (remote ops echoed; all checks/verdicts run for real
against local fixtures). Happy path + one abort path per new script:

`promote.sh` happy (exit 0): SHA256SUMS `web/xtask/build-info.json/
glibc.txt: OK`, `build-sha OK`, `max required GLIBC: 2.34 (cap
2.39)`, `identity OK`, `RELAYED <v7>`. Abort paths (exit 1 each):
`sha256` → `ABORT: SHA256SUMS mismatch ... (nothing relayed)`;
`buildsha` → `ABORT: artifact web does not name BUILD_SHA=...`;
`glibc` → `ABORT: artifact needs GLIBC 99.0 > prod cap 2.39`;
`identity` → `ABORT: artifact build-info sha deadbeef... != ...`.

`release-train.sh` happy (exit 0): three parallel before-checks,
deploy echoed, fixture prod-after 9/10, `TRAIN GREEN: <v7> live, no
regressions vs prod-before`. Abort (exit 3): fixture prod-after 8/10
→ `REGRESSION: GP1 — automatic rollback to deadbee` + rollback ssh
line + `DRY-RUN: ROLLED-BACK ...`.

`staging-accept.sh` happy (exit 0): `staging-green: 9/10 (only
operator-owned S1 red) + demo 3/3`. Abort (exit 1): fixture GP1 red
→ `ABORT: not staging-green — red rows: GP1,S1`. (Dry run also
caught and fixed a real bug: missing `mkdir -p web/shots`.)

`ship-when-green.sh` happy (exit 0): `stamp OK (try 2/3)`, stage +
train echoed. Abort (exit 1): `ABORT: no plant stamp with ci_green
... after 3 tries` + submit remedy.

Edited scripts: `ship-release.sh deadbeef` → `ABORT: not a commit`;
`--bogus` → `ABORT: unknown flag`; grep confirms no `check_day` /
`SPENT-TODAY` / `WITHIN-8H` remains. Worker package addition
simulated in isolation (SHA256SUMS verifies, glibc record written).

## Staging rollback DRILL (builder executes, staging-safe)

Drill dir `$T` is a throwaway Mac dir; nothing touches prod or
staging. Needs the one-line `run_rollback` flag below first.

```sh
export CARGO_TARGET_DIR=/Users/andrii/Code/handbook/target
T=$(mktemp -d); P=18923
mkdir -p $T/staging/{spa,spa-admin,static}
printf '\177ELF-drill-a' > $T/staging/web
printf '\177ELF-drill-a' > $T/staging/xtask
echo ok > $T/staging/spa/index.html
echo ok > $T/staging/spa-admin/index.html
echo ok > $T/staging/static/robots.txt
cat > $T/stub.py <<'EOF'
import http.server, sys
ver = sys.argv[2]
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body = ('{"status":"ok","version":"0.1.0+%s"}' % ver).encode()
        self.send_response(200); self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body))); self.end_headers()
        self.wfile.write(body)
    def log_message(self, *a): pass
http.server.HTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
EOF
python3 $T/stub.py $P aaaaaaa & STUB=$!
cargo run -q -p xtask -- deploy --version aaaaaaa --sha aaaaaaa000000000000000000000000000000000 --src $T/staging --root $T --store $T/store --skip-packs --skip-accept --no-restart --smoke-url http://127.0.0.1:$P/apostille/healthz
# expected: "deployed aaaaaaa (previous: none, pruned: none)"
kill $STUB; python3 $T/stub.py $P bbbbbbb & STUB=$!
cargo run -q -p xtask -- deploy --version bbbbbbb --sha bbbbbbb000000000000000000000000000000000 --src $T/staging --root $T --store $T/store --skip-packs --skip-accept --no-restart --smoke-url http://127.0.0.1:$P/apostille/healthz
# expected: "deployed bbbbbbb (previous: aaaaaaa, pruned: none)"
# rollback drill (the train's automatic step, spelled manually):
kill $STUB; python3 $T/stub.py $P aaaaaaa & STUB=$!
cargo run -q -p xtask -- rollback --to aaaaaaa --root $T --store $T/store --no-restart --smoke-url http://127.0.0.1:$P/apostille/healthz
# expected: "rolled back bbbbbbb -> aaaaaaa"
# failure drill (auto-revert primitive): stub serves a wrong version, deploy C:
kill $STUB; python3 $T/stub.py $P 0000000 & STUB=$!
cargo run -q -p xtask -- deploy --version ccccccc --sha ccccccc000000000000000000000000000000000 --src $T/staging --root $T --store $T/store --skip-packs --skip-accept --no-restart --smoke-url http://127.0.0.1:$P/apostille/healthz
# expected: failure naming ccccccc plus "revert: bbbbbbb live again"
kill $STUB; rm -rf $T
```

## Builder-owned snippets (exact)

1. `xtask/src/main.rs` + `release.rs` — drill needs a restart-free
   rollback. In `run_rollback`, change:
   `restart: true,` → `restart: !has(args, "no-restart"),`
   and document `--no-restart` under the `rollback flags:` help text
   next to `--smoke-url`.
2. D2 remainder — replace the three `std::thread::sleep(
   std::time::Duration::from_secs(SETTLE_SECS))` calls in
   `release.rs` (deploy, revert_to_prev, rollback) with polling.
   Add once:
   ```rust
   /// Poll `url` until it answers 200 with `"status":"ok"` (and, when
   /// `sha7` is given, `"version":"0.1.0+{sha7}"`), or fail with a
   /// clear timeout. Replaces the fixed SETTLE_SECS sleeps (D2).
   pub fn wait_healthy(url: &str, sha7: Option<&str>, tries: u32, interval_secs: u64) -> Result<(), String> {
       for i in 1..=tries {
           let body = std::process::Command::new("curl")
               .args(["-fsS", "--max-time", "15", url])
               .output()
               .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
               .unwrap_or_default();
           let compact: String = body.split_whitespace().collect();
           let ok = compact.contains(r#""status":"ok""#)
               && sha7.is_none_or(|v| compact.contains(&format!(r#""version":"0.1.0+{v}""#)));
           if ok {
               return Ok(());
           }
           if i < tries {
               std::thread::sleep(std::time::Duration::from_secs(interval_secs));
           }
       }
       Err(format!("{url} not healthy after {tries} tries ({interval_secs}s apart)"))
   }
   ```
   Call sites: deploy → `wait_healthy(&opts.smoke_url,
   Some(&opts.version), 18, 5)?;` after `systemctl("restart")?;`
   revert_to_prev → `wait_healthy(smoke_url, None, 18, 5)` in place
   of the sleep + `smoke()` match arms (keep the same `revert: ...`
   log lines); rollback → `wait_healthy(&opts.smoke_url,
   Some(&opts.to), 18, 5)?;`. `SETTLE_SECS` can then be deleted.
   No new main.rs arms needed for this lane (all new surface is
   shell; no config keys added).

## Unresolved

- drill pending builder (commands + expected output above; needs the
  one-line `--no-restart` rollback flag first).
- Live staging/plant proof not run from here (shared services are
  builder-restarted; this lane only added polling clients of them).
