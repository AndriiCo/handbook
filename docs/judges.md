# Run the Apostille Handbook on your machine

One command, no keys, about 10 minutes the first time.

```sh
git clone https://github.com/AndriiCo/handbook.git && cd handbook
cargo xtask judge
```

You need macOS or Linux (WSL2 is fine), `git`, Rust via
[rustup](https://rustup.rs) (the pinned toolchain installs itself from
`rust-toolchain.toml`), Node 24+ with npm 11+, ports 18318 and 18319
free, and about 6 GB of disk. `cargo xtask judge --doctor` checks all of
that and prints one exact fix per problem; nothing is installed for you.
If those ports are taken, `cargo xtask judge --port-base 28318` serves
the public site on 28318 and the staff console on 28319 instead.

## What it does
1. Builds both web apps (public site and staff console) and the release
   binaries.
2. Creates a fresh local store in `.local/` with a file key (loopback
   only; nothing leaves your machine).
3. Loads every authority pack, ingests synthetic evidence, and drafts
   proposals with recorded Meta model responses (no key needed).
4. Serves the public site on http://localhost:18318/apostille and the
   staff console on http://localhost:18319, and prints a one-time setup
   key plus a loopback-only staff link.

With your own Meta Model API key, `META_API_KEY=… cargo xtask judge
--live` fetches the official pages and drafts through `muse-spark-1.3`
for real. `cargo xtask judge --check` boots, probes the health,
authorities and setup endpoints, and exits (what CI runs).

## Five minutes
1. Public home: press ⌘K and search "Washington".
2. Open the dossier: every fact shows its quote, source and capture date
   one tap away.
3. Staff console: http://localhost:18319 (sidebar: Review, Trace,
   Workers, Config; the badge says `judge`).
4. Review: accept a drafted fact. The public dossier updates live over
   the `authority:{slug}` WebSocket, without a reload.
5. Trace: "Draft from this capture" streams a model run with its tokens
   and latency.

Stop with Ctrl-C. Every `cargo xtask judge` starts fresh.
