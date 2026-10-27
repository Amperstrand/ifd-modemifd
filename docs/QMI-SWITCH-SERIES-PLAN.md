# QMI SWITCH-RELIABILITY SERIES — pre-registered plan (v1, PLAN ONLY — not executed)

**Status:** operator-directed plan ("plan it"), 2026-10-27. Execution is a
separate owner GO. Species-level record.
**Hypothesis under test (H_switch):** profile switching executed via lpac's
**native-QMI backend** on **fresh-boot** modems is wedge-free — i.e. the
wedge class is an AT/CSIM-path + reset-debt phenomenon, not an unavoidable
property of profile switching. If true, 13A04 drops from critical path to
defense-in-depth for the switching use-case.

**Motivating counterexample (n=1):** 2026-10-26 ~18:44Z — silent.link enable
via p5-ladder, native-QMI backend, rc=0, fresh boot, **zero wedge**, healthy
through a later ladder reset and the 19:21:51Z baseline (dossier §1.3 C3;
conwrt #577 E2).

## Design

| Parameter | Value |
|---|---|
| N | **5 cycles**, each fresh-boot-gated |
| Toggle pair | **test/travel profile ↔ nadanada** (both already on-card; reversible enable/disable only — **no downloads, no credential spend, silent.link never touched**) |
| Mechanism under test | lpac native-QMI backend enable (exact invocation: reuse `/root/qmi570/p5-ladder.sh` from qmi-research) + **natural reboot landing** (router reboot — the canonical C1 mechanism) |
| Forbidden | CFUN in any form; host-side reset rungs; any AT/CSIM eUICC op mid-cycle; usbnet composition change |
| Fresh-boot gate | modem up ≤10 min AND zero eUICC ops this boot before the enable |
| Settle | ≥90 s after any operation before judging (ladder rung 1) |

## Per-cycle procedure (strict order)

1. Lane lock (`modem-lane.sh take`) + heartbeat; card-safety declaration.
2. Fresh-boot verification (uptime, no prior ops this boot — logged).
3. State capture: functional-port detect (atlib `at_func_port`), QCCID,
   CEREG, CSQ — read-only.
4. `lpac profile enable <target-ICCID>` via native-QMI backend; record rc +
   full JSON.
5. Settle 90 s. **No CFUN. No host reset.**
6. Router reboot (natural landing). Wait for full bring-up (~3 min).
7. Post-boot verify: **profile correctness + gate.** (a) QCCID = target
   ICCID and CEREG registered (the gate does NOT check which profile is
   active — this stays manual). (b) **Call `/root/reliability/verify-
   gate.sh`** (device-reliability lane; standalone POSIX sh, READ-ONLY,
   kill-guarded, takes no lock — safe mid-series; SSOT: conwrt-bench
   `tools/reliability/verify-gate.sh`). Cycle passes iff exit 0 (GATE:GREEN)
   AND tokens `data=PASS modem=PASS` in `/root/reliability/state/last-checks`
   (grep-able #575 assertions). `wg=WARN` acceptable — the CGNAT per-session
   UDP-filtering class is known and out of scope here. Manual `ip route get`
   check retained as fallback if the gate is absent.
8. Archive: cycle log (JSON line), lpac outputs, QCCID/CEREG captures,
   sha256 — to `docs/evidence/qmi-switch-series/` (staged).
9. Reset-debt ledger updated (ops this boot = 0 is the invariant; the reboot
   clears debt by design).

## Wedge handling (pre-registered, non-negotiable)

On **either** wedge signature (ports gone + zero attach events, OR
silent-but-on-bus): **DEFAULT-STOP** —
1. harvest per #53 step 4 BEFORE any recovery: soft snapshot (baseline-soft
   command set), gpiopoke register dump, full `/sys/kernel/debug/usb/devices`,
   dmesg ≥40 lines, USB-stick discriminator;
2. artifacts off-router + sha256 BEFORE any power cut (tmpfs rule);
3. physical cut is the owner's hand; series TERMINATES on first wedge —
   one specimen is the finding, not an invitation.

## Pass/fail (pre-registered)

| Outcome | Verdict | Consequence |
|---|---|---|
| 5/5 clean (enable rc=0, correct QCCID, registered, data OK) | **H_switch supported** | QMI-backend enable + natural reboot = canonical switching mechanism; 13A04 → optional (defense-in-depth); dossier §2 verdict cell updated |
| ≥1 wedge | **H_switch rejected for ops** | firmware track stays critical; harvested specimen feeds the #53 register diff; 13A04 ticket urgency restored |
| Any policy violation (accidental CFUN, wrong profile touched) | cycle void | do not count; re-run cycle after fresh boot |

n=5 rationale: the wedge class showed ~2/3 per-attempt incidence under
reset debt; 5 clean fresh-boot cycles at that base rate would be ~1%
likely if the mechanism were equally fragile — strong evidence for the
transport+debt split. Not proof of zero-risk; it licenses posture change,
not certainty.

## Dependencies & sequencing

1. **gpiopoke register baseline MUST be captured first** (#53 step 3) — a
   series-wedge without the baseline wastes the specimen. **CAPTURED
   2026-10-27T15:19Z** (`docs/evidence/wedge/register-baseline-20261027.txt`)
   — this prerequisite is satisfied.
2. **Start gate (agreed with device-reliability, 2026-10-27):** series
   starts ONLY after (a) lpac-hwval's device-release ping, then (b)
   device-reliability's cycle-3 (final) reboot + gate-green evidence
   (~15 min post-boot). That boot doubles as this series' fresh-boot
   precondition for its first enable (tail-only boot pairing; their cycles
   1–2 stay standalone). Router state during the window: their ladder is
   guarded/disabled until release; the verify-gate itself is always safe
   (read-only, no lock).
3. Bench session window (~2–3 h: 5 cycles × ~20 min + margins).
4. Operator GO per first cycle (first cycle doubles as dress rehearsal).

## Explicitly out of scope

Firmware flash (research only per operator), MBIM (parked — QMI faster +
working), any silent.link operation, any new credential consumption,
induced wedges.
