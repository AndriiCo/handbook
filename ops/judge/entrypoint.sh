#!/bin/sh
# Judge image entrypoint (ops/judge/Dockerfile). The server binds
# 127.0.0.1:18318 (public) and :18319 (admin) inside the container;
# socat forwards the published 28318/28319 to them, so every peer the
# server sees is loopback and no gate is loosened. compose maps the
# host's 127.0.0.1:18318/18319 onto 28318/28319.
#
# Arguments go to `xtask judge`: `--check` boots, probes, runs one
# worker job and exits; `--live` drafts with META_API_KEY.
set -eu
socat TCP-LISTEN:28318,fork,reuseaddr TCP:127.0.0.1:18318 &
socat TCP-LISTEN:28319,fork,reuseaddr TCP:127.0.0.1:18319 &
cd /app
exec /app/bin/xtask judge --prebuilt "$@"
