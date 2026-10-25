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
RUNGS="${RUNGS:-r1a r1b r2}"
LPAC_STOCK="${LPAC_STOCK:-/usr/bin/lpac}"          # stock a43e46ae at_csim
LPAC_UC="${LPAC_UC:-/usr/bin/lpac-atcsimuc}"        # fed4c6c1 discrimination build
BATTERY="${BATTERY:-/usr/libexec/at_csim_c0_battery.sh}"
TTY="${TTY:-/dev/ttyUSB2}"                          # auto-detect if gone

say() { echo "$(date -u +%Y-%m-%dT%H:%M:%SZ) $*" | tee -a "$DIR/session.log"; }
mkdir -p "$DIR"

stop() { say "STOP: $*"; say "artifacts: $DIR"; exit 1; }

# Router-reality timeout (stock OpenWrt has no `timeout` applet; overlay is
# too tight to install coreutils-timeout for this): background + watchdog.
run_to() {  # run_to <secs> <cmd...>
  _secs=$1; shift
  "$@" &
  _p=$!
  _d=$(( $(date +%s) + _secs ))
  while kill -0 "$_p" 2>/dev/null && [ "$(date +%s)" -lt "$_d" ]; do sleep 2; done
  if kill -0 "$_p" 2>/dev/null; then kill "$_p" 2>/dev/null; wait "$_p" 2>/dev/null; return 124; fi
  wait "$_p"
}

# Reader-backed AT exchange (a battery that cannot fail verifies nothing):
# start a reader, send, collect, parse. Prints the raw response block.
at_read() {  # at_read <cmd> [wait_s]
  _cmd=$1; _wait=${2:-3}
  _f="$DIR/at.$$"
  cat "$TTY" > "$_f" &
  _cp=$!
  sleep 1
  printf '%s\r' "$_cmd" > "$TTY"
  sleep "$_wait"
  kill "$_cp" 2>/dev/null
  RESP=$(grep -v '^$' "$_f" | sed -n '/^'"$_cmd"'/,$p' | grep -v '^'"$_cmd"'$')
  rm -f "$_f"
  echo "$RESP"
}

# (C) CFUN recycle + TTY re-detection: the port shuffles across resets.
cfun_recycle() {
  say "CFUN recycle (AT+CFUN=1,1, ~50s, then TTY re-detect)"
  at_read 'AT+CFUN=1,1' 5 >/dev/null 2>&1
  sleep 50
  _found=""
  for t in /dev/ttyUSB*; do
    [ -c "$t" ] || continue
    TTY="$t"
    if at_read 'AT' 2 2>/dev/null | grep -q 'OK'; then _found="$t"; break; fi
  done
  [ -n "$_found" ] || stop "no AT-responsive tty after recycle (checked /dev/ttyUSB*)"
  say "TTY re-detected: $TTY"
  at_read 'AT+CPIN?' 3 | grep -q 'READY' || say "WARN: CPIN not READY after recycle"
}

say "=== on-router read-only session start ($DIR) ==="

# ---------- preflight (reviewer-endorsed preconditions) ----------
[ "$(id -u)" = 0 ] || stop "must run as root"
say "-- config backup --"
sysupgrade -b "$DIR/config-backup.tar.gz" >/dev/null 2>&1 || say "WARN: sysupgrade -b failed (record and continue)"
say "-- clock (TLS + log forensics) --"
date -u; ntpd -q -p "$(uci get system.@system[0].ntpserver 2>/dev/null | awk '{print $1}')" >/dev/null 2>&1 || say "WARN: NTP sync attempted; verify date above is sane"
say "-- flash headroom --"
df -h /overlay /tmp | tee -a "$DIR/session.log"
say "-- AT-level state battery (reader-backed; aborts on mismatch) --"
: > "$DIR/at-battery.log"
for cmd in 'ATI' 'AT+CPIN?' 'AT+QCCID' 'AT+QUIMSLOT?' 'AT+QSIMCFG="dual_slot_status"'; do
  { echo "== $cmd"; at_read "$cmd" 3; echo; } | tee -a "$DIR/at-battery.log"
done
grep -q 'CPIN: READY' "$DIR/at-battery.log" || stop "CPIN not READY — card/modem not in session-start state"
QCCID=$(sed -n 's/^+QCCID: *\([0-9A-Fa-f]*\).*/\1/p' "$DIR/at-battery.log" | tail -1)
QSLOT=$(sed -n 's/^+QUIMSLOT: *\([0-9]*\).*/\1/p' "$DIR/at-battery.log" | tail -1)
if [ -n "${EXPECTED_QCCID:-}" ] && [ "$QCCID" != "$EXPECTED_QCCID" ]; then
  stop "QCCID mismatch: got ${QCCID:-none}, expected eUICC ICCID (wrong card active?)"
fi
[ -n "${EXPECTED_QCCID:-}" ] || say "WARN: EXPECTED_QCCID unset — QCCID not verified"
if [ -n "${EXPECTED_SLOT:-}" ] && [ "$QSLOT" != "$EXPECTED_SLOT" ]; then
  stop "QUIMSLOT mismatch: got ${QSLOT:-none}, expected $EXPECTED_SLOT"
fi
[ -n "${EXPECTED_SLOT:-}" ] || say "WARN: EXPECTED_SLOT unset — slot not verified"
say "-- R0: installed uqmi UIM surface (no modem contact) --"
uqmi --help 2>&1 | grep -o -E "uim-(channel-open|channel-close|apdu-send|slot)" | sort -u | tee "$DIR/r0-uqmi-surface.txt"
[ -s "$DIR/r0-uqmi-surface.txt" ] || say "WARN: uqmi UIM APDU surface not advertised — R3a gate will need the pinned artifact's expectations adjusted"

# ---------- R1a: STOCK at_csim, wire capture ----------
# (D) deployment-order guard: the atcsimUC tarball installs its driver at the
# SAME path as stock (no LPAC_DRIVER_HOME in a43e46ae) — the stock run must
# happen BEFORE any UC overlay, or from a disjoint tree.
UC_SO=""
for _d in /usr/lib/lpac/driver /usr/lib/lpac /usr/lib; do
  [ -f "$_d/driver_apdu_at_csim.so" ] && UC_SO="$_d" && break
done
if [ -n "$UC_SO" ] && grep -qa '0123456789ABCDEF' "$UC_SO/driver_apdu_at_csim.so" 2>/dev/null; then
  stop "uppercase (UC) driver already installed at $UC_SO — R1a stock run impossible; reinstall stock or point LPAC_STOCK at a disjoint tree"
fi
say "deployment-order check: stock at_csim driver confirmed (no UC overlay)"
cfun_recycle
say "-- R1a: stock lpac chip info (AT_DEBUG on) --"
if case " $RUNGS " in *" r1a "*) true;; *) false;; esac; then
AT_DEBUG=1 LPAC_APDU=at_csim LPAC_APDU_AT_DEVICE="$TTY" run_to 120 "$LPAC_STOCK" chip info \
  > "$DIR/r1a-chipinfo.out" 2> "$DIR/r1a-chipinfo.atdebug"
r1a=$?
say "R1a exit=$r1a (checklist 1: grep the lowercase-ERROR fingerprint)"
grep -m3 -i -E 'error|fail' "$DIR/r1a-chipinfo.atdebug" || true
else say "SKIP R1a (RUNGS=$RUNGS)"; fi

# ---------- R1b: atcsimUC (uppercase + #446), same gate ----------
cfun_recycle
say "-- R1b: atcsimUC chip info (AT_DEBUG on) --"
if case " $RUNGS " in *" r1b "*) true;; *) false;; esac; then
if [ -n "${LPAC_UC_ENV:-}" ]; then
  AT_DEBUG=1 LPAC_APDU=at_csim LPAC_APDU_AT_DEVICE="$TTY" run_to 120 env $LPAC_UC_ENV "$LPAC_UC" chip info \
    > "$DIR/r1b-chipinfo.out" 2> "$DIR/r1b-chipinfo.atdebug"
else
  AT_DEBUG=1 LPAC_APDU=at_csim LPAC_APDU_AT_DEVICE="$TTY" run_to 120 "$LPAC_UC" chip info \
    > "$DIR/r1b-chipinfo.out" 2> "$DIR/r1b-chipinfo.atdebug"
fi
else say "SKIP R1b (RUNGS=$RUNGS)"; fi
r1b=$?
say "R1b exit=$r1b (checklist 2: EID printed? die-after-ES10-JSON = segfault caveat, not transport)"
EIDLINE=$(grep -o '"eid"[: ]*"[0-9A-Fa-f]\{32\}"' "$DIR/r1b-chipinfo.out" | head -1)
[ -n "$EIDLINE" ] && { say "R1b: EID OBTAINED ($EIDLINE) — AT path revived, milestone met on R1"; } || true
tail -3 "$DIR/r1b-chipinfo.out" 2>/dev/null || true

# ---------- R2: corrected C0 battery (rehearsed instrument) ----------
cfun_recycle
say "-- R2: corrected C0 battery (N=2) --"
if [ -x "$BATTERY" ] && case " $RUNGS " in *" r2 "*) true;; *) false;; esac; then
  sh "$BATTERY" "$TTY" 2 > "$DIR/r2-battery.log" 2>&1
  r2=$?
  say "R2 exit=$r2"; grep -E "C0:|hex-case:|inline-FCI:" "$DIR/r2-battery.log"
elif [ -x "$BATTERY" ]; then
  say "SKIP R2 (RUNGS=$RUNGS)"
else
  say "SKIP: battery not found at $BATTERY"
fi

say "=== session complete — artifacts $DIR ==="
say "interpretation reminders: stock-vs-UC delta = client-side case verdict;"
say "close-refusal is an independent firmware defect (never a case symptom);"
say "61-21/6A82 outcomes belong to R2 scope only."
exit 0
