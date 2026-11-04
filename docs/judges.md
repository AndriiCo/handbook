# Run the Apostille Handbook on your machine

One command, no keys, about 10 minutes the first time.

```sh
git clone https://github.com/AndriiCo/handbook.git && cd handbook
ops/judge/preflight.sh && cargo xtask judge
```

You need macOS or Linux (WSL2 is fine), `git`, a C toolchain (`cc`,
`make` and `perl`: `xcode-select --install` on macOS,
`sudo apt install build-essential perl` on Debian/Ubuntu), Rust via
[rustup](https://rustup.rs) (the pinned toolchain installs itself from
`rust-toolchain.toml`; a distro `cargo` cannot), Node 24+ with npm 11+
(npm 10 works with a warning), ports 18318 and 18319 free, and about
6 GB of disk.

[`ops/judge/preflight.sh`](../ops/judge/preflight.sh) checks all of that
in about a second, before cargo compiles anything, and prints one exact
fix per problem. `cargo xtask judge --doctor` runs the same checks once
xtask is built. Nothing is installed for you. If those ports are taken,
`ops/judge/preflight.sh --port-base 28318 && cargo xtask judge
--port-base 28318` serves the public site on 28318 and the staff console
on 28319 instead.

## Or in Docker

Only Docker with Compose v2 is needed (no Rust, no Node):

```sh
git clone https://github.com/AndriiCo/handbook.git && cd handbook
docker compose up --build
```

The first build compiles the release binaries inside the image, which
takes 15 to 25 minutes. When the log says `handbook is up`:

- Find the setup key in the log:
  `docker compose logs judge | grep -o 'setup_[0-9a-f]*'`.
- Browse http://localhost:18318/apostille and
  http://localhost:18319/config. Use `localhost`, not `127.0.0.1` or the
  Docker VM's address: staff passkeys only work on the exact origin
  `http://localhost:18319`.
- The ports are published on your machine's loopback only. Inside the
  container the server binds 127.0.0.1 and `socat` forwards to it, so
  the release binary's loopback gates stay on.
- Ctrl-C stops it. There is no volume, so every start is a fresh store
  with a new setup key.

`docker compose run --rm judge --check` boots, probes and exits.
`META_API_KEY=… docker compose run --rm --service-ports judge --live`
drafts with your key. `BUILD_SHA=$(git rev-parse --short HEAD) docker
compose build` stamps the commit into `/healthz`.

## What it does
1. Builds both web apps (public site and staff console) and the release
   binaries.
2. Creates a fresh local store in `.local/` with a file key (loopback
   only; nothing leaves your machine).
3. Loads every authority pack, ingests synthetic evidence, and drafts
   proposals with recorded Meta model responses (no key needed).
4. Seals a fresh worker token into `.local/worker.env` (mode 0600) and
   starts `cargo xtask worker` beside the server, so the staff
   console's Work page shows a connected worker that runs jobs.
5. Serves the public site on http://localhost:18318/apostille and the
   staff console on http://localhost:18319. It prints the one-time
   setup key and says to open `/config` first, then the URL of the
   review queue. That URL is a plain link: it signs nobody in.

With your own Meta Model API key, `META_API_KEY=… cargo xtask judge
--live` fetches the official pages and drafts through `muse-spark-1.3`
for real; without the key, `--live` refuses to start.
`cargo xtask judge --check` boots, probes the health, authorities and
setup endpoints, runs one `scrape.urls` job for the local home page
through the real worker until it is `done`, and exits. The plant build
gate runs it after `cargo xtask ci` on every build
(`ops/plant-pipeline/worker.sh`).

## Five minutes
1. Public home: press ⌘K and search "Washington".
2. Staff console: open http://localhost:18319/config first, paste the
   setup key the command printed (it prints once), and enrol a passkey
   (Chrome and Safari allow passkeys on localhost). The badge says
   `judge`. Later visits sign in at http://localhost:18319/staff-login.
3. Review: every Muse Spark draft shows the quote it came from. Keep
   the Washington dossier open in another tab and accept a few: the
   dossier updates live over the `authority:{slug}` WebSocket, with no
   reload. Nothing is public until a person accepts it.
4. The guide: citations are off by default. Press the Citations button
   or Alt+C (⌥C on a Mac) to number every value's source; hover, focus
   or tap a marker for its quote, source and capture date.
5. Trace: "Draft from this capture" streams a model run with its tokens
   and latency.

Stop with Ctrl-C. Every `cargo xtask judge` starts fresh.
`cargo xtask up` keeps its store: a rerun after you enrolled says
"staff already enrolled" and points to `/staff-login` instead of
minting a new key (`--fresh` starts over).
