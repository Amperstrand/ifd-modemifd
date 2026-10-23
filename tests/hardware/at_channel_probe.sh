#!/bin/sh
# at_channel_probe.sh — modem-side logical-channel capability + reliability probe.
# Runs ON the router (POSIX sh) against the modem AT port directly.
# The caller (validate.sh) MUST stop pcscd first: pcscd's presence poller
# writes to the same tty through the socat bridge and contaminates results.
#
# Usage: at_channel_probe.sh [tty] [cycles]
#
# Card-safety: only non-authenticating operations (CCHO select of the public
# GSMA ISD-R AID, one filesystem SELECT, CCHC close). Never loop
# secure-channel / STORE DATA class commands — see tests/hardware/README.md.

TTY="${1:-/dev/ttyUSB2}"
N="${2:-10}"
# Public GSMA ISD-R AID (standard, published in SGP.22 and lpac sources)
ISDR="A0000005591010FFFFFFFF8900000100"

at_once() {
  f=/tmp/atc.$$
  cat "$TTY" > "$f" &
  cp=$!
  sleep 1
  printf '%s\r' "$1" > "$TTY"
  sleep "${2:-3}"
  kill $cp 2>/dev/null
  RESP=$(grep -v '^$' "$f")
  rm -f "$f"
}

echo "=== capability probes (bare OK = supported on Quectel; no range line) ==="
at_once 'AT+CCHO=?' 3; echo "$RESP" | head -3
at_once 'AT+CCHC=?' 3; echo "$RESP" | head -3
at_once 'AT+CGLA=?' 3; echo "$RESP" | head -3

echo "=== $N x (CCHO -> CGLA SELECT MF -> CCHC) cycles ==="
ok=0; fail=0
i=0
while [ $i -lt $N ]; do
  i=$((i+1))
  at_once "AT+CCHO=\"$ISDR\"" 5
  ch=$(echo "$RESP" | grep -o '+CCHO: *[0-9]*' | grep -o '[0-9]*$')
  if [ -z "$ch" ]; then
    echo "cycle $i: CCHO FAIL: $(echo "$RESP" | head -2 | tr '\n' ' ')"
    fail=$((fail+1))
    sleep 2
    continue
  fi
  at_once "AT+CGLA=$ch,14,\"00A40000023F00\"" 4
  gla=$(echo "$RESP" | grep -c '+CGLA:')
  at_once "AT+CCHC=$ch" 4
  chc=$(echo "$RESP" | grep -c 'OK')
  if [ "$gla" -ge 1 ] && [ "$chc" -ge 1 ]; then
    ok=$((ok+1))
  else
    fail=$((fail+1))
    echo "cycle $i: CGLA=$gla CCHC=$chc RAW: $(echo "$RESP" | head -2 | tr '\n' ' ')"
  fi
done
echo "RESULT: $ok/$N cycles clean"
[ "$ok" -eq "$N" ] || exit 1
