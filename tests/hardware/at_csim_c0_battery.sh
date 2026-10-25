#!/bin/sh
# at_csim_c0_battery.sh — corrected CSIM probe battery: the post-window
# experiment that decides the session-state hypothesis (issue #32).
#
# Runs ON the router (POSIX sh) against the modem AT port directly. The
# caller MUST hold the modem-lane lock / tty exclusivity — see
# tests/hardware/README.md doctrine. POST-WINDOW class: do not run during a
# provisioning window.
#
# What this fixes vs the 2026-10-24 probes (findings correction 3):
#   * GET RESPONSE with genuine INS C0 (the lineage issued INS 2C = Reset
#     Retry Counter, whose 6D00 rejection is correct card behavior);
#   * CLA encodes the ALLOCATED channel per ISO/IEC 7816-4: channels 1-3 in
#     CLA bits 1-0 (0x01-0x03), channels 4-19 in the b7=1 form
#     0x40+(n-4) — the archived 61-21 row used CLA 01 while allocation
#     gave 02/03, and the round-4 template used 4%X for channels 2/3
#     (addressing channels 6/7 instead);
#   * every AT+CSIM line is mechanically length-audited (TS 27.007: <length>
#     is the command hex CHARACTER count; mismatch yields unconditional
#     ERROR — findings doc section 7).
#
# Card-safety: non-authenticating only (MANAGE CHANNEL, SELECT of the public
# GSMA ISD-R AID, GET RESPONSE, MANAGE CHANNEL close, one FID select).
# Never loop secure-channel / STORE DATA class commands.

TTY="${1:-/dev/ttyUSB2}"
N="${2:-3}"
LOG="/tmp/csim-c0-battery-$(date -u +%Y%m%dT%H%M%SZ).log"
# Public GSMA ISD-R AID (published in SGP.22 and lpac sources)
ISDR="A0000005591010FFFFFFFF8900000100"

ts() { date -u +%Y-%m-%dT%H:%M:%SZ; }
say() { echo "$(ts) $*" | tee -a "$LOG"; }

# iso7816 CLA for an allocated logical channel (1..19)
cla_for_channel() {
  n=$1
  if [ "$n" -le 3 ]; then printf '%02X' "$n"; else printf '%02X' $((0x40 + n - 4)); fi
}

# hexlen <hexstring> -> character count (2x byte count)
hexlen() { printf '%s' "$1" | wc -c | tr -d ' '; }

# csim <hexcommand> [wait_s] [gap_before_s]
#   Length-audited AT+CSIM; REFUSES to send on a length mismatch.
csim() {
  cmd=$1; wait=${2:-3}; gap=${3:-1}
  len=$(hexlen "$cmd")
  # declared length must equal the hex character count of the command
  if [ "$len" -ne "${#cmd}" ]; then
    say "LENGTH-AUDIT-FAIL: internal ('$len' vs '${#cmd}') — line refused"
    RESP=""; return 2
  fi
  line="AT+CSIM=$len,\"$cmd\""
  sleep "$gap"
  f=/tmp/atc.$$
  cat "$TTY" > "$f" &
  cp=$!
  sleep 1
  printf '%s\r' "$line" > "$TTY"
  sleep "$wait"
  kill "$cp" 2>/dev/null
  RESP=$(grep -v '^$' "$f" | sed -n '/^'"$line"'/,$p' | grep -v '^'"$line"'$')
  rm -f "$f"
  say "TX[$line] -> $(echo "$RESP" | tr '\n' '|')"
}

# sw_extract <field-index> — parse '+CSIM: n,"HEX"' SW1SW2 (last 2 bytes of HEX)
last_sw() { echo "$RESP" | grep -o '+CSIM: [0-9]*,"[0-9A-Fa-f]*"' | sed 's/.*"\([0-9A-Fa-f]*\)".*/\1/' | tail -1 | tr 'a-f' 'A-F'; }

RESULT_C0="NOT-TESTED"; RESULT_FCI="NOT-TESTED"; RESULT_CASE="NOT-TESTED"; LEAKED=0; OPENED=""

close_channel() {
  ch=$1
  csim "$(printf '007080%02X' "$ch")" 3 1
  case "$RESP" in *ERROR*) LEAKED=$((LEAKED+1)); say "close ch$ch: AT ERROR (leak — known firmware class)";; *) say "close ch$ch: ok";; esac
}

say "=== corrected C0 battery start (tty=$TTY cycles=$N) ==="
say "controls:"
printf '\rAT\r' > "$TTY"; sleep 1   # plain AT wake/drain (never through csim())

# --- basic-channel controls ---
csim '00A40000023F00' 3 1          # SELECT MF by FID on ch0 (known-good cell)
csim '00A4000C022FE2' 3 1          # SELECT EF ICCID by FID, P2=0C (known-good cell)
sw=$(last_sw)
case "$sw" in
  61*)
    le=${sw#61}
    csim "00C00000$le" 3 1         # C0 on BASIC channel — control cell
    c0sw=$(last_sw)
    case "$c0sw" in *9000|9000) RESULT_C0="BASIC-CHANNEL-OK";; *) RESULT_C0="BASIC-CHANNEL-FAIL($c0sw)";; esac
    ;;
  9000) say "basic channel: no 61xx offered (P2=0C — expected)";;
  *) say "basic channel SELECT unexpected SW: $sw";;
esac

# --- hex-case control (correction 4 discriminator; pySim gerrit 8e15c18 shape) ---
# Same SELECT-by-FID as above, LOWERCASE hex. Case-rejecting Qualcomm-class
# modems answer bare ERROR (fast-fail); case-tolerant ones answer normally.
csim '00a4000c022fe2' 3 1
lowsw=$(last_sw)
case "$lowsw" in
  *9000|*61*) RESULT_CASE="LOWERCASE-ACCEPTED($lowsw)";;
  *) RESULT_CASE="LOWERCASE-REJECTED($(echo "$RESP" | head -2 | tr '\n' '|' | cut -c1-40))";;
esac
say "hex-case control: lowercase SELECT -> $RESULT_CASE"

i=0
while [ "$i" -lt "$N" ]; do
  i=$((i+1))
  say "--- cycle $i ---"
  csim '0070000001' 4 1            # MANAGE CHANNEL open
  # open success = [channel byte][90][00] e.g. 029000 (findings/probe lineage)
  chhex=$(last_sw | sed -n 's/^\([0-9A-F]\{2\}\)9000$/\1/p')
  if [ -n "$chhex" ]; then ch=$(printf '%d' "0x$chhex" 2>/dev/null) || ch=0; else ch=0; fi
  if [ "$ch" -lt 1 ] || [ "$ch" -gt 19 ]; then
    say "open refused/exhausted (SW $(last_sw)) — stopping cycles; CFUN recycle clears leaks (verified class)"
    break
  fi
  OPENED="$OPENED $ch"
  CLA=$(cla_for_channel "$ch")
  say "allocated channel $ch -> CLA $CLA"

  # ISD-R SELECT by AID, P2=0C, on the ALLOCATED channel
  csim "${CLA}A4040C10${ISDR}" 4 1
  sw=$(last_sw)
  say "ISD-R SELECT (P2=0C) SW: $sw"
  case "$sw" in
    61*)
      le=${sw#61}
      csim "${CLA}C00000$le" 5 1   # THE DECISIVE CELL: genuine INS C0
      c0sw=$(last_sw)
      say "GET RESPONSE (INS C0, ch$ch, Le=$le) SW: $c0sw"
      case "$c0sw" in
        *9000) RESULT_C0="ALLOCATED-CHANNEL-OK";;
        *) RESULT_C0="ALLOCATED-CHANNEL-FAIL($c0sw)";;
      esac
      ;;
    9000) say "no 61xx after P2=0C SELECT (9000 — coherent-basic class)";;
    *) say "ISD-R SELECT unexpected SW: $sw (6A82 would echo the old inline-FCI row)";;
  esac

  # gap sensitivity on the decisive path (only if 61xx is offered again)
  csim "${CLA}A4040C10${ISDR}" 4 8   # 8 s gap before this exchange
  sw2=$(last_sw)
  case "$sw2" in
    61*)
      le2=${sw2#61}
      csim "${CLA}C00000$le2" 5 0    # immediate follow-up
      say "gap-variant C0 SW: $(last_sw)"
      ;;
  esac

  # inline-FCI form re-test on the ALLOCATED channel (old 6A82 row)
  csim "${CLA}A4040010${ISDR}00" 4 1
  fsw=$(last_sw)
  case "$fsw" in
    9000|61*) RESULT_FCI="OK($fsw)";;
    6A82) RESULT_FCI="6A82-CONFIRMED";;
    *) RESULT_FCI="OTHER($fsw)";;
  esac
  say "inline-FCI SELECT (P2=00 Le=00) SW: $fsw -> $RESULT_FCI"

  close_channel "$ch"
done

# --- hygiene: attempt closes of everything we opened (both forms already tried) ---
for ch in $OPENED; do close_channel "$ch"; done

say "=== VERDICT ==="
say "C0: $RESULT_C0"
say "hex-case: $RESULT_CASE"
say "inline-FCI: $RESULT_FCI"
say "leaked closes: $LEAKED (channels:$OPENED)"
say "log: $LOG"
case "$RESULT_C0" in
  *OK*) echo "session-state hypothesis: C0 AVAILABLE — AT-class LPA sessions plausible; re-rank architecture options (issue #32)"; exit 0;;
  *FAIL*|NOT-TESTED) echo "session-state hypothesis: C0 unavailable/untested on allocated channels — mechanism evidence restored-with-instruments or inconclusive (issue #32)"; exit 1;;
esac
