# Open-source licences page

`crates/web/static/licenses.html` is the software bill of materials for
the Apostille Handbook, served at <https://andrii.ai/licenses.html> (and
`/licenses`). It lists every third-party component the web app, the
plant workers and the build and release tooling use: version, scope,
licence, copyright notices and a link to the source. Each licence text
appears once below the table.

`cargo xtask licenses` writes it. Never edit the HTML by hand: the next
run overwrites it, and a hand-edited page is never committed over.

## What goes in

| Source | Read from | Rows |
|---|---|---|
| Rust crates | `cargo metadata --locked --filter-platform x86_64-unknown-linux-gnu` (Cargo.lock) | every non-workspace crate the production target builds |
| npm packages | `web/package-lock.json`, plus `web/node_modules/*/package.json` and licence files | everything `npm ci` installs on the release Mac (darwin-arm64) or Linux x64 |
| Toolchains, images, tools | `docs/licenses/extra.toml`, `rust-toolchain.toml`, `.nvmrc`, the Dockerfiles' `FROM` lines | Rust, Node, npm, Zig, Docker base images, distro packages, the plant Python stack, model weights, fonts outside npm |

Scope tags say where a component runs:

- `app`: production (the web server, or the SPA bundle);
- `worker`: xtask (the plant worker, timers, ops);
- `build`: building and shipping (build scripts and proc-macros included);
- `test`: test suites, CI and the rehearsal server only;
- `image`: the judges' Docker image.

## Licence texts

`texts/` holds one `<ID>.txt` per SPDX licence or exception id the BOM
names. `texts/index.toml` holds each one's display name and source.
When a new id appears, the xtask fetches it from `spdx/license-list-data`
at the tag pinned in `xtask/src/licenses/texts.rs`. Hand-written
`LicenseRef-*` texts (the SAM License, the OS-image note, the
proprietary font) are never fetched: add the file and an `index.toml`
entry yourself.

Each package's own LICENSE/COPYING/NOTICE files are compared with the
canonical texts:

- **Same text apart from the copyright lines:** the row links to the
  one canonical text. The copyright lines go in the row's Copyright cell.
- **Different text:** printed once as a variant and linked from every
  package that ships that same text. A variant that holds most of a
  standard licence replaces that licence's link. Anything else, such as
  third-party notice files, shows as "additional terms".
- **NOTICE files:** reproduced in the Notices section, as Apache-2.0
  requires.
- **Short pointer files** ("dual licensed, see LICENSE-MIT"): skipped.

## When it regenerates

```sh
cargo xtask licenses              # regenerate + commit when stale
cargo xtask licenses --force      # regenerate + commit now
cargo xtask licenses --check      # exit 1 when stale; writes nothing
cargo xtask licenses --root <dir> --no-commit
```

The page is **stale** when either of these holds:

- its `bom:generated` stamp is 7 days old;
- its `bom:inputs` hash no longer matches the inputs: the lockfiles,
  `extra.toml`, the toolchain pins, the Dockerfiles and `texts/`.

Both stamps are `<meta>` tags in `<head>` and never show on the page.

Three places run the stale check:

- `cargo xtask ci` runs it last, and commits.
- `cargo xtask setup` staging runs it with `--no-commit` in its
  throwaway worktree.
- `xtask/ops/ship-release.sh` runs it with `--no-commit` in its
  throwaway worktree.

The two ship paths never stage a page older than 7 days, even when the
committed copy lags.

The commit holds only `licenses.html` and `texts/` (`git commit --only`),
so anything else you have staged stays staged. The message is:

```text
licenses: regenerate licenses.html (<date>, <n> packages, <reason>)
```

The pre-commit hook runs as usual. The commit is skipped, with the
reason printed, in any of these cases:

- `--no-commit` was passed;
- the root is not a git checkout (plant's `git archive` extract);
- HEAD is detached (the ship worktrees);
- a merge, rebase or cherry-pick is in progress;
- `licenses.html` has uncommitted edits.

The xtask never pushes.

## Adding something no lockfile pins

Add a `[[package]]` to `extra.toml` with these fields:

- `name`
- `ecosystem`: one of `toolchain`, `apt`, `pip`, `binary`, `model`, `font`
- `version`, or `version_from` naming a pin file
- `url`
- `license`: an SPDX expression
- `scope`: a list of tags
- optional: `copyright`, `note`, and `license_file` (a licence file in
  this repo to match and scan)

A new Docker base image needs an `[[image]]` entry, or generation fails.
The Glimmer weights entry is a commented template until the operator
names the repo.
