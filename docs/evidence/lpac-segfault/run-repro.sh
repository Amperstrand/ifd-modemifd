#!/bin/bash
# run-repro.sh <lpac-binary> <outfile> <errfile> [mock extra args...]
# Starts mock_modem.py + socat PTY, runs lpac under timeout(1), cleans up by PID.
set -u
BIN="$1"; OUT="$2"; ERR="$3"; shift 3
D=/tmp/opencode/segfault-dig
PORT=17029
cd "$D" || exit 97
rm -f at0
python3 /home/ubuntu/src/ifd-modemifd/tests/hardware/mock_modem.py --listen 127.0.0.1:$PORT --log "${OUT%.out}-mock.jsonl" "$@" > mock-r.stdout 2>&1 &
MOCKPID=$!
sleep 0.5
socat PTY,raw,echo=0,link=$D/at0 TCP:127.0.0.1:$PORT > socat-r.stdout 2>&1 &
SOCATPID=$!
sleep 1
timeout 30 env LPAC_APDU=at_csim LPAC_APDU_AT_DEVICE=$D/at0 \
    ASAN_OPTIONS=detect_leaks=0 "$BIN" chip info > "$OUT" 2> "$ERR"
RC=$?
kill $SOCATPID $MOCKPID 2>/dev/null
echo "exit=$RC"
