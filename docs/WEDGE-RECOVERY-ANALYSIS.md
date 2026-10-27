# WEDGE RECOVERY ANALYSIS — reboots, watchdogs, and what actually fixes what

**Question (owner, 2026-10-27):** will reboots fix the wedge (with a
watchdog), or does it require manual power reset?
**Short answer:** host reboots — **no**, proven for the deep class. Manual
power reset — **yes on stock hardware**, but the cut is automatable at the
*power* layer (PoE control), which is the recommended workaround. A
watchdog + automated PoE cycle = full autonomous recovery.

## The evidence chain (all incident-tested, no speculation)

| Intervention | Class it clears | Evidence |
|---|---|---|
| Host reboot (incl. watchdog-triggered) | **nothing above mild** — W1 survived it | MODEM-REFRESH-POLICY §1: unresponsive to host reboot |
| Router reboot pulses RESET_N (modem-guard shutdown hook) | clean-boot hygiene only — W1 survived RESET_N 1/3/30 s | modem-guard README; policy §1 |
| HC unbind/rebind / USBDEVFS_RESET | host-side shapes, partial | policy "pre-physical rungs" (2026-10-26) |
| QMI UIM slot power-cycle / gentle CFUN 0→1 | mild classes (settle, UIM) | ladder rungs 2–3; matrix |
| gpiopoke 30 s `lte-power` rail cut | **NOT W1** (30 s > 20 s cut, different rails) | gpiopoke README incident § |
| PWRKEY 12 s force-off | **NOT W1** | gpiopoke README |
| **20 s physical cut (drops VBUS + every rail)** | **all three wedge classes (3/3)** | policy §1–3 |

Why reboots can't work: the latch lives in a **VBUS-fed (or standby-fed)
power domain** on the modem side — a domain the host cannot drop in software
(DTS truth: no VBUS switch, no hub, hardwired USB power) and which SoC-level
resets (reboot, xHCI re-init, RESET_N) never de-power. Duration is not the
differentiator (the failed rail cut was 30 s vs the working 20 s cut);
**which rails drop** is.

## What modem-guard already gets right — and the one rung to add

The deployed ladder (`tools/modem/modem-guard/`) correctly escalates
settle → UIM cycle → gentle CFUN → gpiopoke rail cycle and then **STOP:
human action required (20 s power cut)** with budget + exponential backoff.
That STOP is honest — on unmodified hardware nothing software-side lies
above it.

**Recommended extension (the workaround):** automate the *power* layer, not
the reboot layer. **Update 2026-10-27: this automation already exists on the
bench** — the NR7101's PoE feed sits behind a managed Zyxel GS1900 with
per-port control (`stock_ctl cycle 7`, port p7 per the registry — the
documented RUNBOOK path), and a labgrid place `nr7101-router` exposes the
same power control labgrid-natively (source: wedge-cont lane handoff,
2026-10-27). Known caveat: the switch has been intermittently unreachable
from the bench host — verify the control path BEFORE depending on it
mid-series (probe it as a pre-flight, not during an incident). Remaining
work is therefore only the hook wiring:

    exhaustion rung → POE_CMD (port off 20 s → on) → boot health check → probe

Concretely: add an optional `POE_CMD` hook to modem-guard's exhaustion rung
(executed once, budget-guarded, STOP state preserved as fallback when no
power control is deployed). Result: a complete autonomous recovery chain —
probe fail → rungs 1–4 → PoE cut → verify — with the human rung retained
for "PoE control absent/failed". No hardware modification to the router;
bench-side gear only.

Hardware-mod alternative (documented, NOT recommended until needed): splice
a VBUS-switching hub/circuit internally — converts the cut into an on-device
software rung. Invasive (weatherproof enclosure, warranty), and unnecessary
while PoE control achieves the same effect upstream.

## New discriminator for the harvest protocol (add to #53 step 4)

On any future off-bus event, FIRST check for EDL before calling it a wedge:

    grep -l . /sys/bus/usb/devices/*/idVendor | while read f; do
      d=$(dirname $f); [ "$(cat $d/idVendor):$(cat $d/idProduct)" = "05c6:9008" ] && echo EDL at $d
    done

`05c6:9008` = Qualcomm EDL/firehose mode — a deep crash-to-bootloader that
is **software-recoverable via QFirehose** (full-image reflash), NOT the
wedge class. Distinguishing the two on capture changes the entire recovery
path and must precede any "wedge" claim in the record.

## Bottom line

- Watchdog **reboots alone**: fix only the mild classes — already covered
  by modem-guard rungs 1–3. For the deep class they are proven useless.
- **Manual power reset**: currently required, and correctly encoded as the
  STOP rung.
- **Recommended**: watchdog + automated PoE port cycle (upstream power
  control) = the wedge becomes fully self-healing with zero router
  modification. One `POE_CMD` hook away.
