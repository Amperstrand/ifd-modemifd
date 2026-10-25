# Modem refresh policy & wedge prevention (NR7101 / RG502Q-EA)

**Status:** operational law, adopted 2026-10-25 after two wedge incidents
in one session. **Executable form:** `tools/modem/refresh.sh` (the ladder,
with a reset budget) over `tools/modem/atlib.sh` (port-safe AT I/O).
Species-level record.

## What happened (the evidence this policy is made of)

1. **Wedge 1 (19:11Z):** `AT+CFUN=1,1` fired one second after a successful
   eUICC profile enable. The modem's USB controller failed re-init: device
   flapped once, then left the bus entirely — zero USB attach events in any
   mode, unresponsive to host reboot, RESET_N pulses (1/3/30 s), a
   register-verified 30 s power-rail cut (`tools/gpiopoke`), and the PWRKEY
   force sequence. **Recovered only by a 20-second physical power cut**
   (drops VBUS with every rail — strictly stronger than the load-switch
   rail cycle).
2. **Wedge 2 (19:55Z):** during the measurement re-attempt, the *gentle*
   `CFUN 0→1` variant wedged the modem into **silent-but-on-bus** (ports
   enumerate, nothing answers). Same recovery applies. Both reset signatures
   observed on one firmware in one day ⇒ the differentiator is not the
   reset method — it is **reset debt after profile operations**.
3. Field corroboration: Quectel BG96 `CFUN=1,1` permanent-disconnect
   transcript (netdev 2022-12-12); Dell eSIM fleet — reset-heavy automation
   bricking modems silent (pitfalls doc); RM551-class post-profile-operation
   APDU instability until settle (Quectel forum); Quectel RG50xQ AT manual —
   hardware reset recommended when CFUN resets misbehave.

## The completed matrix (2026-10-25, firmware RG502QEAACR13A02M4G_ZYXEL)

All three profile-switch mechanisms were live-tested, one wedge each — the
differentiator is **not the refresh method**; it is this firmware's USIM
re-initialization after ANY profile change:

| Mechanism | Result |
|---|---|
| `CFUN=1,1` after enable | wedge #1 — modem left USB entirely; recovered only by 20 s physical cut |
| gentle `CFUN 0→1` | wedge #2 — silent-but-on-bus; recovered by 20 s physical cut |
| card-driven REFRESH (`lpac profile enable <iccid> 1`, zero host resets) | wedge #3 — silent-but-on-bus (enable itself returned `code:0`; evidence `0a681623…`) |

Consequences: profile *switching* on this firmware generation is
unreliable by every software path (the wedge is card-safe and
power-cut-recoverable — a viable absorb-it runbook posture exists when a
switch is unavoidable); reliable switching requires the firmware track
(field-proven 13A04 path — separate owner research). Daily data stability
between switches is unaffected. The ladder below still governs: rung 0
remains correct (it verified the profile first and asked nothing of the
modem); rungs exist for the no-switch refresh cases.

## The ladder (post profile enable/disable — run in this order)

| Rung | Action | Notes |
|---|---|---|
| 0 | **Check first** | If the enable carried `refreshFlag=1` (`lpac profile enable <ICCID> 1`), the **card** issues ETSI TS 102 223 REFRESH and drives the modem itself — often no host action is needed at all. Verify by QCCID before touching anything. |
| 1 | **Settle-wait 90 s** | Post-op APDU-surface instability is real; silence in the first minute is not evidence. |
| 2 | **QMI UIM slot power-cycle** | `uqmi --uim-power-off/--uim-slot=1` → on (the OpenWrt lpac-documented recipe). No module reset, no USB. Needs `/dev/cdc-wdm0` (QMI composition). |
| 3 | **Gentle `CFUN 0→1`** | USIM re-read on the functionality transition; USB stack stays up. Settle + **functional** port re-detect after (the AT port can shuffle). |
| 4 | `CFUN=1,1` | **Forbidden after profile operations.** Executable only via explicit force with a fresh reason — wedge 1's exact cause. |

**Reset budget:** at most **2 reset-class operations per hour** (rungs 2–4;
budget file clears on power cuts, which also clear the debt). Enforcement
is in `refresh.sh`.

## Detection rules (how to not fool yourself)

- **Functional port probe:** several ports echo `OK` to bare `AT`; only the
  AT-command port answers `AT+CPIN?` (`CPIN:` **or** `+CME` — a CME error
  proves the parser; during USIM re-init `CPIN?` returns `+CME ERROR: 13`).
  `atlib.sh:at_func_port` implements this.
- **Never hardcode the port.** The AT function shuffled twice in one day
  (ttyUSB2→3→2).
- **Never issue blocking reads** without a background-reader + kill pattern
  (`atlib.sh:at_read`); `timeout` does not exist on stock OpenWrt and
  `head -c N` blocks forever on a silent port.
- **Wedge signatures:** (a) ports gone from `/dev/ttyUSB*` + zero dmesg
  attach events; (b) ports present, zero responses on any port. On either:
  **DEFAULT-STOP** — no further resets, no ladder flailing.

## Recovery rungs (in order, all proven 2026-10-25)

1. `sysfs gpio515` (RESET_N; not DTS-hogged): pulse 1 s.
2. `tools/gpiopoke` kernel module: `mode=cycle` (30 s load-switch rail cut),
   then `mode=pwrkey` (12 s force-off + boot press). Register map and
   usage in its README; dump-first, self-unloading.
3. **Physical power cut, 20 seconds** — the only rung that drops VBUS with
   every rail; fixed both wedges. The owner's hand; nothing to be embarrassed
   about — it is the designed last rung, not a failure of skill.

## Process rules (paid for in artifacts)

- **Pull artifacts off the router *before* requesting a power cut** —
  `/tmp` is tmpfs and dies with the cut (the seq-2 notification evidence
  was lost exactly this way).
- Archive with sha256 immediately after every window; the evidence chain is
  the product.
- One lane lock per window; release at every exit path; DEFAULT-STOP is a
  clean exit, not an error.
- Measurement/retry cycles after any profile operation start at rung 0 of
  the ladder — never at a reset.
