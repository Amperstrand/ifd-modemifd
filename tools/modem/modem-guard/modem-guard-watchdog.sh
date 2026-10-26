#!/bin/sh
# modem-guard-watchdog — procd daemon: probe the modem's functional port,
# escalate the merged refresh ladder (docs/MODEM-REFRESH-POLICY.md) with
# budget + backoff, alert-and-stop at the top (the physical cut stays human).
# Patterns adapted from vjt/quectel-5g-tools 5g-watchdog, 4IceG
# lite-watchdog, and the watchcat -W lesson (never let a probe hang).
. /usr/libexec/atlib.sh 2>/dev/null || . "$(dirname "$0")/../atlib.sh"

PROBE_INTERVAL=60
FAIL_THRESHOLD=3
COOLDOWN_OK=300
COOLDOWN_FAIL=300
COOLDOWN_MAX=1800
STATE=/var/run/modem-guard.state
[ -f /etc/config/modem-guard ] && . /etc/config/modem-guard 2>/dev/null

consec=0; cooldown=$COOLDOWN_FAIL
logger_it() { logger -t modem-guard "$1"; echo "$(date -u +%FT%TZ) $1" >> /tmp/modem-guard.log; }

probe() {
  # functional probe with hard timeout: CPIN answered by parser = healthy
  T=$(timeout_run 10 at_func_port 2>/dev/null || at_func_port)
  [ -n "$T" ]
}

timeout_run() { # absent-timeout-applet-safe probe wrapper
  ( "$@" & p=$!; d=$(( $(date +%s) + 10 )); while kill -0 $p 2>/dev/null && [ $(date +%s) -lt $d ]; do sleep 1; done
    kill $p 2>/dev/null ) ; wait
}

escalate() {
  rung=$1
  case $rung in
    1) logger_it "rung1: settle 90s"; sleep 90;;
    2) if [ -e /dev/cdc-wdm0 ]; then uqmi -d /dev/cdc-wdm0 --uim-power-off --uim-slot=1; sleep 3; uqmi -d /dev/cdc-wdm0 --uim-power-on --uim-slot=1; sleep 20; logger_it "rung2: UIM power-cycle"; else logger_it "rung2: n/a (ECM)"; fi;;
    3) T=$(at_func_port) && { at_read "$T" 'AT+CFUN=0' 5 >/dev/null; sleep 8; at_read "$T" 'AT+CFUN=1' 5 >/dev/null; sleep 25; logger_it "rung3: gentle CFUN"; };;
    4) if [ -x /usr/sbin/gpiopoke_load ]; then logger_it "rung4: gpiopoke rail cycle"; gpiopoke_load cycle; else logger_it "rung4: gpiopoke absent — ALERT"; fi;;
    *) logger_it "STOP: all rungs exhausted — human action required (20s power cut). No further resets."; return 9;;
  esac
}

rung=1; last_action=0
while :; do
  if probe; then
    [ $consec -gt 0 ] && logger_it "recovered after $consec failed probes"
    echo "healthy $(date +%s)" > "$STATE"; consec=0; rung=1; cooldown=$COOLDOWN_OK
  else
    consec=$((consec+1))
    logger_it "probe failed ($consec/$FAIL_THRESHOLD)"
    if [ $consec -ge $FAIL_THRESHOLD ] && [ $(( $(date +%s) - last_action )) -ge $cooldown ]; then
      escalate $rung; rc=$?
      [ $rc = 9 ] && { echo "stopped-human-required" > "$STATE"; exit 0; }
      last_action=$(date +%s); rung=$((rung+1)); cooldown=$(( cooldown * 2 )); [ $cooldown -gt $COOLDOWN_MAX ] && cooldown=$COOLDOWN_MAX
    fi
  fi
  sleep $PROBE_INTERVAL
done
