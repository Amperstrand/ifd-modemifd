# Architecture roadmap — 9eSIM provisioning path & PC/SC reader evolution

**Date**: 2026-10-25 · **Author**: architecture-research lane (owner-commissioned
independent assessment; external research: vsmartcard/upstream vpcd, wwand,
lpac, pySim, osmo-remsim, android-apdu-proxy) · **Status**: assessment complete;
near-term plan operationalized by the provisioning lane; roadmap pending owner
adoption.

Species-level record, same policy as `CSIM-SESSION-FINDINGS.md`.

## Decision summary

1. **Near-term (the upcoming profile operation): direct on-router lpac over
   MBIM UICC low-level access** (`usbnet=2`), per the provisioning lane's
   runbook (v3). No pcscd, no vpcd, no host-side processes on the one-shot
   critical path. Artifacts built, independently verified (two hashes), gate
   sequence and rollback tree encoded.
2. **PR #30 (vpcd over AT+CSIM): do not merge as-is.** Five findings filed on
   the PR thread; disposition options: re-scope to blank-card/generic-SIM, or
   swap the router transport to native QMI/MBIM and fix the listed defects.
3. **Medium-term (PC/SC-any-app access): two candidates, decision deferred
   until after the profile operation** — (C) vpcd host side + native-QMI/MBIM
   router agent; (F) adopt wwand (feed-packaged for this exact target,
   production-tested on the NR7101, eSIM-native) for modem+eSIM management.
4. **The CSIM session-state mechanism is a hypothesis, not a fact** (correction
   3 in the findings doc, fully closed across all three lane artifacts): no
   surviving instrument from any lane ever issued INS `C0` — the slot-1
   allowlist round's "C0 blocked" cell is also uninstrumented, so the T=0-
   absorption hypothesis loses its motivator entirely (`6D00` was the card
   correctly rejecting Reset Retry Counter). A corrected probe battery
   (`tests/hardware/at_csim_c0_battery.sh`, tracking issue #32) is the
   designated post-window experiment and will decide whether AT-class
   transports revive for enabled-profile cards — which materially changes
   Option B/C economics.

## Options considered

| Option | Verdict | Decisive evidence |
|---|---|---|
| A — PR #30 as-is (vpcd over AT+CSIM) | **Not viable near/mid-term for eUICC-with-profile**; code + protocol defects filed | transport rides AT+CSIM (session-state question open even after correction: channel-close AT-refusal + leak/`6A81` + lpac init failures stand); see PR #30 findings below |
| B — v4.1 SSH IFD hardening | Legacy/blank-card bench path only | same AT transport; relay fixes individually live-verified, integrated first-run pending (RELEASE-GATE.md) |
| C — vpcd + native QMI/MBIM agent | **Medium-term candidate** for PC/SC-any-app | vendor channel is QMI UIM (quectel_lpad is QMI-only); wwand verified QMI UIM OPEN_LOGICAL_CHANNEL/SEND_APDU on the exact modem; requires protocol fixes + timeout redesign + ownership model |
| D — lpac direct on-router (MBIM) | **Near-term winner — operationalized** | fewest moving parts; vendor channel; wwand MBIM-first priority + RM520N-GL HW proof; feed libqmi 1.36.0 compiles out UIM logical-channel APIs → MBIM variant won on buildability |
| E — hybrid (validated path now, architecture later) | **What was adopted** | D for the operation; PR #30 dispositioned on-thread; C/F decided post-window |
| F — adopt wwand | Medium-term alternative to C | feed packages exist for this target (mipsel_24kc); production-tested incl. NR7101; ES10c native + bundled lpac; no PC/SC server |

## Evidence index (external)

- **vsmartcard** (frankmorgner/vsmartcard, active; pin `93286a90` = upstream
  HEAD at research time): `vpcd.h` controls OFF/ON/RESET/ATR = `0x00/0x01/0x02/0x04`;
  **2-byte big-endian framing is upstream**; `vicc_poweron/poweroff/reset` are
  send-only — replies are read only after `0x04` and APDUs; empty `0x04` reply
  = card absent. The amperstrand patch changes two things: `SO_RCVTIMEO=2s`
  and eject-only-on-`r<0>`. The stock pcscd→vpcd chain has **no per-APDU
  timeout anywhere**.
- **wwand** (ddimension/wwand): QMI UIM APDU verified on the exact modem
  (NR7101/RG502Q-EA: logical channel + SEND_APDU + EF read, 2026-09-26,
  `docs/STATUS.md`); transport priority MBIM UICC → QMI UIM → AT; 30s APDU
  timeouts; eSIM E2E proven on RG650E/FM350-GL/RM520N-GL; RG650E quirk —
  ES10c *writes* over QMI refused (`undefinedError 127`) → write-path gating
  by a low-stakes write first is the right probe class.
- **lpac**: AT backend unbounded upstream (`poll(…, -1)`), docs verbatim
  "FOR DEMO PURPOSES ONLY… Maximum Response Time is typically 300ms, which is
  insufficient for many eUICC operations"; PR #149 documents modem-firmware
  AT timeouts aborting eUICC ops on Quectel EC25/EP06/RG520N; PRs
  #446/#442/#448 add bounded deadlines (240s CGLA for BPP; measured BPP wall
  clocks 26 s / 89 s). MBIM backend: 10s/command hard-coded (patched to 30s in
  the lane's artifact), 15s open, 20s close; `UIM_SLOT` 1-based, default 1;
  slot-mapping self-management (set `UIM_SLOT=2` or `SKIP_SLOT_MAPPING`);
  leak-safe channel close on disconnect.
- **pySim** `transport/modem_atcmd.py`: 0.2s per-command AT timeout — right
  class for classic SIMs, wrong for eUICC (corroborates timeout thesis).
- **osmo-remsim**: transferable patterns for any future reader agent —
  exclusive card access by default, idempotent slot-map re-mapping, bounded
  map-retry (10s), TPDU length validation before dispatch, bounded reconnect
  backoff ladder.
- **android-apdu-proxy**: synthesizes SW only for *unsupported* operations
  (MANAGE CHANNEL→`6D00`); terminates sessions on real errors — the error-
  semantics precedent vpcd agents should copy.

## PR #30 findings (filed on the thread, pull/30#issuecomment-6394910718)

1. Transport errors → synthetic `6F00` destroys the v3.4 `AtOutcome`
   executed/not-executed discipline; ambiguous failures should terminate the
   session (vpcd has no in-band error frame).
2. Hard-coded 1.5s APDU timeout (p50-sized) capped by the patch's 2s receive
   window — structurally incompatible with slow eUICC operations; the chain
   needs coordinated, operation-aware deadlines.
3. RESET answered with a canned ATR: (a) answering `0x02` at all is a
   one-frame desync generator under the actual pinned patched driver (mechanism
   matches ccid-firmware-rs's open "APDU response echoed the ATR" bench
   item); (b) control labels inverted vs upstream; (c) a modem cannot
   physically reset the seated SIM — fresh-state implication is false.
4. No per-tty `flock` in `open_serial` despite VPCD-ARCHITECTURE.md requiring
   it.
5. "Structural supersession by CSIM session-state findings" — **qualified by
   correction 3**: the session-state mechanism is now hypothesis-grade; if the
   corrected probe battery revives CSIM LPA sessions, this finding weakens
   (findings 1–4 stand regardless).

## Phased roadmap

1. **Complete** — findings stamped + corrected; PR #30 findings filed;
   artifacts built and dually verified; runbook v3 (see provisioning lane);
   external evidence index above.
2. **The window** (owner gates: runbook review → `usbnet=2` GO → write GO):
   runbook v3 as written — read-only gate, soak, first write = notification
   process (reversible, ES10-write class), one-shot download+enable,
   no-retry discipline, nadanada displaced only at the enable step.
3. **Post-op**: registration verify, reconciliation, composition
   restore-or-adopt decision (owner).
4. **PR #30 disposition**: re-scope or transport-swap per the thread.
5. **Medium term**: decide C vs F (driver: is PC/SC-any-app a real
   requirement?); if the driver keeps shipping, add the session-state
   preflight — **must use genuine INS `C0`** (findings §8.3 + correction 3).
6. **Long term**: native transports as baseline; retire the SSH path when the
   successor lands; `QSIMCFG="APDU_decrypt"` semantics only via Quectel
   support ticket (no public documentation exists; RM520N R01-baseline-only,
   dropped in R03 — never blind-toggle, wedge history).

## Known unknowns (explicit)

| Unknown | Why it matters | Resolution path |
|---|---|---|
| Whether AT+CSIM can hold an ISD-R session with an enabled profile (correct `C0` + allocated-channel CLA never probed) | Decides if AT-class transports revive; changes Option B/C economics and the session-state preflight design | Post-window corrected probe battery (see tracking issue); read-only, under lane lock |
| ECP3 final-install single-APDU latency | Timeout budget for the one-shot; unmeasurable without a consumable activation code | Bounded by the 30s patched artifact; residual accepted and recorded |
| MBIM UICC on RG502Q-EA specifically | The MBIM pivot's exact-modem evidence is same-family (RM520N-GL) + similar-platform (RM502Q-GL), not same-modem | Window gate step 2 (read-only `chip info`) detects immediately; abort → restore composition |
| OmniKey external-reader fallback | Staged but never exercised end-to-end (dry-run reached the reader with the correct no-card error) | Last-resort only; a rehearsal with a sacrificial card is cheap insurance if the fallback ever becomes primary |
| `APDU_decrypt` semantics | Only unmeasured AT knob; could in principle open raw APDU access | Quectel ticket; excluded from any window |
| pcscd-side ~2-min post-restart transmit dead window (ccid-firmware-rs TRANSMIT_DEAD_WINDOW.md) | Blocks any vpcd-based reader from fast recovery after pcscd restart | Open upstream-of-patch issue; pcscd-host-stack lane owns it |
