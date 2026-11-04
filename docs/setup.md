# Setup guide: local and SSH environments

The handbook runs in two kinds of place. A **local** environment runs on
your machine, needs no accounts or keys, and is what a judge should use.
An **SSH** environment runs on a box you reach over SSH: staging on
`plant`, or a production server.

```sh
cargo xtask setup wizard
```

The wizard opens on a system map with one panel for each part. Its intro
says what runs where, and **All units** runs, stops or removes everything
at once. Start with local; nothing needs a server until you want real
customers.

## Local, in one go

On a fresh clone, with no sops, no age and no keys:

1. Run `cargo xtask setup wizard`. It creates a plain `.local/setup.yaml`
   (0600, git-ignored, local target only) with working defaults.
2. Open the `local` tab and press **Boot local**, or press **Run all** on
   the map. The first boot builds everything, about 10 minutes.
3. When the run finishes, it links the public site and the staff console
   and shows the setup key with a **Copy** button.
4. Open the staff link (`http://localhost:18319/config` unless the ports
   moved), paste the key, and enrol a passkey.

If you only want the site and not the wizard, `cargo xtask judge` does the
same boot and prints a tour ([judges.md](judges.md)).

### The defaults

A new local target needs no edits:

| Setting | Default | What it means |
|---|---|---|
| Ports | public `18318`, admin `18319` | Loopback only; next free pair if taken (below) |
| FedEx and USPS labels | `labels.mode: fake` | Mock labels; nothing ships, nothing is bought |
| Stripe | `stripe.mode: fake` | Checkout works without a key; nothing is charged |
| Telnyx calls | `telnyx.mode: fake` | No real calls |
| Email in and out | `mail.mode`, `mailer.mode: fake` | An outbox in the store instead of sending |
| Hugging Face | `hf.mode: fake` | Recorded verdicts; no weights downloaded |
| Orders | `orders.enabled: true` | Paid tiers are on against the fakes |
| Live switches | `labels.live`, `labels.customer_live`, `usps.live`, `tracking.poll_enabled`, `acme.enabled`: off | Nothing live is bought, polled or certified |
| Meta Model API | unset | `up` replays the recorded answers; `--live` with `META_API_KEY` drafts for real |
| Staff access | `admin.allow_ips` unset | Loopback only, which is what a local console needs |

A local boot also ignores values that would break it. `net.bind`, the
ports, `auth.rp_id` and `auth.origin` are owned by `up`; a shared
`net.bind6` or `admin.allow_ips` is dropped. On an older file, **Fill local-only defaults
(fakes)** on the `local` tab adds whatever is missing and keeps what you
set.

### Ports

Before it builds anything or mints a key, a local boot checks that both
ports are free on `127.0.0.1` and `::1`.

- **Taken by something else:** it moves to the next free pair (`18328` and
  `18329`, then `18338`, ...), says which process held the ports, and the
  wizard saves the new pair on the target so its links follow.
- **Taken by this checkout's own server:** it stops and tells you to stop
  that server first, because a second server would share its store.
- **`--port-base N`:** you asked for those ports, so it stops and names
  the holder instead of moving.

The Docker runtime is the exception. `compose.yaml` publishes `18318` and
`18319` as fixed ports, and `docker compose` fails if they are taken.

### The setup key

The key opens the first staff enrolment. A rerun prints a new key and the
old one stops working, until someone enrols a passkey; after that a rerun
prints no key and points to `/staff-login`. A failed attempt costs nothing:
boot again and use the newest key. A wizard's boot prints it in the Run
tab; `docker compose logs judge` has it for the Docker runtime.

### Native or Docker

The map's front-end panel switches the local target between the two.

| | Native | Docker |
|---|---|---|
| Needs | Rust, Node 24, a C toolchain | Docker with Compose v2 |
| First start | about 10 minutes | 15 to 25 minutes |
| Store | `.local/store`, kept between runs | none: every start is fresh |
| Stop | **Stop all**, or Ctrl-C in the wizard's terminal | **Stop all**, or `docker compose stop` |

### What needs a server

These work on localhost: the handbook and search, the staff console and
review queue, drafting with a key, passkeys for this browser, email-link
sign-in through the fake outbox, and orders against the fakes.

These need a host on the internet:

- Customer passkeys on your domain. A passkey is bound to the site's host
  name, so one made on `localhost` is useless anywhere else.
- Sign in with Google and Apple, which return to an https address you
  registered.
- Stripe and Telnyx callbacks: webhooks, call events, browser calls.
- Real email delivery, live FedEx and USPS labels, TLS certificates.

## SSH environments

These need sops and age: secrets for a machine other than yours are never
kept in plaintext. With them installed, `cargo xtask setup` keeps everything
in the encrypted, git-ignored `private/config.yaml`, created from a template
with all four targets. `--host <name>` adds a production server.

| | Staging (`plant`) | Production (`server`) |
|---|---|---|
| Reach | `ssh` alias, an unprivileged user, never root | `ssh` alias as `hal`; a separate `ssh_admin` with sudo only to provision |
| What a run does | Pushes the SHA, builds there, seals config, restarts by verified PID | Stamp, probe, DNS, build, provision, seal, deploy, ACME, scoreboard |
| Who may run it | Anyone with the alias | Only with an operator order (`--operator-order "<who, when, what>"`, or the wizard's field), logged in `private/setup-log.jsonl` |
| Open it | `ssh -N -L 18318:127.0.0.1:18318 -L 18319:127.0.0.1:18319 plant` | `https://<host>/apostille` and `https://admin.<host>/` |
| Staff access | Through the tunnel | From an address in `admin.allow_ips`; set it before opening `/config` |
| **Stop all**, **Remove all** | Stops `web`; removes its store and KEK, keeps the checkout | **Never**: a release goes back with the rollback tools |

The full release path and its rails are in [ops/deploy.md](ops/deploy.md).

A production run merges the setup file into the server's config, including
the values the file computes from `public_host`. If the server was set up
differently, pin those values on the target first, and check with:

```sh
cargo xtask setup pull
```

It saves what the server holds beside the setup file and lists every key
that differs. `--secrets --reason "<who, why>"` also reads secrets for a
backup or rebuild: audited on the server, written sops-encrypted, and
command-line only.

## Troubleshooting

**`{"code":"admin_host_required", ... "ip":"127.0.0.1"}` on `/config`.** You
reached a server without the local dev seam, usually staging through an SSH
tunnel holding the same port. It is not an allowlist problem, and your setup
key is still good for the local server. See who holds the port:

```sh
lsof -nP -iTCP:18319 -sTCP:LISTEN
```

If it is `ssh`, use the local ports the boot printed, or close the tunnel.
Before this check existed, a local boot could not bind and mistook the
tunnel's `/healthz` for its own, so it reported success.

**`ip_not_allowed` on a server.** Add your address to `admin.allow_ips` as
the page says, then restart the service.

**A passkey fails on `127.0.0.1`.** Use `localhost`: staff passkeys are bound
to the exact origin `http://localhost:<admin port>`.

**`this checkout's handbook already listens`.** Press **Stop all**, or Ctrl-C
in the terminal that started it. `cargo xtask up --fresh` empties `.local/`
apart from the plain setup file.
