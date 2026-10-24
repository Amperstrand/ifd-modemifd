#!/bin/sh
# validate-sacrificial.sh — sacrificial-SIM qualification suite (class-A only).
#
# Zero-risk, strictly read-only transport qualification of a DISPOSABLE SIM:
#   SAC-001..003  basic-channel SELECT-by-FID + modem drop fingerprints
#                 (A4/P1=04 and B0 are dropped on the basic channel by the
#                 RG502Q-EA modem — README "Modem findings"; the drop itself
#                 is the expected PASS, so the probes never touch the card)
#   SAC-004..008  logical-channel cycle: MANAGE CHANNEL open, SELECT MF and
#                 EF.ICCID by FID, READ BINARY (fixture fingerprint), close
#   SAC-009       post-cycle basic-channel sanity select
#
# Default is DRY-RUN (MODEMIFD_DRY_RUN=1): a mock modem on 127.0.0.1:17002
# is started automatically if nothing is listening. No hardware, no pcscd.
#
# Hardware runs (pcsc backend) additionally require: MODEMIFD_READER (the
# FRIENDLYNAME — the first reader is never implicitly taken), MODEMIFD_CARD
# (must equal "sacrificial"; any value naming the 9eSIM is refused), and a
# private pcscd lane via lane-runner.sh (never the system pcscd).
#
# Env: MODEMIFD_DRY_RUN(1) MODEMIFD_BACKEND(direct|pcsc) MODEMIFD_ENDPOINT
#      MODEMIFD_READER MODEMIFD_CARD SAC_REPEATS(1) SAC_TIMEOUT_MS(3000)
#      REPORT_DIR(tests/hardware/reports)
# Exit: 0 all PASS · 1 any FAIL · 2 SKIP/BLOCKED · 3 infra error.

set -u

HERE=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
MODEMIFD_DRY_RUN="${MODEMIFD_DRY_RUN:-1}"
MODEMIFD_BACKEND="${MODEMIFD_BACKEND:-}"
MODEMIFD_ENDPOINT="${MODEMIFD_ENDPOINT:-127.0.0.1:17002}"
MODEMIFD_READER="${MODEMIFD_READER:-}"
MODEMIFD_CARD="${MODEMIFD_CARD:-}"
SAC_REPEATS="${SAC_REPEATS:-1}"
SAC_TIMEOUT_MS="${SAC_TIMEOUT_MS:-3000}"
REPORT_DIR="${REPORT_DIR:-$HERE/reports}"

MOCK_PID=""
# shellcheck disable=SC2317  # invoked only via trap
cleanup() {
    if [ -n "$MOCK_PID" ]; then
        kill "$MOCK_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT INT TERM

say() { printf '\n=== %s ===\n' "$*"; }

if [ -z "$MODEMIFD_BACKEND" ]; then
    if [ "$MODEMIFD_DRY_RUN" = "1" ]; then
        MODEMIFD_BACKEND=direct
    else
        MODEMIFD_BACKEND=pcsc
    fi
fi

MODE=hw
case "$MODEMIFD_ENDPOINT" in
    127.0.0.1:*|localhost:*) IS_LOOPBACK=1 ;;
    *) IS_LOOPBACK=0 ;;
esac
if [ "$MODEMIFD_BACKEND" = "direct" ] && [ "$IS_LOOPBACK" = 1 ]; then
    MODE=dry-run
fi

if [ "$MODE" = "dry-run" ]; then
    if python3 "$HERE/suite.py" --probe-endpoint "$MODEMIFD_ENDPOINT" >/dev/null 2>&1; then
        say "dry-run: reusing modem already listening on $MODEMIFD_ENDPOINT"
    else
        say "dry-run: starting mock modem on $MODEMIFD_ENDPOINT"
        mkdir -p "$REPORT_DIR"
        python3 "$HERE/mock_modem.py" --listen "$MODEMIFD_ENDPOINT" \
            --log "$REPORT_DIR/mock-modem.log" &
        MOCK_PID=$!
        i=0
        until python3 "$HERE/suite.py" --probe-endpoint "$MODEMIFD_ENDPOINT" >/dev/null 2>&1; do
            i=$((i + 1))
            [ "$i" -ge 20 ] && break
            sleep 0.3
        done
    fi
fi

RESULTS=$(mktemp "${TMPDIR:-/tmp}/sacrificial-cases.XXXXXX")
SUITERC=0
REPORTRC=0

set -- sacrificial --backend "$MODEMIFD_BACKEND" \
    --repeats "$SAC_REPEATS" --timeout-ms "$SAC_TIMEOUT_MS" --results "$RESULTS"
if [ "$MODEMIFD_BACKEND" = "direct" ]; then
    set -- "$@" --endpoint "$MODEMIFD_ENDPOINT"
else
    [ -n "$MODEMIFD_CARD" ] && set -- "$@" --card-decl "$MODEMIFD_CARD"
    [ -n "$MODEMIFD_READER" ] && set -- "$@" --reader "$MODEMIFD_READER"
fi

say "suite: sacrificial (backend $MODEMIFD_BACKEND, repeats $SAC_REPEATS)"
python3 "$HERE/suite.py" "$@" || SUITERC=$?
echo "results (kept): $RESULTS"

say "report"
python3 "$HERE/report.py" --suite validate-sacrificial --mode "$MODE" \
    --results "$RESULTS" --out-dir "$REPORT_DIR" \
    --meta "endpoint=$MODEMIFD_ENDPOINT" \
    --meta "reader=${MODEMIFD_READER:-(n/a: direct backend)}" \
    --meta "repeats=$SAC_REPEATS" \
    --meta "card=${MODEMIFD_CARD:-undeclared (dry-run)}" \
    || REPORTRC=$?

if [ "$SUITERC" -ne 0 ]; then
    exit "$SUITERC"
fi
exit "$REPORTRC"
