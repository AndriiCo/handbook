#!/bin/bash
# Reinstall the watch/mail/instruct/backup systemd units after the
# F1/F2 fixes. Run as root AFTER the agent stages the 8 fixed files:
#
#   /home/hal/apostille/units-staging/apostille-{watch,mail,instruct,backup}.{service,timer}
#
#   bash /home/hal/apostille/scripts/ops-units-reinstall.sh
#
# Validates every staged file (F1 gate: every xtask ExecStart
# carries --store /var/lib/apostille; F2 gate: an instruct --pack
# path must exist under /srv/apostille/current), prints diffs,
# backs the installed set up under /root, installs,
# daemon-reloads, and enables ONLY the D41-approved timers
# (watch + mail). instruct stays installed-but-disabled
# (PROPOSED: enable only after its first live sweeps are
# reviewed); backup stays installed-but-disabled (no enable
# decision yet). Paste the whole output back to the agent.
#
# Trust note: validation guards against F1/F2-class breakage, not
# malice — hal's release binaries already run as andrii-ai.
set -u

[ "$(id -u)" = 0 ] || { echo "ABORT: run as root"; exit 1; }

STAGE=/home/hal/apostille/units-staging
DEST=/etc/systemd/system
BAK=/root/apostille-units.bak.$(date -u +%Y%m%dT%H%M%SZ)

UNITS="watch mail instruct backup"

echo "=== 0. staged set present (8 fixed files)"
[ -d "$STAGE" ] || { echo "ABORT: no staging dir $STAGE (agent stages the 8 unit files first)"; exit 1; }
for u in $UNITS; do
    for ext in service timer; do
        f=$STAGE/apostille-$u.$ext
        [ -f "$f" ] && [ -s "$f" ] || { echo "ABORT: $f missing or empty"; exit 1; }
    done
done
echo "staged set complete"

echo "=== 1. validate services (User/Group, ExecStart allowlist, F1 + F2 gates)"
for u in $UNITS; do
    f=$STAGE/apostille-$u.service
    grep -qx 'User=andrii-ai' "$f" || { echo "ABORT: $f: User=andrii-ai missing"; exit 1; }
    grep -qx 'Group=andrii-ai' "$f" || { echo "ABORT: $f: Group=andrii-ai missing"; exit 1; }
    grep -q '^User=root\|^Group=root' "$f" && { echo "ABORT: $f: root unit refused"; exit 1; }
    [ "$(grep -c '^ExecStart=' "$f")" -ge 1 ] || { echo "ABORT: $f: no ExecStart"; exit 1; }
    case "$u" in
        watch|mail|instruct)
            BAD="$(grep '^ExecStart=' "$f" | grep -v '^ExecStart=/srv/apostille/current/xtask ' || true)"
            [ -z "$BAD" ] || { echo "ABORT: $f: non-xtask ExecStart:"; echo "$BAD"; exit 1; }
            case "$u" in
                watch) WANT='watch-sweep|hcch-poll' ;;
                mail) WANT='mail-poll' ;;
                instruct) WANT='instruct-sweep' ;;
            esac
            # WANT is a fixed alternation, never input.
            BAD="$(grep '^ExecStart=' "$f" | grep -v -E "^ExecStart=/srv/apostille/current/xtask ($WANT)( |$)" || true)"
            [ -z "$BAD" ] || { echo "ABORT: $f: unexpected xtask subcommand:"; echo "$BAD"; exit 1; }
            ;;
        backup)
            BAD="$(grep '^ExecStart=' "$f" | grep -v -E '^ExecStart=(/srv/apostille/current/|/usr/bin/)' || true)"
            [ -z "$BAD" ] || { echo "ABORT: $f: ExecStart outside release-/usr/bin:"; echo "$BAD"; exit 1; }
            BAD="$(grep '^ExecStart=' "$f" | grep -E '[;|&$`]' || true)"
            [ -z "$BAD" ] || { echo "ABORT: $f: shell metachar in ExecStart:"; echo "$BAD"; exit 1; }
            ;;
    esac
    BAD="$(grep '^ExecStart=/srv/apostille/current/xtask ' "$f" | grep -v -F -- '--store /var/lib/apostille' || true)"
    [ -z "$BAD" ] || { echo "ABORT: $f: xtask ExecStart without --store /var/lib/apostille (F1):"; echo "$BAD"; exit 1; }
    if [ "$u" = instruct ]; then
        # ExecStart lines only: header comments may mention --pack
        # (seen 2026-10-30: "# ... --pack repointed ..." ABORTed the gate).
        PACK="$(grep '^ExecStart=' "$f" | grep -o -E -- '--pack [^ ]+' | head -n 1 | cut -d' ' -f2 || true)"
        if [ -z "$PACK" ]; then
            echo "(note: $f has no --pack flag; xtask default applies)"
        else
            case "$PACK" in
                /srv/apostille/current/*) ;;
                *) echo "ABORT: $f: --pack outside the live release: $PACK"; exit 1 ;;
            esac
            [ -f "$PACK" ] || { echo "ABORT: $f: --pack $PACK missing from the live release (F2)"; exit 1; }
            echo "(instruct --pack OK: $PACK)"
        fi
    fi
    echo "service OK: apostille-$u.service"
done

echo "=== 2. validate timers (trigger + install target, no ExecStart)"
for u in $UNITS; do
    t=$STAGE/apostille-$u.timer
    grep -q '^\[Timer\]' "$t" || { echo "ABORT: $t: no [Timer] section"; exit 1; }
    grep -q '^OnCalendar=\|^OnBootSec=' "$t" || { echo "ABORT: $t: no OnCalendar=/OnBootSec= trigger"; exit 1; }
    grep -q '^WantedBy=timers.target' "$t" || { echo "ABORT: $t: no WantedBy=timers.target"; exit 1; }
    grep -q '^ExecStart=' "$t" && { echo "ABORT: $t: ExecStart does not belong in a timer"; exit 1; }
    UL="$(grep '^Unit=' "$t" || true)"
    if [ -n "$UL" ] && [ "$UL" != "Unit=apostille-$u.service" ]; then
        echo "ABORT: $t: $UL does not match apostille-$u.service"; exit 1
    fi
    echo "timer OK: apostille-$u.timer"
done

echo "=== 3. diffs vs installed (new files noted)"
for u in $UNITS; do
    for ext in service timer; do
        if [ -f "$DEST/apostille-$u.$ext" ]; then
            diff -u "$DEST/apostille-$u.$ext" "$STAGE/apostille-$u.$ext" || true
        else
            echo "(new file: apostille-$u.$ext)"
        fi
    done
done

echo "=== 4. back up installed set, then install"
mkdir -p "$BAK" || { echo "ABORT: cannot create $BAK"; exit 1; }
for u in $UNITS; do
    for ext in service timer; do
        [ -f "$DEST/apostille-$u.$ext" ] && cp "$DEST/apostille-$u.$ext" "$BAK/" || true
    done
done
echo "backup dir: $BAK ($(ls "$BAK" 2>/dev/null | wc -l) file(s) — 0 means fresh install)"
for u in $UNITS; do
    for ext in service timer; do
        cp "$STAGE/apostille-$u.$ext" "$DEST/apostille-$u.$ext" || { echo "ABORT: install of apostille-$u.$ext failed"; exit 1; }
        chmod 0644 "$DEST/apostille-$u.$ext"
    done
done
echo "installed: 8 files"

echo "=== 5. daemon-reload + enable ONLY the D41 timers (watch + mail)"
/usr/bin/systemctl daemon-reload || { echo "ABORT: daemon-reload failed"; exit 1; }
echo "daemon-reload OK"
/usr/bin/systemctl enable --now apostille-watch.timer || { echo "ABORT: enable watch timer failed"; exit 1; }
/usr/bin/systemctl enable --now apostille-mail.timer || { echo "ABORT: enable mail timer failed"; exit 1; }
echo "enabled: apostille-watch.timer apostille-mail.timer (decision 41)"
echo "left disabled by design: apostille-instruct.timer (PROPOSED), apostille-backup.timer (no enable decision)"

echo "=== 6. evidence: states + installed ExecStarts + next fires"
for u in watch mail; do
    ST="$(/usr/bin/systemctl is-enabled "apostille-$u.timer")"
    [ "$ST" = enabled ] || { echo "ABORT: apostille-$u.timer state is $ST (expected enabled)"; exit 1; }
    echo "apostille-$u.timer: $ST"
done
for u in instruct backup; do
    ST="$(/usr/bin/systemctl is-enabled "apostille-$u.timer" || true)"
    [ "$ST" = disabled ] || echo "WARNING: apostille-$u.timer state is $ST (expected disabled — left as-is, agent call)"
    [ "$ST" = disabled ] && echo "apostille-$u.timer: $ST"
done
for u in watch mail instruct; do
    /usr/bin/systemctl show -p ExecStart "apostille-$u.service" | grep -q -F -- '--store /var/lib/apostille' \
        || { echo "ABORT: installed apostille-$u.service lost --store (F1)"; exit 1; }
done
echo "installed ExecStarts carry --store (F1 confirmed live)"
/usr/bin/systemctl list-timers --all | grep -E 'apostille|NEXT' || true
echo "DONE units reinstall (backup: $BAK) — paste this whole output back to the agent"
