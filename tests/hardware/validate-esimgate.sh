#!/bin/sh
# validate-esimgate.sh — pre-provisioning READ-ONLY gate for an eSIM seat.
#
# Runs ONLY non-mutating readiness checks before any one-shot profile
# operation is considered:
#   ESG-001..003  logical-channel open -> SELECT ISD-R by public AID
#                 (non-authenticating, doctrine-sanctioned) -> close
#   ESG-004       lpac chip info xN (ES10b GetEID / euiccInfo): EID captured
#                 and cross-checked for stability. SKIPs when lpac is absent
#                 or the backend is the dry-run mock.
#
# The provisioning gate stays CLOSED unless mode=hw AND every case PASSed.
# A dry-run report can never green-light provisioning (enforced by
# report.py).
#
# CARD SAFETY (session-2 directive): the production 9eSIM must NEVER be the
# target of this gate. Hardware runs require MODEMIFD_CARD declaring the
# actual seated card (e.g. esim-test / blank-euicc); any value naming the
# 9eSIM is refused by suite.py before a single APDU is sent.
#
# Env: MODEMIFD_DRY_RUN(1) MODEMIFD_BACKEND(direct|pcsc) MODEMIFD_ENDPOINT
#      MODEMIFD_READER MODEMIFD_CARD LPAC_REPEATS(3) SAC_TIMEOUT_MS(3000)
#      REPORT_DIR(tests/hardware/reports)
# Exit: 0 all PASS (hw mode: gate may open) · 1 any FAIL · 2 SKIP/BLOCKED
#       · 3 infra error.

set -u

HERE=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
MODEMIFD_DRY_RUN="${MODEMIFD_DRY_RUN:-1}"
MODEMIFD_BACKEND="${MODEMIFD_BACKEND:-}"
MODEMIFD_ENDPOINT="${MODEMIFD_ENDPOINT:-127.0.0.1:17002}"
MODEMIFD_READER="${MODEMIFD_READER:-}"
MODEMIFD_CARD="${MODEMIFD_CARD:-}"
LPAC_REPEATS="${LPAC_REPEATS:-3}"
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

RESULTS=$(mktemp "${TMPDIR:-/tmp}/esimgate-cases.XXXXXX")
SUITERC=0
REPORTRC=0

set -- esimgate --backend "$MODEMIFD_BACKEND" \
    --lpac-repeats "$LPAC_REPEATS" --timeout-ms "$SAC_TIMEOUT_MS" --results "$RESULTS"
if [ "$MODEMIFD_BACKEND" = "direct" ]; then
    set -- "$@" --endpoint "$MODEMIFD_ENDPOINT"
else
    [ -n "$MODEMIFD_CARD" ] && set -- "$@" --card-decl "$MODEMIFD_CARD"
    [ -n "$MODEMIFD_READER" ] && set -- "$@" --reader "$MODEMIFD_READER"
fi

say "suite: esimgate (backend $MODEMIFD_BACKEND, lpac repeats $LPAC_REPEATS)"
python3 "$HERE/suite.py" "$@" || SUITERC=$?
echo "results (kept): $RESULTS"

say "report"
python3 "$HERE/report.py" --suite validate-esimgate --mode "$MODE" \
    --results "$RESULTS" --out-dir "$REPORT_DIR" \
    --meta "endpoint=$MODEMIFD_ENDPOINT" \
    --meta "reader=${MODEMIFD_READER:-(n/a: direct backend)}" \
    --meta "lpac_repeats=$LPAC_REPEATS" \
    --meta "card=${MODEMIFD_CARD:-undeclared (dry-run)}" \
    || REPORTRC=$?

if [ "$SUITERC" -ne 0 ]; then
    exit "$SUITERC"
fi
exit "$REPORTRC"
