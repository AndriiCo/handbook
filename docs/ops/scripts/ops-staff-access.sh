#!/bin/bash
# Export a staffer's mTLS client cert for browser install + show
# staff account IDs for passkey login. Runs as ROOT (reads 0600
# keys and the live DB).
#
#   bash /home/hal/apostille/scripts/ops-staff-access.sh <cn>
#
# Zero placeholders: prompts for the .p12 export password.
# Output: /home/hal/<cn>-staff.p12 (hal-owned, 0600) + the
# staff user_id rows. The staffer then:
#   scp andrii.ai:<cn>-staff.p12 ~/Downloads/   (from their Mac)
#   double-click → import into the login Keychain
#   reload https://admin.andrii.ai/admin/login, enter the user_id,
#   approve the passkey prompt (browser offers the cert itself).
# Then delete the .p12 copies (server + Mac).
set -u

CN="${1:?usage: $0 <cn> (e.g. andrii.ai)}"
[[ "$CN" =~ ^[A-Za-z0-9._-]+$ ]] || { echo "ABORT: bad cn: $CN"; exit 1; }
[ "$(id -u)" = "0" ] || { echo "ABORT: run as root"; exit 1; }
STORE="/var/lib/apostille"
CERTS="$STORE/admin-certs"
XTASK="/srv/apostille/current/xtask"
[ -x "$XTASK" ] || { echo "ABORT: no xtask at $XTASK"; exit 1; }

echo "=== 1. newest live cert for cn=$CN"
CERT="$(ls -t "$CERTS"/client-"$CN"-*.pem 2>/dev/null | grep -v '\.key\.pem$' | head -1 || true)"
[ -n "$CERT" ] || { echo "ABORT: no client cert for cn=$CN in $CERTS"; exit 1; }
KEY="${CERT%.pem}.key.pem"
[ -f "$KEY" ] || { echo "ABORT: key missing for $CERT"; exit 1; }
"$XTASK" admin-cert-verify --store "$STORE" --cert "$CERT" \
    || { echo "ABORT: cert not live (revoked/expired/unknown?)"; exit 1; }

echo "=== 2. export PKCS#12"
read -r -s -p "Export password for the .p12 (empty = none): " P1; echo
read -r -s -p "Repeat to confirm: " P2; echo
[ "$P1" = "$P2" ] || { echo "ABORT: passwords differ"; exit 1; }
PASSFILE="$(mktemp /root/.p12pass.XXXXXX)" || exit 1
chmod 600 "$PASSFILE"
printf '%s' "$P1" > "$PASSFILE"
P1=""; P2=""
OUT="/home/hal/$CN-staff.p12"
openssl pkcs12 -export -out "$OUT" -in "$CERT" -inkey "$KEY" \
    -name "$CN staff" -passout "file:$PASSFILE" \
    || { rm -f "$PASSFILE"; echo "ABORT: openssl export failed"; exit 1; }
rm -f "$PASSFILE"
chown hal:hal "$OUT" && chmod 600 "$OUT"
echo "wrote $OUT"

echo "=== 3. staff account IDs (login = user_id + passkey)"
sqlite3 "$STORE/handbook.sqlite" \
    "SELECT user_id, role FROM account WHERE role IN ('admin','staff') ORDER BY role, user_id;" \
    || { echo "ABORT: account lookup failed"; exit 1; }

echo "=== next"
echo "On the staffer's Mac: scp andrii.ai:$CN-staff.p12 ~/Downloads/"
echo "Double-click to import into the login Keychain, reload the admin"
echo "page (browser offers the cert itself), log in with the user_id above."
echo "Then delete the .p12 on the server (rm $OUT) and on the Mac."
