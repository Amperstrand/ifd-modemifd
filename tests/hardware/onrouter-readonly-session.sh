#!/bin/sh
# onrouter-readonly-session.sh — session-in-a-box for the NR7101-only
# read-only milestone (docs/ONROUTER-READONLY-OPTIONS.md ladder: preflight
# -> R1a/R1b -> R2; R3a/R3b are owner-gated composition switches and are
# deliberately NOT in this script).
#
# Runs ON THE ROUTER as root, under the modem-lane lock (caller holds it).
# READ-ONLY against the card: chip info / profile list / notification list /
# C0 battery — no writes, no profile operations, no AT commands outside the
# class-A list. Card-safety doctrine: tests/hardware/README.md.
#
# Artifacts: /tmp/onrouter-readonly-<ts>/ (also echoed to stdout). The three
# window-checklist outcomes (fork-research lane):
#   1. R1a stock capture shows the lowercase-ERROR fingerprint (or not)
#   2. R1b atcsimUC reaches FCI -> EID
#   3. any die-after-ES10-error-JSON = upstream post-error segfault caveat,
#      NOT a transport finding (d7deded)

TS=$(date -u +%Y%m%dT%H%M%SZ)
DIR="/tmp/onrouter-readonly-$TS"
LPAC_STOCK="${LPAC_STOCK:-/usr/bin/lpac}"          # stock a43e46ae at_csim
LPAC_UC="${LPAC_UC:-/usr/bin/lpac-atcsimuc}"        # fed4c6c1 discrimination build
BATTERY="${BATTERY:-/usr/libexec/at_csim_c0_battery.sh}"
TTY="${TTY:-/dev/ttyUSB2}"                          # auto-detect if gone

say() { echo "$(date -u +%Y-%m-%dT%H:%M:%SZ) $*" | tee -a "$DIR/session.log"; }
mkdir -p "$DIR"

stop() { say "STOP: $*"; say "artifacts: $DIR"; exit 1; }

say "=== on-router read-only session start ($DIR) ==="

# ---------- preflight (reviewer-endorsed preconditions) ----------
[ "$(id -u)" = 0 ] || stop "must run as root"
say "-- config backup --"
sysupgrade -b "$DIR/config-backup.tar.gz" >/dev/null 2>&1 || say "WARN: sysupgrade -b failed (record and continue)"
say "-- clock (TLS + log forensics) --"
date -u; ntpd -q -p "$(uci get system.@system[0].ntpserver 2>/dev/null | awk '{print $1}')" >/dev/null 2>&1 || say "WARN: NTP sync attempted; verify date above is sane"
say "-- flash headroom --"
df -h /overlay /tmp | tee -a "$DIR/session.log"
say "-- AT-level state battery --"
for cmd in 'ATI' 'AT+CPIN?' 'AT+QCCID' 'AT+QUIMSLOT?' 'AT+QSIMCFG="dual_slot_status"'; do
  printf '%s\r' "$cmd" > "$TTY" 2>/dev/null; sleep 2
done > "$DIR/at-battery.log" 2>&1 || say "WARN: AT battery write failed (tty $TTY?)"
cat "$DIR/at-battery.log" || true
say "-- R0: installed uqmi UIM surface (no modem contact) --"
uqmi --help 2>&1 | grep -o -E "uim-(channel-open|channel-close|apdu-send|slot)" | sort -u | tee "$DIR/r0-uqmi-surface.txt"
[ -s "$DIR/r0-uqmi-surface.txt" ] || say "WARN: uqmi UIM APDU surface not advertised — R3a gate will need the pinned artifact's expectations adjusted"

# ---------- R1a: STOCK at_csim, wire capture ----------
say "-- R1a: stock lpac chip info (AT_DEBUG on) --"
AT_DEBUG=1 LPAC_APDU=at_csim LPAC_QMI_DEV="$TTY" timeout 120 "$LPAC_STOCK" chip info \
  > "$DIR/r1a-chipinfo.out" 2> "$DIR/r1a-chipinfo.atdebug"
r1a=$?
say "R1a exit=$r1a (checklist 1: grep the lowercase-ERROR fingerprint)"
grep -m3 -i -E 'error|fail' "$DIR/r1a-chipinfo.atdebug" || true

# ---------- R1b: atcsimUC (uppercase + #446), same gate ----------
say "-- R1b: atcsimUC chip info (AT_DEBUG on) --"
AT_DEBUG=1 LPAC_APDU=at_csim LPAC_QMI_DEV="$TTY" timeout 120 "$LPAC_UC" chip info \
  > "$DIR/r1b-chipinfo.out" 2> "$DIR/r1b-chipinfo.atdebug"
r1b=$?
say "R1b exit=$r1b (checklist 2: EID printed? die-after-ES10-JSON = segfault caveat, not transport)"
grep -m1 -E '^[0-9A-F]{32}$' "$DIR/r1b-chipinfo.out" && say "R1b: EID OBTAINED — AT path revived, milestone met on R1" || true
tail -3 "$DIR/r1b-chipinfo.out" 2>/dev/null || true

# ---------- R2: corrected C0 battery (rehearsed instrument) ----------
say "-- R2: corrected C0 battery (N=2) --"
if [ -x "$BATTERY" ]; then
  sh "$BATTERY" "$TTY" 2 > "$DIR/r2-battery.log" 2>&1
  r2=$?
  say "R2 exit=$r2"; grep -E "C0:|hex-case:|inline-FCI:" "$DIR/r2-battery.log"
else
  say "SKIP: battery not found at $BATTERY"
fi

say "=== session complete — artifacts $DIR ==="
say "interpretation reminders: stock-vs-UC delta = client-side case verdict;"
say "close-refusal is an independent firmware defect (never a case symptom);"
say "61-21/6A82 outcomes belong to R2 scope only."
exit 0
