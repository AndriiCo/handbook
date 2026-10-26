# Web Stack: Local Testing, Serving, Realtime, UI Options

- Status: accepted (UI stack picked in prompt 005: SvelteKit SPA + axum API;
  WS-first realtime decided in prompt 006)
- Date: 2026-10-26

Priorities for this track: **P1 (mandatory)** — transcripts become inputs
for handbook pages (already in `docs/design.md` §5), including a live call
console showing the transcript and follow-up questions *during* the call;
**P3 (nice to have)** — live two-way voice translation (§5).

## 1. Run anywhere: Mac locally, Ubuntu on the server

Local dev runs natively on macOS (Apple Silicon), no Docker. Production is
one Ubuntu **26.04 LTS** ("Resolute Raccoon") server, app served under
`andrii.ai/apostille`. Parity rules:

- **Pure-Rust dependencies only.** `rusqlite` bundled (no system SQLite),
  `rustls` (no OpenSSL), nothing needing brew/apt system libraries or
  codegen beyond `cc`. What compiles on the Mac compiles on Ubuntu.
- **Cargo-only workflow.** `cargo xtask dev` runs the server;
  `cargo xtask ci` is the same gate everywhere. No compose files, no
  containers, no Node unless the picked UI stack needs a build step (§4).
- **Build on the server.** `git pull && cargo build --release` on Ubuntu —
  no cross-compilation, no musl wrestling. The deploy unit is one binary
  plus the `store/` directory.
- **Config by environment:** `STORE_PATH`, `BIND_ADDR`, `PORT`,
  `BASE_PATH=/apostille`, `META_API_KEY`, … Secrets never in the repo.
- **Sub-path serving from day one.** Axum `Router::nest("/apostille", …)`
  locally too, with relative asset URLs — so local testing exercises the
  exact production URL shape.
- **TLS terminates outside the app.** Caddy as a reverse proxy on Ubuntu
  (auto-HTTPS, one static binary, ~5 lines of config); the app stays plain
  HTTP both locally and in prod. (Alternative: in-app `rustls` via
  `axum-server` — kept as fallback, not the plan.)
- **Synthetic dev data only.** The repo holds schema, never data
  (`docs/design.md` §0), so `xtask dev-seed` will generate fake
  authorities/captures/tasks into a tmp store at runtime. Nothing seeded
  is ever committed.

## 2. Serving crates (app + static assets)

One axum server does both:

- **App/API:** `axum` 0.8 on `tokio`/`hyper`, graceful shutdown via
  `axum-server` (or plain tokio signal handling — decide at scaffolding).
- **Static assets:** `tower-http` `ServeDir`/`ServeFile`, which already
  give ETag + Last-Modified generation, `If-None-Match` /
  `If-Modified-Since` conditional responses (304s), and `Range` support.
- **Compression:** `tower-http` `CompressionLayer` (Brotli/zstd/gzip
  content negotiation), plus `ServeDir::precompressed_*` for build-time
  `.br`/`.gz` files so hot assets never compress per-request.
- **Caching:** hashed filenames + `Cache-Control: public, max-age=31536000,
  immutable` for versioned assets; `no-cache` for HTML entry points —
  set via `tower-http` `SetHeader`.
- UI templates (if server-rendered, §4 option A) compile into the binary
  (`askama`); third-party JS (e.g. htmx) is vendored as a static file, not
  a CDN dependency, so prod works offline.

No separate asset server, no CDN needed at this scale.

## 3. Live updates (stack-agnostic core)

**WebSocket-first** (decided prompt 006, replacing SSE-first).
Rationale: the call console is genuinely bidirectional — transcript and
questions flow down while operator input, presence, and (P3) translation
control flow up — so one persistent connection per client handles both,
and axum serves WebSockets natively next to the API routes. SSE stays a
documented fallback for networks that block WS upgrades; re-add only if
needed.

- **Event bus:** `tokio::sync::broadcast` per topic
  (`authority:{slug}`, `call:{id}`, `submission:{id}`,
  `shipment:{id}`, `billing:{customer_ref}` — frozen set, F10
  notify topic named at scaffolding), fanning out JSON envelopes
  (`{type, …}`) to WebSocket subscribers (`axum::extract::WebSocketUpgrade`,
  one send/recv task pair per connection). Anonymous subscribers
  receive accepted facts only; proposed records stay on
  staff-authorized topics.
- **Concurrent edits:** last-write-wins + mandatory `audit` row + live
  "updated by X" events; presence ("X is viewing/editing") falls out of
  tracked WS connections, no heartbeat endpoint needed.
  No CRDT (`yrs` revisit only if true collaborative text editing is ever
  needed — overkill today).
- **Call console (P1):** transcription chunks (streamed from the Meta
  Models API when it supports streaming, else polled + appended) go to
  the `call:{id}` topic; server-generated follow-up questions ride the
  same socket, so the human sees both mid-call and can ask more before
  hanging up; operator messages (flag a turn, request re-transcription)
  ride back up the same connection. Transcripts persist as
  `capture(kind='transcript')` and feed `proposed` facts exactly as
  `docs/design.md` §5 describes.

## 4. UI stack options (C chosen — SvelteKit SPA + axum API)

| | A. htmx + Askama + WS | B. Leptos full-stack | C. SvelteKit SPA + axum API |
|---|---|---|---|
| Language | Rust templates, ~zero JS | All Rust (SSR + WASM) | TypeScript UI, Rust API |
| Live updates | WS via htmx ws extension | Signals + server fns | WS client, easy |
| Build | `cargo build`, single binary | `cargo-leptos` (WASM step) | Node build + Rust binary |
| Ship speed | Fastest | Medium (young ecosystem) | Medium (two toolchains) |
| Fit | Simplest thing that works | Modern all-Rust reactive | Best JS DX + ecosystem |

- **A fits** the simplicity rule and §3 best: server-rendered pages,
  htmx swaps on WS events, everything in one binary.
- **B fits** if all-Rust reactivity (fine-grained signals, shared types
  end to end) is worth the WASM build step and slower compiles.
- **C fits** if mainstream JS DX and hiring pool beat the Rust-only rule
  for UI code (API stays Rust regardless).
- Not shortlisted: Dioxus (less mature than Leptos for web), React+Vite
  (heavier than SvelteKit with no compensating win here).

## 5. Live translation (P3)

The Meta model for this job is **SeamlessStreaming** (built on
[SeamlessM4T v2](https://ai.meta.com/research/seamless-communication/)):
streaming speech-to-text and speech-to-speech translation for ~100 input
languages (≈36 speech outputs, Spanish included) at roughly two seconds of
latency — i.e. translation while the speaker is still talking, both
directions on a Mexico call. Decided prompt 008: Seamless is
**absent** from the Model API (see `docs/meta-api.md`), so P3 stays
conditional — nothing in P1 depends on it.

Integration shape (reserved, not built): call audio fans out to the
translation stream alongside transcription; translated turns persist as a
second `capture(kind='transcript')` (language-tagged in `authenticity`
JSON) linked to the same `call_session`; the call console shows both and
lets the human toggle. P3-lite fallback if streaming is unavailable:
batch-translate completed turns after the call.

## 6. Local-testing checklist (before any deploy)

1. `cargo xtask ci` green on the Mac.
2. `cargo xtask dev` serves `localhost:$PORT/apostille` (sub-path!).
3. `xtask dev-seed` populates a tmp store; WS live updates visible with
   two browser windows (concurrent-edit banners, presence).
4. Static assets return 304s on revalidation and `content-encoding: br`
   when the client offers it (`curl -v` spot-checks, scripted as an
   `xtask` check later).
5. Same three commands pass on a fresh Ubuntu 26.04 shell (rustup +
   `git clone` only — no system packages).

## 7. Open decisions

1. UI stack: A, B, or C (§4) — **decided in prompt 005: C, SvelteKit SPA
   + axum API**. Consequence: the UI gets a Node build step (the one
   sanctioned exception to the Rust-only rule); axum serves the built
   static assets plus the JSON/WS API from the same binary.
2. ~~Meta Models API spec~~ — decided prompt 008 (see
   `docs/meta-api.md`); residual open: exact JSON shapes at
   implementation, data-handling terms read before real media.
3. `axum-server` vs plain tokio graceful shutdown (minor, at scaffolding).
