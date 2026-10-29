# Prompt 072 — telemetry options and file-local OpenTelemetry

- Prompt timestamp (UTC): 2026-10-29T07:15:00Z
- Response timestamp (UTC): 2026-10-29T07:16:00Z
- Author: Andrii
- Agent/model: Muse Code (Muse Spark)

## Prompt

What other telemetry options we can add to the app? Is OpenTelemetry applicable here and can it be used with file-based backend instead of separate server at targer?  Will logs from this recorder be useful for debugging issues like this one?

## Response

Answered as architecture/research, no implementation.

Current baseline found: request spans/logs to stdout/journald
with `id method uri` in and `status latency_ms` out plus
`x-request-id`; auth/config audit rows in SQLite;
`/healthz` and `/version`; systemd restart and manual
journald error checks. Missing: route/transport labels,
stable 5xx cause events, local metrics, file rotation, and
QUIC connection metrics (spec O3).

Recommended smallest useful direction: keep `tracing` and
enrich the response/error event with transport, route/branch,
status, request ID, and stable error kind; add local counters
or snapshots by route/status/transport; optionally add a
rotating JSON file under the service-writable state dir while
keeping journald primary.

OpenTelemetry: applicable, but optional. It can run without a
collector/server by using stdout/debug exporters and/or a file
appender/custom file exporter for OTel-shaped NDJSON/JSON.
That preserves a standard schema for later shipping, but adds
SDK/exporter complexity, flush/shutdown handling, retention,
and PII/cardinality governance. For one-box/no-vendor ops,
the same debugging value can come from enriched `tracing`
JSON; adopt full OTel mainly if multi-service tracing or a
future collector is planned.

Recorder usefulness: only if it captures the discriminating
fields. Existing logs would show the 500 but not TCP-vs-QUIC
or the missing-extension cause; QUIC bridge failures are
debug-only while prod forbids debug. A recorder emitting
`transport`, route, status, request ID, and error kind would
have split “QUIC auth 500 / TCP OK” and pointed at the bridge.

## Files changed

None (question only; this log file).

## Commits

- (this log; hash on commit)

## Verification

- Read-only inspection of request logging, QUIC bridge logs,
  prod logging/monitoring, service writable paths, and the
  QUIC observability open item.
- `git status --short`: clean before writing this log.
- No code changes, so no test suite was run; staged
  commit-time secrets scan runs via the repo pre-commit hook.

## Follow-ups

- If approved: add transport/route/error-kind fields and 5xx
  cause events; decide journald-only vs rotating local JSON;
  add QUIC counters; record the OTel yes/no decision.
