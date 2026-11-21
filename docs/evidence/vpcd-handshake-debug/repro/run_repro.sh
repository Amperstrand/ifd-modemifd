#!/usr/bin/env bash
# run_repro.sh — old-vs-new modem-vpicc agent against the patched-driver
# machine (mock_libifdvpcd.py) and a slow modem (slow_modem.py).
#
# Demonstrates the 2026-11-21 bench bug OFFLINE:
#   OLD agent (master d6c214d): handshake between connect and serve loop
#     -> the driver's ATR probe times out in its 2 s window every cycle
#     -> eject churn, zero answered probes, reconnect loop (= pcscd
#        "no smart card" on the bench).
#   NEW agent (this branch): handshake before connect -> first probe
#     answered in milliseconds, one connection, zero ejects.
#
# Usage:
#   ./run_repro.sh OLD_BIN NEW_BIN [DURATION_S]
# Results land in ../transcripts/{old,new}-driver.log (+ .summary).
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
OUT="$HERE/../transcripts"
mkdir -p "$OUT"

OLD_BIN="${1:?usage: run_repro.sh OLD_BIN NEW_BIN [DURATION_S]}"
NEW_BIN="${2:?usage: run_repro.sh OLD_BIN NEW_BIN [DURATION_S]}"
DUR="${3:-8}"

free_port() { python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()'; }

run_one() {  # run_one LABEL AGENT_BIN
  local label="$1" bin="$2"
  local modem_port vpcd_port
  modem_port="$(free_port)"
  vpcd_port="$(free_port)"
  python3 "$HERE/slow_modem.py" --listen "127.0.0.1:$modem_port" \
    --ate1-delay-ms 1500 --csim-test-delay-ms 1500 \
    >"$OUT/$label-modem.log" 2>&1 &
  local modem_pid=$!
  python3 "$HERE/mock_libifdvpcd.py" --listen "127.0.0.1:$vpcd_port" \
    --duration-s "$DUR" --log "$OUT/$label-driver.log" \
    >"$OUT/$label-driver.stdout" 2>&1 &
  local driver_pid=$!
  "$bin" --vpcd "127.0.0.1:$vpcd_port" --target "tcp://127.0.0.1:$modem_port" \
    --reconnect-ms 500 >/dev/null 2>&1 &
  local agent_pid=$!
  wait "$driver_pid" || true
  kill "$agent_pid" "$modem_pid" 2>/dev/null || true
  wait "$agent_pid" "$modem_pid" 2>/dev/null || true
  tail -1 "$OUT/$label-driver.log" | tee "$OUT/$label-driver.summary"
}

echo "== OLD agent: $OLD_BIN"
run_one old "$OLD_BIN"
echo "== NEW agent: $NEW_BIN"
run_one new "$NEW_BIN"
echo "transcripts in $OUT"
