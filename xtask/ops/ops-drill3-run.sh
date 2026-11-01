#!/bin/bash
# Drill 3 (backup + restore): ONE full run. Run as root; repeat for 3 green runs.
#
#   bash /home/hal/apostille/scripts/ops-drill3-run.sh
#
# Restores the NEWEST daily backup into a fresh /tmp scratch dir
# (/tmp/apostille-restore-<UTC-date>[-n], first-free), re-ports it to
# probed loopback ports, boots the LIVE release web binary against it
# as andrii-ai, proves healthz + packs + sealed-config, attests the
# KEK-escrow envelope WITHOUT disclosure, then tears everything down.
# Never touches live data: every config-set targets the scratch DB.
# Paste the whole output back to the agent.
#
# Exit paths: any failure prints ABORT: <reason> and stops. Scratch is
# removed ONLY on full green; on failure it stays under /tmp for
# inspection (the staged KEK copy is always deleted by the exit trap).
set -u

[ "$(id -u)" = 0 ] || { echo "ABORT: run as root"; exit 1; }

TODAY="$(date -u +%F)"
ROOT=/srv/apostille
DEST=/var/lib/apostille-backups
KEK=/etc/apostille/handbook-kek

echo "=== 0. preconditions"
LIVE="$(readlink "$ROOT/current" 2>/dev/null || true)"
[ -n "$LIVE" ] || { echo "ABORT: $ROOT/current is not a readable symlink"; exit 1; }
case "$LIVE" in
    "$ROOT"/releases/*) ;;
    *) echo "ABORT: $ROOT/current points outside $ROOT/releases ($LIVE)"; exit 1 ;;
esac
[ -d "$LIVE" ] || { echo "ABORT: live release dir missing: $LIVE"; exit 1; }
[ -x "$LIVE/web" ] || { echo "ABORT: no web binary at $LIVE/web"; exit 1; }
[ -x "$LIVE/xtask" ] || { echo "ABORT: no xtask at $LIVE/xtask"; exit 1; }
XTASK=$LIVE/xtask
BUILD="$(basename "$LIVE")"
echo "live release: $LIVE (build $BUILD)"

BASE=/tmp/apostille-restore-$TODAY
SCRATCH="$BASE"
N=1
while [ -e "$SCRATCH" ]; do
    N=$((N + 1))
    SCRATCH="$BASE-$N"
done
DATA=$SCRATCH/data
CREDS=$SCRATCH/creds
PIDFILE=$SCRATCH/web.pid
LOG=$SCRATCH/web.log
echo "scratch: $SCRATCH (first-free)"

cleanup_web() {
    if [ -f "$PIDFILE" ]; then
        PID="$(cat "$PIDFILE" 2>/dev/null || true)"
        case "$PID" in
            ''|*[!0-9]*) ;;
            *)
                if kill -0 "$PID" 2>/dev/null; then
                    kill "$PID" 2>/dev/null || true
                    W=0
                    while [ "$W" -lt 10 ]; do
                        kill -0 "$PID" 2>/dev/null || break
                        sleep 1
                        W=$((W + 1))
                    done
                    kill -9 "$PID" 2>/dev/null || true
                fi
                ;;
        esac
    fi
    rm -f "$CREDS/handbook-kek" 2>/dev/null || true
}
trap cleanup_web EXIT

for t in ss curl sudo sha256sum tar stat; do
    command -v "$t" >/dev/null 2>&1 || { echo "ABORT: missing tool: $t"; exit 1; }
done
"$XTASK" help | grep -q 'backup-verify' || { echo "ABORT: live xtask has no backup-verify (deploy first)"; exit 1; }
# config-* tasks share one pipe-joined help line (no separate
# 'config-set'/'config-verify' substrings to match).
"$XTASK" help | grep -q 'config-enroll|set|get|verify|rotate' || { echo "ABORT: live xtask has no config tasks (deploy first)"; exit 1; }
"$XTASK" help | grep -q 'packs-validate' || { echo "ABORT: live xtask has no packs-validate (deploy first)"; exit 1; }
[ -f "$KEK" ] || { echo "ABORT: KEK file missing: $KEK"; exit 1; }
[ "$(stat -c %s "$KEK")" = 32 ] || { echo "ABORT: KEK $KEK is not 32 bytes"; exit 1; }
[ -d "$LIVE/packs" ] || { echo "ABORT: live release carries no packs dir ($LIVE/packs)"; exit 1; }

DAILY=""
for d in "$DEST"/daily/*/; do
    [ -d "$d" ] || continue
    DAILY="$d"
done
[ -n "$DAILY" ] || { echo "ABORT: no daily backup under $DEST/daily"; exit 1; }
DAILY="${DAILY%/}"
BID="$(basename "$DAILY")"
echo "newest daily: $DAILY (backup id $BID)"

port_free() {
    if ss -ltn 2>/dev/null | grep -q ":$1 "; then
        return 1
    fi
    return 0
}
PUB=0
P=18080
N=0
while [ "$N" -lt 20 ]; do
    if port_free "$P"; then PUB="$P"; break; fi
    P=$((P + 1))
    N=$((N + 1))
done
[ "$PUB" != 0 ] || { echo "ABORT: no free scratch public port in 18080-18099"; exit 1; }
ADMIN=0
P=18443
N=0
while [ "$N" -lt 20 ]; do
    if port_free "$P"; then ADMIN="$P"; break; fi
    P=$((P + 1))
    N=$((N + 1))
done
[ "$ADMIN" != 0 ] || { echo "ABORT: no free scratch admin port in 18443-18462"; exit 1; }
echo "scratch ports: public $PUB admin $ADMIN (loopback)"
echo "preconditions OK"

echo "=== 1. sha256sums of the daily (as andrii-ai)"
for f in handbook.sqlite p2.sqlite files.tar.gz manifest.json sha256sums; do
    [ -f "$DAILY/$f" ] || { echo "ABORT: $DAILY missing $f"; exit 1; }
done
[ "$(ls "$DAILY" | wc -l)" = 5 ] || { echo "ABORT: $DAILY holds extra files (WAL residue?)"; exit 1; }
(cd "$DAILY" && sudo -u andrii-ai sha256sum -c sha256sums) \
    || { echo "ABORT: sha256sums failed in $DAILY"; exit 1; }
echo "daily $BID: file set exact, sums OK"

echo "=== 2. restore proof (backup-verify into scratch, as andrii-ai)"
sudo -u andrii-ai "$XTASK" backup-verify --backup "$DAILY" --scratch "$SCRATCH" \
    || { echo "ABORT: backup-verify failed (scratch left at $SCRATCH for inspection)"; exit 1; }
[ -d "$DATA" ] || { echo "ABORT: backup-verify exited 0 but $DATA is missing (scratch left at $SCRATCH for inspection)"; exit 1; }
echo "backup-verify green"

echo "=== 3. re-port + plain-HTTP loopback repin (scratch DB only, as root)"
set_scratch() {
    "$XTASK" config-set --store "$DATA" --key "$1" --type "$2" --value "$3" --kek-file "$KEK" --actor drill3 \
        || { echo "ABORT: config-set $1 failed (scratch left at $SCRATCH for inspection)"; exit 1; }
}
set_scratch net.port text "$PUB"
set_scratch net.admin_port text "$ADMIN"
set_scratch net.bind text "127.0.0.1"
set_scratch net.bind6 text "::1"
set_scratch tls.cert_file text ""
set_scratch tls.key_file text ""
set_scratch acme.enabled bool "false"
set_scratch quic.enabled bool "false"
echo "scratch rows: ports $PUB/$ADMIN, loopback binds, TLS/ACME/QUIC off"
mkdir -p "$CREDS"
chmod 700 "$CREDS"
/usr/bin/install -m 600 -o andrii-ai -g andrii-ai "$KEK" "$CREDS/handbook-kek" \
    || { echo "ABORT: cannot stage KEK copy (scratch left at $SCRATCH for inspection)"; exit 1; }
chown -R andrii-ai:andrii-ai "$SCRATCH" \
    || { echo "ABORT: chown scratch failed (scratch left at $SCRATCH for inspection)"; exit 1; }
echo "scratch chowned to andrii-ai (prod-like ownership), KEK staged 0600"

echo "=== 4. boot scratch web (as andrii-ai) + healthz"
sudo -u andrii-ai /bin/bash -c "echo \$\$ > \"$PIDFILE\"; exec /usr/bin/env CREDENTIALS_DIRECTORY=\"$CREDS\" STORE_DIR=\"$DATA\" \"$LIVE/web\" >>\"$LOG\" 2>&1" &
N=0
while [ "$N" -lt 10 ] && [ ! -f "$PIDFILE" ]; do sleep 1; N=$((N + 1)); done
[ -f "$PIDFILE" ] || { echo "ABORT: scratch web wrote no pidfile (scratch left at $SCRATCH for inspection)"; exit 1; }
echo "scratch web pid: $(cat "$PIDFILE") (log $LOG)"
HEALTH=""
N=0
while [ "$N" -lt 30 ]; do
    if HEALTH="$(curl -fsS "http://127.0.0.1:$PUB/apostille/healthz" 2>/dev/null)"; then
        break
    fi
    HEALTH=""
    sleep 1
    N=$((N + 1))
done
if [ -z "$HEALTH" ]; then
    echo "FAIL: scratch web never answered healthz in 30s (log tail below)"
    tail -n 50 "$LOG" 2>/dev/null || true
    echo "ABORT: scratch healthz timeout (scratch left at $SCRATCH for inspection; KEK copy deleted)"
    exit 1
fi
echo "healthz: $HEALTH"
echo "$HEALTH" | grep -q "$BUILD" || { echo "ABORT: healthz stamp does not name build $BUILD: $HEALTH (scratch left at $SCRATCH for inspection)"; exit 1; }
echo "healthz 200, stamp names build $BUILD"
ss -ltn 2>/dev/null | grep -E ":$PUB |:$ADMIN " || true

echo "=== 5. packs-validate + sealed-config verify"
sudo -u andrii-ai "$XTASK" packs-validate --packs "$LIVE/packs" \
    || { echo "ABORT: packs-validate failed (scratch left at $SCRATCH for inspection)"; exit 1; }
"$XTASK" config-verify --store "$DATA" --kek-file "$KEK" \
    || { echo "ABORT: config-verify on scratch failed (scratch left at $SCRATCH for inspection)"; exit 1; }
echo "packs clean, scratch sealed-config verifies"

echo "=== 6. KEK-escrow envelope attestation (NO disclosure)"
[ -t 0 ] || { echo "ABORT: no interactive terminal for envelope attestation (scratch left at $SCRATCH for inspection)"; exit 1; }
echo "Locate the sealed KEK-escrow envelope now. Do NOT open it."
echo "Type NOTHING secret - just press Enter to attest you located it."
JUNK=""
read -s -r -p "envelope located - press Enter to attest: " JUNK || { echo; echo "ABORT: attestation input closed (scratch left at $SCRATCH for inspection)"; exit 1; }
echo
[ -z "$JUNK" ] || { echo "ABORT: input was typed during attestation - never type secrets here; re-run and press Enter on an empty line (scratch left at $SCRATCH for inspection)"; exit 1; }
echo "attested: operator located the KEK-escrow envelope (unopened, nothing disclosed)"

echo "=== 7. teardown (kill scratch web, verify silence, remove scratch)"
cleanup_web
if ss -ltn 2>/dev/null | grep -qE ":$PUB |:$ADMIN "; then
    echo "ABORT: a scratch-port listener is still up after kill (scratch left at $SCRATCH for inspection)"
    exit 1
fi
echo "no scratch-port listener remains"
case "$SCRATCH" in
    /tmp/apostille-restore-*) ;;
    *) echo "ABORT: refusing to remove unexpected path: $SCRATCH"; exit 1 ;;
esac
rm -rf "$SCRATCH" || { echo "ABORT: cannot remove $SCRATCH"; exit 1; }
echo "scratch removed (full green)"

echo "=== 8. evidence (paste back to the agent)"
echo "build: $BUILD"
echo "backup id: $BID"
echo "scratch ports: public $PUB admin $ADMIN (loopback)"
echo "healthz: $HEALTH"
echo "drill-log row:"
echo "| $TODAY | 3 backup+restore | root | $BUILD | green (backup $BID, scratch boot $PUB/$ADMIN loopback, healthz 200 + stamp, packs clean, config-verify clean, envelope located) |"
echo "DONE drill-3 run (backup $BID restored + booted green, scratch removed) - paste this whole output back to the agent"
