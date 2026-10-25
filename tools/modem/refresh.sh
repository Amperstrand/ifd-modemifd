#!/bin/sh
# refresh.sh — the post-profile-operation modem refresh LADDER (executable
# policy; docs/MODEM-REFRESH-POLICY.md is the doctrine). Two wedges on
# 2026-10-25 bought this: rungs go gentle-first, CFUN=1,1 needs FORCE,
# and a reset budget is enforced.
#
# usage: refresh.sh <expected-iccid> [--force]
#   Verifies the active profile after each rung; stops at the first green.
#   Exit 0 = profile active (however we got there); 1 = wedge suspected
#   (recovery rungs printed); 2 = budget/policy refusal.

. "$(dirname "$0")/atlib.sh"
EXPECT_ICCID=$1; FORCE=$2
BUDGET_FILE=/tmp/modem-reset-budget   # volatile by design: budget clears on the power cuts that also clear the debt
bump_budget() { echo "$(date +%s)" >> "$BUDGET_FILE"; }
budget_count() { now=$(date +%s); awk -v n="$now" 'n - $1 < 3600' "$BUDGET_FILE" 2>/dev/null | wc -l; }

qccid() { T=$(at_func_port) || return 1; at_read "$T" 'AT+QCCID' 3 | grep -o '[0-9A-F]\{19,20\}' | head -1; }

say() { echo "$(date -u +%H:%M:%SZ) $*"; }
wedge_help() { say "WEDGE SUSPECTED. Recovery rungs in order:"; say " 1. AT+CFUN=1,1 is NOT recovery — never repeat into a wedge"; say " 2. sysfs gpio515 (RESET_N): echo 1 > value; 1s; echo 0"; say " 3. tools/gpiopoke kmod: rail cycle (mode=cycle) then pwrkey (mode=pwrkey)"; say " 4. PHYSICAL: 20s power cut (proven; drops VBUS+all rails)"; say " Then verify-recovery.sh. DEFAULT-STOP: no ladder flailing."; }

say "rung 0: is the profile already active? (refreshFlag enables may need no refresh at all)"
Q=$(qccid) && [ "$Q" = "$EXPECT_ICCID" ] && { say "GREEN: $Q active — no refresh needed"; exit 0; }

say "rung 1: settle-wait 90s (post-op APDU-surface instability class — RM551 datum)"
T=$(at_wait_func_port 90) || { wedge_help; exit 1; }
Q=$(qccid) && [ "$Q" = "$EXPECT_ICCID" ] && { say "GREEN after settle: $Q"; exit 0; }

if [ -e /dev/cdc-wdm0 ]; then
  say "rung 2: QMI UIM slot power-cycle (no module reset; feed recipe)"
  uqmi -d /dev/cdc-wdm0 --uim-power-off --uim-slot=1 && sleep 3 && uqmi -d /dev/cdc-wdm0 --uim-power-on --uim-slot=1
  sleep 20; Q=$(qccid) && [ "$Q" = "$EXPECT_ICCID" ] && { say "GREEN after UIM cycle: $Q"; bump_budget; exit 0; }
else
  say "rung 2 unavailable (no cdc-wdm — ECM composition); documented: QMI composition unlocks it"
fi

N=$(budget_count)
if [ "$N" -ge 2 ] && [ "$FORCE" != "--force" ]; then say "POLICY: reset budget exhausted ($N/h) — refusing rung 3 (use --force with a fresh reason)"; exit 2; fi
say "rung 3: gentle CFUN 0 -> 1 (USB stays; settle + functional re-detect after)"
T=$(at_func_port) || { wedge_help; exit 1; }
at_read "$T" 'AT+CFUN=0' 5 >/dev/null; sleep 10; at_read "$T" 'AT+CFUN=1' 8 >/dev/null; bump_budget
T=$(at_wait_func_port 120) || { wedge_help; exit 1; }
Q=$(qccid) && [ "$Q" = "$EXPECT_ICCID" ] && { say "GREEN after gentle cycle: $Q"; exit 0; }
say "profile still not active after rung 3"; wedge_help; exit 1
