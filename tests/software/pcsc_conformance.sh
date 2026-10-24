#!/usr/bin/env bash
# pcscd-level software conformance round for ifd-modemifd.
#
# Closes issue #7 (by-value SCARD_IO_HEADER ABI smoke, the v3.2 evidence
# gap) and pre-runs the software half of issue #8 (GET_FEATURE, ATR,
# warm reset, escape rejection, multi-reader, TX_GAP env) against a
# REAL pcscd — no hardware. Everything is hermetic:
#
#   - two mockmodem instances play the modems (loopback TCP)
#   - pcscd runs PRIVATELY inside a mount namespace with a tmpfs over
#     /run/pcscd (the system pcscd and its socket are never touched;
#     teardown kills strictly via the namespace-private pidfile — never
#     by process name, which could hit a system daemon)
#   - USB hotplug readers on the bench (e.g. an ACS ACR1252) may also
#     appear in the private instance; the battery ignores them
#   - the driver .so is the freshly built one; IFD_MODMIFD_TX_GAP_MS=60
#     rides on the pcscd process so the pacing check runs fast
#
# Usage: tests/software/pcsc_conformance.sh   (sudo is used only for
# unshare -m; every stage is timeout-bounded, and no path exits while
# the namespaced pcscd is still alive)

set -u

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
TARGET=$(cargo metadata --no-deps --format-version 1 \
    | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
LIB="$TARGET/release/libifd_modemifd.so"
EX="$TARGET/release/examples"

# --lib first: building only the examples puts the cdylib under
# release/deps/, not at the release/libifd_modemifd.so path $LIB uses
cargo build --release --lib >/dev/null
cargo build --release --example mockmodem --example pcsc_conformance >/dev/null

WORK=$(mktemp -d /tmp/modemifd-pcsc-round.XXXXXX)
MOCK_PIDS=""
MOCK_PORT=""

cleanup() {
    for p in $MOCK_PIDS; do kill "$p" 2>/dev/null || true; done
    rm -rf "$WORK"
}
trap cleanup EXIT

# Sets MOCK_PORT; appends the child pid to MOCK_PIDS. Must NOT run in a
# command substitution — the subshell would orphan the background
# mockmodem and the parent would lose its pid (leaked children hold the
# session's pipes open).
start_mock() { # $1 = name
    local name=$1
    local log="$WORK/$name.port"
    "$EX/mockmodem" 0 >"$log" 2>&1 &
    MOCK_PIDS="$MOCK_PIDS $!"
    for _ in $(seq 50); do
        [ -s "$log" ] && break
        sleep 0.1
    done
    MOCK_PORT=$(sed -n 's/^PORT //p' "$log")
}

start_mock a
PORT_A=$MOCK_PORT
start_mock b
PORT_B=$MOCK_PORT
echo "mock modems: A=127.0.0.1:$PORT_A B=127.0.0.1:$PORT_B"

READERS="$WORK/readers.conf.d"
mkdir -p "$READERS"
cat >"$READERS/a.conf" <<EOF
FRIENDLYNAME MIFD_A
DEVICENAME 127.0.0.1:$PORT_A
LIBPATH $LIB
CHANNELID 1
EOF
cat >"$READERS/b.conf" <<EOF
FRIENDLYNAME MIFD_B
DEVICENAME 127.0.0.1:$PORT_B
LIBPATH $LIB
CHANNELID 1
EOF

# --- private pcscd + battery inside one mount namespace --------------
# args: $1=WORK $2=readers dir $3=examples dir. Runs under POSIX sh:
# no bashisms (PIPESTATUS, arrays). The EXIT trap kills the namespaced
# pcscd by its $! pid baked in at definition time — the pidfile only
# appears late in startup, and a leaked namespaced pcscd holds the
# session's pipes open forever.
INNER='
    mount -t tmpfs tmpfs /run/pcscd
    IFD_MODMIFD_TX_GAP_MS=60 \
    IFD_MODMIFD_DEBUG=1 \
    IFD_MODMIFD_LOG_PATH="$1/driver.log" \
        pcscd --foreground --disable-polkit --config "$2" \
        >"$1/pcscd.log" 2>&1 &
    trap "kill $! 2>/dev/null" EXIT INT TERM
    i=0
    while [ $i -lt 300 ] && [ ! -S /run/pcscd/pcscd.comm ]; do
        sleep 0.1
        i=$((i + 1))
    done
    if [ ! -S /run/pcscd/pcscd.comm ]; then
        echo "FAIL pcscd_socket — never appeared; pcscd.log tail:" | tee -a "$1/round.log"
        tail -20 "$1/pcscd.log" >&2
        exit 3
    fi
    timeout 120 "$3/pcsc_conformance" >"$1/round.log" 2>&1
    rc=$?
    cat "$1/round.log"
    exit "$rc"
'

timeout 180 sudo unshare -m sh -c "$INNER" round "$WORK" "$READERS" "$EX"
rc=$?

if [ "$rc" -ne 0 ]; then
    keep=/tmp/modemifd-pcsc-round-failed
    rm -rf "$keep"
    mkdir -p "$keep"
    cp "$WORK"/round.log "$WORK"/pcscd.log "$WORK"/driver.log "$keep"/ 2>/dev/null || true
    echo "failure artifacts kept in $keep"
fi
echo "battery rc=$rc"
echo "artifacts: $WORK/round.log $WORK/pcscd.log $WORK/driver.log"
exit "$rc"
