# syntax=docker/dockerfile:1
# Judge image: `docker compose up` (root compose.yaml) serves the same
# keyless boot as `cargo xtask judge`, for judges without Rust or Node.
# docs/judges.md has the walkthrough.
#
# Every gate stays on: the server binds 127.0.0.1 inside the container
# (the release binary refuses the dev key and `admin.mtls_off` on any
# other bind), and socat forwards 0.0.0.0:28318/28319 to it, so the
# server only ever sees loopback peers. compose publishes those on the
# host's 127.0.0.1:18318/18319, which keeps the browser's origin
# `http://localhost:18319`, the one staff WebAuthn accepts.
#
#   docker compose up --build          # serve; Ctrl-C stops it
#   docker compose run --rm judge --check   # boot, probe, one worker job, exit

# 1. Both web apps (public site and staff console).
FROM node:24-slim AS web
WORKDIR /src/web
COPY web/package.json web/package-lock.json ./
RUN --mount=type=cache,target=/root/.npm npm ci --no-audit --no-fund
COPY web/ ./
RUN npm run build && npm run build:admin

# 2. The release binaries. `.dockerignore` drops `.git`, so the commit
# comes in as BUILD_SHA (`healthz` reports it; `unknown` otherwise).
FROM rust:1.99.0-bookworm AS rust
ARG BUILD_SHA=unknown
# The image tag is the rust-toolchain.toml pin; this stops rustup from
# fetching that file's extra components and cross target.
ENV RUSTUP_TOOLCHAIN=1.99.0
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    BUILD_SHA="$BUILD_SHA" cargo build --locked --release -p web -p xtask \
 && mkdir -p /out \
 && cp target/release/web target/release/xtask /out/

# 3. Runtime: the repo's data (packs, gold set, recorded responses,
# static files), both SPA builds and the two binaries. No toolchain.
FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends socat ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --create-home --uid 10001 judge \
 && mkdir /app && chown judge:judge /app
WORKDIR /app
COPY --chown=judge:judge . /app
COPY --from=web --chown=judge:judge /src/web/build /app/web/build
COPY --from=web --chown=judge:judge /src/web/build-admin /app/web/build-admin
COPY --from=rust /out/web /out/xtask /app/bin/
COPY --chmod=755 ops/judge/entrypoint.sh /app/bin/entrypoint
USER judge
EXPOSE 28318 28319
ENTRYPOINT ["/app/bin/entrypoint"]
