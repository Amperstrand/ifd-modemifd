#!/bin/sh
# validate.sh — pre-provisioning validation gate for the modemifd reader.
# Run this BEFORE any one-shot eSIM profile download through the reader.
# Exit 0 = green light. Any phase failure = do NOT attempt provisioning.
#
# Phases (EXCLUSIVE access per phase — see README.md for why):
#   1. Direct-AT channel probe with pcscd STOPPED (clean tty)
#   2. PC/SC small-APDU sustained load (SELECT MF x300)
#   3. PC/SC BPP-sized APDU load (224-byte SELECT x150)
#   4. ISD-R session loop (lpac chip info x5) when lpac is installed
#
# Env overrides: ROUTER_SSH (default root@192.168.1.1), AT_TTY
# (default /dev/ttyUSB2 — the RG502Q AT port SHUFFLES across power cycles:
# re-detect before trusting the default).

set -u
ROUTER_SSH="${ROUTER_SSH:-root@192.168.1.1}"
AT_TTY="${AT_TTY:-/dev/ttyUSB2}"
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$HERE/../.." && pwd)"
# Deterministic local target dir (immune to global CARGO_TARGET_DIR pins)
export CARGO_TARGET_DIR="$REPO_ROOT/target"
LOADTEST="$REPO_ROOT/target/release/examples/loadtest"
rc=0

say() { echo; echo "=== $* ==="; }

say "phase 0: build + reader presence"
(cd "$REPO_ROOT" && cargo build --release --examples) || exit 1
"$LOADTEST" --count 1 --mode small || rc=1

say "phase 1: direct-AT channel probe (pcscd STOPPED for exclusive tty; informational)"
sudo -n systemctl stop pcscd.service pcscd.socket 2>/dev/null
scp -O -q "$HERE/at_channel_probe.sh" "$ROUTER_SSH":/tmp/ \
  && ssh -o BatchMode=yes "$ROUTER_SSH" "sh /tmp/at_channel_probe.sh $AT_TTY 10" \
  || echo "NOTE: native CCHO/CGLA/CCHC unusable on this firmware (known RG502Q-EA: CGLA responses swallowed) — pcscd/CSIM transport is the path"
sudo -n systemctl start pcscd.socket pcscd.service 2>/dev/null
sleep 2

say "phase 2: PC/SC sustained load, small APDUs (300x)"
"$REPO_ROOT/target/release/examples/loadtest" --count 300 --mode small || rc=1

say "phase 3: PC/SC BPP-sized APDUs (150x, expect 6A82 passes)"
"$REPO_ROOT/target/release/examples/loadtest" --count 150 --mode large || rc=1

say "phase 4: ISD-R session loop (lpac chip info x5, optional)"
if command -v lpac >/dev/null 2>&1; then
  for i in 1 2 3 4 5; do
    t0=$(date +%s)
    out=$(LPAC_APDU=pcsc lpac chip info 2>&1)
    t1=$(date +%s)
    echo "$out" | grep -q '"code":0' && echo "chip info $i: OK ($((t1-t0))s)" \
      || { echo "chip info $i: FAIL: $out"; rc=1; }
  done
else
  echo "lpac not on PATH — skipped"
fi

say "verdict"
if [ "$rc" -eq 0 ]; then
  echo "GREEN — transport validated; one-shot provisioning may proceed."
else
  echo "RED — fix the failing phase before attempting any one-shot download."
fi
exit $rc
