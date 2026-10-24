#!/bin/sh
# lane-runner.sh — private-pcscd lane wrapper for the modem-hil suites.
#
# Lane isolation per the bench-proven recipe from ccid-firmware-rs AGENTS.md
# (2026-10-21): a socket-activated pcscd instance with its own reader.conf.d
# directory containing ONLY the modemifd entry, addressed exclusively via
# PCSCLITE_CSOCK_NAME. The system pcscd is never touched, stopped, or
# restarted by this lane.
#
#     systemd-socket-activate -l /run/pcscd-lanes/<lane>.sock -- \
#         /usr/sbin/pcscd --foreground -c /etc/reader.conf.d-lanes/<lane>
#     export PCSCLITE_CSOCK_NAME=/run/pcscd-lanes/<lane>.sock
#
# Reader selection is BY FRIENDLYNAME, at two layers: this runner waits for
# exactly FRIENDLYNAME to appear in the private instance, and suite.py
# refuses pcsc runs without an unambiguous --reader match. The first reader
# is never implicitly selected.
#
# Modes:
#   --dry-run (default) — NO pcscd, NO hardware: the suites run against the
#       local mock modem (loopback AT). Full software rehearsal.
#   hw (no flag) — private pcscd lane pointed at MODEMIFD_ADDR (the socat
#       bridge of the modem under test). Requires root-writable lane dirs
#       (or LANES_RUN_BASE/LANES_CONF_BASE overrides) and:
#         MODEMIFD_ADDR  — no default, ever (nothing dials 192.168.x implicitly)
#         MODEMIFD_CARD  — declaration of the seated card; values naming the
#                          9eSIM are refused (session-2 directive: the
#                          production 9eSIM is never a suite target)
#         HW_CONFIRM=YES_I_OWN_THE_HARDWARE — required when MODEMIFD_ADDR is
#                          not loopback (NR7101 is owned by another lane
#                          until the coordinator hands it over)
#   --plan — print the resolved hardware-mode commands; run nothing.
#
# Env: LANE(modem-hil) FRIENDLYNAME(NR7101_SIM_dut) MODEMIFD_ADDR
#      MODEMIFD_CARD SUITE(both|sacrificial|esimgate) SO_FILE PCSCD_BIN
#      SA_BIN LANES_RUN_BASE(/run/pcscd-lanes) LANES_CONF_BASE
#      (/etc/reader.conf.d-lanes) HW_CONFIRM REPORT_DIR + suite env
#      (SAC_REPEATS, LPAC_REPEATS, SAC_TIMEOUT_MS, MODEMIFD_ENDPOINT).
# Exit: suite ladder — 0 all PASS · 1 FAIL · 2 SKIP/BLOCKED · 3 infra error.

set -u

HERE=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(CDPATH='' cd -- "$HERE/../.." && pwd)
# Deterministic local target dir (immune to the global ~/.cargo/config.toml
# target-dir pin — same workaround the old validate.sh carried)
export CARGO_TARGET_DIR="$REPO_ROOT/target"
LANE="${LANE:-modem-hil}"
FRIENDLYNAME="${FRIENDLYNAME:-NR7101_SIM_dut}"
MODEMIFD_ADDR="${MODEMIFD_ADDR:-}"
MODEMIFD_CARD="${MODEMIFD_CARD:-}"
MODEMIFD_ENDPOINT="${MODEMIFD_ENDPOINT:-127.0.0.1:17002}"
SUITE="${SUITE:-both}"
HW_CONFIRM="${HW_CONFIRM:-}"
PCSCD_BIN="${PCSCD_BIN:-/usr/sbin/pcscd}"
SA_BIN="${SA_BIN:-systemd-socket-activate}"
LANES_RUN_BASE="${LANES_RUN_BASE:-/run/pcscd-lanes}"
LANES_CONF_BASE="${LANES_CONF_BASE:-/etc/reader.conf.d-lanes}"
SO_FILE="${SO_FILE:-$REPO_ROOT/target/release/libifd_modemifd.so}"

DRYRUN=1
PLAN=0
for arg in "$@"; do
    case "$arg" in
        --dry-run) DRYRUN=1 ;;
        --plan) PLAN=1 ;;
        hw|--hw) DRYRUN=0 ;;
        *) printf 'lane-runner.sh: unknown arg %s\n' "$arg" >&2; exit 3 ;;
    esac
done

LANE_CONF_DIR="$LANES_CONF_BASE/$LANE"
LANE_SOCK="$LANES_RUN_BASE/$LANE.sock"
SA_PID=""

say() { printf '\n=== %s ===\n' "$*"; }

teardown() {
    if [ -n "$SA_PID" ]; then
        kill "$SA_PID" 2>/dev/null || true
        SA_PID=""
    fi
    pkill -f "systemd-socket-activate -l $LANE_SOCK" 2>/dev/null || true
    pkill -f "pcscd --foreground -c $LANE_CONF_DIR" 2>/dev/null || true
    rm -f "$LANE_SOCK" 2>/dev/null || true
}
# shellcheck disable=SC2317  # invoked only via trap
trap teardown EXIT INT TERM

case "$SUITE" in
    both|sacrificial|esimgate) ;;
    *) printf 'lane-runner.sh: SUITE must be both|sacrificial|esimgate (got %s)\n' "$SUITE" >&2
       exit 3 ;;
esac

run_suite_scripts() {
    case "$SUITE" in
        both) "$HERE/validate.sh" ;;
        sacrificial) "$HERE/validate-sacrificial.sh" ;;
        esimgate) "$HERE/validate-esimgate.sh" ;;
    esac
}

hw_preflight() {
    if [ -z "$MODEMIFD_ADDR" ]; then
        echo "lane-runner.sh: hw mode needs MODEMIFD_ADDR (the modem socat bridge host:port)." >&2
        echo "  There is NO default on purpose — this lane never dials a router implicitly." >&2
        exit 3
    fi
    case "$MODEMIFD_ADDR" in
        127.0.0.1:*|localhost:*) ;;
        *)
            if [ "$HW_CONFIRM" != "YES_I_OWN_THE_HARDWARE" ]; then
                echo "lane-runner.sh: MODEMIFD_ADDR=$MODEMIFD_ADDR is not loopback and" >&2
                echo "  HW_CONFIRM is not YES_I_OWN_THE_HARDWARE. The NR7101 modem is owned" >&2
                echo "  by another lane; confirm hardware ownership before any hw run." >&2
                exit 3
            fi
            ;;
    esac
    if [ -z "$MODEMIFD_CARD" ]; then
        echo "lane-runner.sh: hw mode needs MODEMIFD_CARD declaring the seated card" >&2
        echo "  (sacrificial suite requires MODEMIFD_CARD=sacrificial)." >&2
        exit 3
    fi
    case "$MODEMIFD_CARD" in
        *9esim*|*9ESIM*|*9eSIM*)
            echo "lane-runner.sh: refusing — MODEMIFD_CARD names the production 9eSIM." >&2
            echo "  The 9eSIM must never be a suite target (session-2 directive)." >&2
            exit 3
            ;;
    esac
    for bin in "$PCSCD_BIN" "$SA_BIN"; do
        if ! command -v "$bin" >/dev/null 2>&1; then
            echo "lane-runner.sh: required binary missing: $bin" >&2
            exit 3
        fi
    done
    if [ ! -f "$SO_FILE" ]; then
        say "building $SO_FILE"
        (CDPATH='' cd -- "$HERE/../.." && cargo build --release) || exit 3
    fi
    if ! mkdir -p "$LANES_RUN_BASE" "$LANE_CONF_DIR" 2>/dev/null; then
        echo "lane-runner.sh: cannot create lane dirs under $LANES_RUN_BASE / $LANE_CONF_DIR." >&2
        echo "  Run as root, or override LANES_RUN_BASE/LANES_CONF_BASE to writable paths." >&2
        exit 3
    fi
}

print_plan() {
    say "hardware-mode plan (resolved)"
    cat <<EOF
LANE_CONF_DIR   = $LANE_CONF_DIR
LANE_SOCK       = $LANE_SOCK
FRIENDLYNAME    = $FRIENDLYNAME
MODEMIFD_ADDR   = ${MODEMIFD_ADDR:-(unset — hw mode will refuse)}
MODEMIFD_CARD   = ${MODEMIFD_CARD:-(unset — hw mode will refuse)}
SO_FILE         = $SO_FILE
SUITE           = $SUITE

reader.conf ($LANE_CONF_DIR/modemifd.conf):
    FRIENDLYNAME $FRIENDLYNAME
    DEVICENAME $MODEMIFD_ADDR
    LIBPATH $SO_FILE
    CHANNELID 1

commands:
    systemd-socket-activate -l $LANE_SOCK -- $PCSCD_BIN --foreground -c $LANE_CONF_DIR &
    export PCSCLITE_CSOCK_NAME=$LANE_SOCK
    # wait for the reader BY NAME (never the first reader):
    #     suite.py --list-readers | grep -Fx '$FRIENDLYNAME'
    MODEMIFD_DRY_RUN=0 MODEMIFD_BACKEND=pcsc MODEMIFD_READER='$FRIENDLYNAME' \\
        MODEMIFD_CARD='$MODEMIFD_CARD' $HERE/validate.sh

teardown: kill the systemd-socket-activate pid; rm -f $LANE_SOCK
EOF
}

if [ "$PLAN" = 1 ]; then
    print_plan
    exit 0
fi

if [ "$DRYRUN" = 1 ]; then
    say "lane-runner: DRY-RUN (mock modem on $MODEMIFD_ENDPOINT, no pcscd, no hardware)"
    MODEMIFD_DRY_RUN=1 MODEMIFD_BACKEND=direct MODEMIFD_ENDPOINT="$MODEMIFD_ENDPOINT" \
        run_suite_scripts
    exit $?
fi

hw_preflight
say "lane-runner: hardware mode — private pcscd lane '$LANE'"
printf 'FRIENDLYNAME %s\nDEVICENAME %s\nLIBPATH %s\nCHANNELID 1\n' \
    "$FRIENDLYNAME" "$MODEMIFD_ADDR" "$SO_FILE" > "$LANE_CONF_DIR/modemifd.conf" || exit 3

# pcsc-lite >= 2.0 refuses to start when the SYSTEM /run/pcscd/pcscd.comm
# exists with a live pid (second-instance check runs before socket
# activation), so the AGENTS.md plain recipe dies on 2.x hosts. Isolate the
# instance in a private mount namespace with a tmpfs over /run/pcscd; the
# lane socket still lives in the real fs for clients.
# systemd-socket-activate STRIPS the environment for the activated service,
# so driver tuning (IFD_MODMIFD_*: debug log, ssh key/cmd, TX gap) must be
# re-injected explicitly via an env(1) wrapper around pcscd.
PCSC_MAJOR=$("$PCSCD_BIN" --version 2>/dev/null | sed -n 's/^pcsc-lite version \([0-9]*\)\..*/\1/p')
PCSC_ENV_CMD=""
for v in IFD_MODMIFD_DEBUG IFD_MODMIFD_LOG_PATH IFD_MODMIFD_LOG_APDU \
         IFD_MODMIFD_SSH_KEY IFD_MODMIFD_SSH_CMD IFD_MODMIFD_TX_GAP_MS; do
    eval "val=\${$v:-}"
    [ -n "$val" ] && PCSC_ENV_CMD="$PCSC_ENV_CMD $v='$val'"
done
[ -n "$PCSC_ENV_CMD" ] && PCSC_ENV_CMD="env$PCSC_ENV_CMD"

PCSC_LAUNCH="$PCSC_ENV_CMD '$PCSCD_BIN' --foreground -c '$LANE_CONF_DIR'"
if [ "${PCSC_MAJOR:-0}" -ge 2 ]; then
    echo "lane-runner: pcsc-lite $PCSC_MAJOR.x detected — namespace-isolated instance"
    unshare -m sh -c "mount -t tmpfs tmpfs /run/pcscd && exec '$SA_BIN' -l '$LANE_SOCK' \
-- sh -c \"exec $PCSC_LAUNCH\"" &
    SA_PID=$!
else
    "$SA_BIN" -l "$LANE_SOCK" -- sh -c "exec $PCSC_LAUNCH" &
    SA_PID=$!
fi
export PCSCLITE_CSOCK_NAME="$LANE_SOCK"

i=0
while [ ! -S "$LANE_SOCK" ]; do
    i=$((i + 1))
    [ "$i" -ge 40 ] && break
    sleep 0.25
done
chmod 666 "$LANE_SOCK" 2>/dev/null

say "waiting for reader '$FRIENDLYNAME' (by name, never first)"
i=0
until python3 "$HERE/suite.py" --list-readers 2>/dev/null | grep -q "^$FRIENDLYNAME"; do
    i=$((i + 1))
    if [ "$i" -ge 60 ]; then
        echo "lane-runner.sh: reader '$FRIENDLYNAME' never appeared on the private lane" >&2
        exit 3
    fi
    sleep 0.5
done
echo "reader up: $FRIENDLYNAME (sock $LANE_SOCK)"

MODEMIFD_DRY_RUN=0 MODEMIFD_BACKEND=pcsc MODEMIFD_READER="$FRIENDLYNAME" \
    MODEMIFD_CARD="$MODEMIFD_CARD" run_suite_scripts
