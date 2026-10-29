# Coverage gaps (queued work, ranked)

## E2E queue (browser)

1. J2 login/register — needs a `navigator.credentials`
   stub helper (`e2e/helpers/webauthn-stub.mjs`: canned
   create/get responses via init script). Unlocks J2 +
   staff-login-prefixed admin specs.
2. CUJ-2 review decide — needs staff session mock
   (`GET .../admin/me`) + queue fixtures.
3. J3 form submit + J4 label purchase — page-mock specs,
   no new helpers.
4. CUJ-4 call→quote + CUJ-3 workers sweep — stub-transport
   fixtures per voice-telnyx §11.
5. CUJ-5 suppress + CUJ-6 re-key — small review/config
   specs once #2 lands.
6. Full-ceremony E2E (registrations that verify real
   signatures) — needs a CDP virtual authenticator;
   explicitly deferred, handler tests cover crypto.

## Thin server/client rows

- Media upload/redact/transcribe: 1 route test, no node
  client tests. Add route tests with the redact/transcribe
  seams before any moderation UI.
- WS live updates: 1 live route test. Add a Tx/Rx pair test
  per topic when precheck/forms-review grow.
- MCP transport: 2 tests. Extend when tools ship (F12).
- Publish/static build: happy-path only. Add the failure
  matrix (partial write, concurrent build) with F11.
- Labels/media/publish node clients: no JS test files.
  Add mirror tests if those clients gain logic; today
  they are thin fetch wrappers (documented, not urgent).
- Demo drill: no E2E walkthrough. Add when the Viet Nam
  demo path stabilizes.

## Blocked on backends (no E2E until built)

- J5 uploads POST + F8/F9/F10 verdicts/masking/precheck.
- J6 orders/billing endpoints + pricing spec.
- Voice §11 build-out (packs, fix/annotate, playback,
  sweep, backlinks) — spec reviewed, awaiting build.
- F7 Egypt-chain view, F3 notice inbox, B8 task work API.

## Deliberately untested in browser

- Shell steps (`setup-key-mint`, `xtask analyze`,
  KEK rotation): covered by xtask Rust tests instead.
- Provider pages (Google/Apple/Meta consoles, Telnyx
  portal): mocked at our boundary; live behavior confirmed
  by the operator runbooks, not CI.
