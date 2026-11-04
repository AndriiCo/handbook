# Web Stack: Local Testing, Serving, Realtime

- Status: implemented. SvelteKit SPA + axum API (decided prompt 005),
  WebSocket-first realtime (decided prompt 006). Section numbers are
  cited across `docs/specs/`; keep them stable.
- Date: 2026-10-26, refreshed 2026-11-04.

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
- **Cargo-only workflow.** `cargo xtask up` boots the handbook on
  loopback with synthetic data; `cargo xtask ci` is the same gate
  everywhere. Node is used only for the SvelteKit build (§4).
- **Cross-built releases.** `cargo xtask setup` builds the exact SHA
  with `cargo-zigbuild` for the server's CPU in a throwaway worktree;
  nothing compiles on the production host (`docs/ops/deploy.md`).
- **Config in the database, not the environment.** Runtime settings
  and secrets live as sealed rows in the store (`docs/specs/config-db.md`,
  `docs/specs/secure-config.md`); only a few bootstrap paths are read
  from the environment. Secrets never live in the repo.
- **Sub-path serving from day one.** Axum `Router::nest("/apostille", …)`
  locally too, with relative asset URLs — so local testing exercises the
  exact production URL shape.
- **TLS terminates in the app.** Since prompt 015: in-app `rustls`
  via `axum-server` (HTTPS on 443 + HTTP→HTTPS 308 on 80, no
  proxy anywhere). Certificates come from Google Public CA via
  in-binary ACME http-01 (see `docs/ops/acme.md`). The old
  Caddy-reverse-proxy plan is retired.
- **Synthetic dev data only.** The repo holds schema, never data
  (`docs/design.md` §0); `cargo xtask up` generates fake
  authorities/captures/tasks into a fresh store at runtime. Nothing
  seeded is ever committed.

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
- Third-party code is vendored or bundled at build time, never loaded
  from a CDN, so production works offline.

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

## 4. UI stack

SvelteKit, built as two static SPAs (public/customer and admin; see
`docs/specs/spa-separation.md`), served by the same axum binary that
serves the JSON and WebSocket API. The SvelteKit build is the one
sanctioned exception to the Rust-only rule. Styling is Tailwind
(decision D38; `docs/site-theme.md`). Rejected at decision time:
htmx + Askama, Leptos, Dioxus and React + Vite.

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
2. `cargo xtask up` serves `localhost:<port>/apostille` (sub-path!)
   with synthetic data; WS live updates visible with two browser
   windows (concurrent-edit banners, presence).
3. Static assets return 304s on revalidation and `content-encoding: br`
   when the client offers it.
4. The plant gate (`ops/plant-pipeline/`) runs the same `ci` on Linux
   before a SHA may ship.
