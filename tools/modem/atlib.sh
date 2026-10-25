# atlib.sh — POSIX-sh modem AT I/O for OpenWrt (NR7101/RG502Q-EA class).
# Sources: the 2026-10-25 session (eight rewrites of these patterns; the
# lessons are load-bearing — see docs/MODEM-REFRESH-POLICY.md).
# Never hardcode the AT port. Never use blocking reads without a killer.
# Detect by FUNCTION (CPIN answers), not bare AT (several ports echo OK).

# at_read <tty> <cmd> [wait_s] — reader-backed exchange, echo-correlated.
at_read() {
  _tty=$1; _cmd=$2; _wait=${3:-3}; _f=$(mktemp)
  cat "$_tty" > "$_f" & _p=$!
  sleep 1
  printf '%s\r' "$_cmd" > "$_tty"
  sleep "$_wait"
  kill $_p 2>/dev/null
  RESP=$(grep -v '^$' "$_f" | sed -n '/^'"$_cmd"'/,$p' | grep -v '^'"$_cmd"'$')
  rm -f "$_f"; echo "$RESP"
}

# at_func_port — echo the first port whose AT COMMAND parser works
# (AT+CPIN? answered by CPIN: or +CME — both prove the parser; silence
# or bare-AT-OK do not). Empty string if none.
at_func_port() {
  for _t in /dev/ttyUSB0 /dev/ttyUSB1 /dev/ttyUSB2 /dev/ttyUSB3; do
    [ -c "$_t" ] || continue
    at_read "$_t" 'AT+CPIN?' 3 | grep -qE 'CPIN:|\+CME' && { echo "$_t"; return 0; }
  done
  return 1
}

# at_wait_func_port <seconds> — poll for the functional port (post-reset
# settle). Echo the port or fail.
at_wait_func_port() {
  _deadline=$(( $(date +%s) + ${1:-120} ))
  while [ "$(date +%s)" -lt "$_deadline" ]; do
    _t=$(at_func_port) && { echo "$_t"; return 0; }
    sleep 10
  done
  return 1
}
