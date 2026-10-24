#!/bin/sh
# validate.sh — chain entry for the split validation gate.
#
# OLD (v4.0): one monolithic hardware gate (direct-AT probe with the SYSTEM
# pcscd stopped, 300x/150x PC/SC loads, optional lpac loop). That gate
# required stopping the system pcscd and hardcoding a router SSH target —
# both now forbidden practice for this lane.
#
# NEW: two focused suites, chained (sacrificial must qualify first):
#   1. validate-sacrificial.sh — zero-risk class-A transport qualification
#      of a disposable SIM (SELECT by FID, EF reads via logical channels)
#   2. validate-esimgate.sh   — pre-provisioning read-only eSIM gate (EID,
#      chipInfo, ISD-R readiness); runs only if (1) was green, otherwise
#      its cases are reported BLOCKED (prerequisite) — never silently
#      green, never silently skipped.
#
# Default is DRY-RUN against the local mock modem (no hardware, no pcscd):
#   ./tests/hardware/validate.sh
# Hardware sessions run through lane-runner.sh (private pcscd lane, reader
# by FRIENDLYNAME, card-safety declarations). Env passthrough: everything
# validate-sacrificial.sh / validate-esimgate.sh read.
#
# Exit: 0 chain green · 1 FAIL anywhere · 2 SKIP/BLOCKED · 3 infra error.

set -u

HERE=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
REPORT_DIR="${REPORT_DIR:-$HERE/reports}"

say() { printf '\n=== %s ===\n' "$*"; }

say "chain 1/2: validate-sacrificial.sh"
"$HERE/validate-sacrificial.sh"
SAC=$?

say "chain 2/2: validate-esimgate.sh"
if [ "$SAC" -ne 0 ]; then
    echo "prerequisite NOT green (exit $SAC) — esimgate cases are BLOCKED, not executed"
    RESULTS=$(mktemp "${TMPDIR:-/tmp}/esimgate-cases.XXXXXX")
    MODEMIFD_DRY_RUN="${MODEMIFD_DRY_RUN:-1}"
    if [ "$MODEMIFD_DRY_RUN" = "1" ]; then
        MODE=dry-run
    else
        MODE=hw
    fi
    python3 "$HERE/suite.py" esimgate \
        --block-all "prerequisite validate-sacrificial did not pass (exit $SAC)" \
        --results "$RESULTS" || true
    python3 "$HERE/report.py" --suite validate-esimgate --mode "$MODE" \
        --results "$RESULTS" --out-dir "$REPORT_DIR" \
        --meta "blocked_by=validate-sacrificial exit $SAC" || true
    echo "results (kept): $RESULTS"
    exit "$SAC"
fi

"$HERE/validate-esimgate.sh"
