# CSIM Session-State Findings & On-Device Provisioning Plan (NR7101 / RG502Q-EA)

**Date**: 2026-10-24/25 night session · **Authors**: spare-eSIM provisioning lane +
modem-hil lane (cross-reviewed) · **Status**: findings stamped, plan pending external
review + owner GO.

Species-level record. No unit identifiers, ICCIDs, EIDs, IMEIs, or bench
addresses (repo public-repo policy). All measurements taken on one NR7101
(MT7621 host, OpenWrt 25.12.6) with an RG502Q-EA modem (AACR track firmware,
identical string to the unit that produced the v4.0 hardware evidence) and a
removable ECP3-class eUICC carrying one enabled operational profile in slot 2
and one inactive test USIM in slot 1.

## 1. Question under test

Can a host- or device-side LPA (lpac) hold a coherent ISD-R session with the
seated eUICC through the modem's `AT+CSIM` transport, on a unit whose eUICC
has an **enabled, registered profile**? (The v4.0 evidence — full SGP.22
download+enable+notification through this driver family — was produced on a
unit whose card was **blank** at download time. See §5.)

## 2. Measured matrix (all read-only against the card; stamped logs on the
bench router, copies archived lane-side)

| Exchange | Result | Notes |
|---|---|---|
| Basic-channel SELECT by FID (`00 A4 00 00 02 3F 00`) | `9000` ×3 | deterministic across windows |
| Basic-channel SELECT EF by FID (P2=0C) + READ BINARY 10 B | `9000` then real EF data | the card's ICCID EF was read back byte-exact — genuine card access at filesystem level |
| MANAGE CHANNEL open (basic, `00 70 00 00 01`) | allocates `02…`/`03…`, `9000` | channels allocate from 2 upward while a profile is enabled |
| MANAGE CHANNEL close (any form: basic-close `00 70 80 0n`, channel-form `0n 70 80 0n`) | AT-level `ERROR`, always | opens leak until a CFUN recycle; exhaustion (`6A81` on open) after ~3–4 opens |
| ISD-R SELECT by AID, ISO channel encoding (`0n A4 04 0C 10 <ISD-R AID>`) | `61 21` deterministic | bytes nominally pending; see §3 for why this SW is likely NOT card-generated |
| GET RESPONSE (`0n 2C 00 00 21`) | `6D00` at 1.5 s and 8 s gaps, both ch1- and ch2-class CLAs | never succeeds in any tried form |
| ISD-R SELECT inline-FCI form (`P2=00, Le=00`) | `6A82` | on an opened channel |
| ISD-R SELECT after a *refused* open (post-exhaustion) | still `61 21` | the SELECT "answer" is decoupled from channel accounting — modem-intercepted context |
| lpac (at_csim build `2.3.0.r466.a43e46ae`) `chip info` / `profile list` / `notification list` | `euicc_init` failure ×3 | consistent with the GET RESPONSE result — init cannot complete |
| `AT+CFUN=0` then CSIM | `+CME ERROR: 13` on CPIN, `+CME ERROR: 0` on CSIM | matches the hardware design doc: CFUN=0 makes the SIM *invalid* |
| `AT+QUIMSLOT=2` state | already active | QUIMSLOT selects the baseband's active interface; it is not a host-owned card bus |

## 3. Mechanism (final, with correction log)

**While the eUICC has an enabled profile, the modem's own USIM stack holds the
card session and `AT+CSIM` never yields an independent, coherent logical
channel to an LPA.** SELECT-class exchanges "answer" (modem-managed contexts),
but INS `2C` (GET RESPONSE) never works and inline-FCI SELECTs fail, so no
multi-APDU ES10 session can complete.

Corrections issued during the session (kept for the record because reviewers
may meet the same traps):
1. *"The inactive slot-1 USIM holds channel 1"* — RETRACTED. Channels are
   per-card; a second physical card cannot hold a channel on the eUICC. The
   session holder is the baseband's own USIM session **with the enabled
   profile on the eUICC itself**.
2. *"CLA `0x4X` is allowlist-blocked"* — RETRACTED. `0x41`/`0x42` are the ISO
   7816-4 encodings of **channels 5/6** (the `b7=1` form covers channels
   4–19). Probes using them addressed channels that were never opened; the
   refusals were no-such-channel, not filtering.
3. *"Pacing explains the GET RESPONSE failures"* — RETRACTED. `6D00` at both
   1.5 s and 8 s inter-command gaps.

## 4. Slot architecture (researched against the RG50xQ hardware design doc)

The RG50xQ provides **two separate physical (U)SIM interfaces** — USIM1 and
USIM2 each with dedicated VDD/DATA/CLK/RST/DET pins — not a shared-wire
hardware switch. But the module is **dual-SIM-single-standby**: one baseband
USIM stack activates one interface at a time (`AT+QUIMSLOT`, persisted in NVM,
applies after reset). Consequences:
- There is no state in which the host "owns" one card while the baseband owns
  the other. The inactive slot's card is simply not in session with anything
  (the modem can still read its ATR/ICCID for detection purposes, which is why
  `AT+QSIMCFG="dual_slot_status"` reports both slots).
- `AT+CSIM` is bound to the active slot's card; there is no per-slot APDU
  routing (TS 27.007 defines no slot parameter for CSIM, and the Quectel AT
  set adds none).
- Host-side stacks (pcscd etc.) never touch the cards directly; every card
  byte crosses the baseband.

## 5. Why the v4.0 success differs

The unit that produced the v4.0 evidence downloaded its profile onto a
**blank** eUICC (no enabled profile → no baseband USIM session with the card →
channel accounting and APDU routing behaved like a clean pass-through; the
driver's own evidence table records FCI being answered during logical-channel
lifecycles, which requires working response retrieval). Card **session state**
— not firmware strings, not the second SIM, not pacing — is the differentiator
between that success and the present failures.

## 6. Plan (pending external review + owner GO)

**Phase 0 — hygiene:** CFUN recycle (clears leaked channels; verified class).
**Phase A — minimal external-reader write (the unlock):** move the eUICC to a
physical reader on the bench host; execute exactly one write: *disable the
enabled operational profile* (reversible; re-enable documented). Reseat. This
recreates the blank-card session state that Phase B requires.
**Phase B — on-device everything else (the appliance goal):** CFUN re-init →
on-device lpac gate (`chip info` must return an EID — this is the theory's
decisive check) → single-attempt profile download from the SM-DP+ over the
router's non-SIM uplink → enable ladder (ICCID → AID → refreshFlag) →
notifications → modem re-read + registration verify.
**Rollback tree:** any Phase-B failure post-download → re-enable the old
profile (the Phase-A write in reverse, on-device if the gate passed, else
reader); download-failure pre-landing → stop, no retry without diagnosis
(one-shot activation-code discipline — a cancelled download can close the
SM-DP+ window); catastrophic card error → freeze, escalate, no further writes.
**Fallback if the Phase-B gate fails:** complete provisioning on the external
reader (proven class) and reseat; the on-device recipe then needs a different
enabler (see §8).

## 7. Standard annotations for the audit pass

- ISO/IEC 7816-4 logical-channel CLA encoding: channels 1–3 in CLA bits 1–0
  (`0x01`–`0x03`); channels 4–19 use the `b7=1` form where **bits 4–1** carry
  (channel−4), i.e. CLA = `0x40+(n−4)`, range `0x40`–`0x4F` (SM moves to b6,
  chaining to b5 in that form). Our initial probes violated this (§3.2) —
  probes must encode the *allocated* channel.
- 3GPP TS 27.007 `+CSIM=<length>,<command>`: `<length>` is the command length
  in **characters**; a mismatch yields an unconditional `ERROR`
  (live-verified) — indistinguishable from a policy refusal without a
  length-audit. All probe lines are mechanically length-audited.
- GSMA SGP.22: ISD-R AID is the published GSMA constant; profile download is
  additive (never overwrites); enabling a profile disables the previously
  enabled one on non-MEP eUICCs (reversible); pending notifications should be
  processed promptly; a cancelled download can invalidate the session window
  (one-shot discipline).
- TLS: SM-DP+ endpoints present GSMA RSP PKI certificates (root: *GSM
  Association – RSP2 Root CI1*, SKID `81:37:0F:51…EB:FB`), absent from
  generic OS trust stores — provision a fingerprint-verified CA bundle rather
  than disabling verification. Bundle note: SGP.22 v3 chains can also root at
  *OISTE GSMA CI G1* — include all live consumer roots, not CI1 alone.

## 8. Open questions for external review

1. Is there a baseband state short of "no enabled profile" that yields a
   clean CSIM pass-through? (CFUN=4 airplane mode keeps the SIM *valid* per
   the hardware doc but is untested for session behavior; expected: session
   persists. Cheap test, low value.)
2. Does any Quectel AT command expose per-slot or raw card access on this
   firmware family beyond CSIM/CCHO/CGLA/QESIM? (QESIM measured as a facade;
   `QSIMCFG="APDU_decrypt"` capability exists, semantics unmeasured — Q-knob
   wedge history counsels caution.)
3. Should the driver (ifd-modemifd) grow a *session-state preflight* that
   detects the enabled-profile condition (open→SELECT→GET RESPONSE probe) and
   fails fast with a dedicated error, so future users don't re-derive §3?
4. Should the driver (ifd-modemifd) grow a *session-state preflight* that
   detects the enabled-profile condition (open→SELECT→GET RESPONSE probe) and
   fails fast with a dedicated error, so future users don't re-derive §3?
5. Two standards-compliant alternative mechanisms for the §2 matrix that §3's
   "modem-intercepted contexts" reading should be tested against before being
   treated as final: **(a) MT CLA-rewrite** — the modem forwards SELECTs with
   a rewritten CLA (basic or its own channel), so the card's answers are
   genuine but belong to a channel the host's GET RESPONSE never addresses;
   **(b) T=0 procedure absorption** — if the MT↔card link runs T=0, `61xx` is
   the normal case-4 SELECT procedure byte and the T=0 layer owns the GET
   RESPONSE exchange; a host-issued INS `2C` arrives with no procedure context
   and is answered `6D00` by the MT without reaching the card. Measured card
   ATR (read via `AT+QSIMCFG="atr"`): T=0 offered first, T=1 also offered
   (TD2 protocol 1 with TA3/TB3 present, TCK present) — both link protocols
   are available to the MT, so the T=0 hypothesis is live. Both alternatives
   keep the §6 plan intact but imply a patched host LPA cannot help.
6. Would a patched lpac at_csim (correct ISO channel encoding + explicit
   response-retrieval) behave differently, or is the block strictly
   modem-side? (§2 suggests modem-side; a wire-level lpac debug was not
   obtainable with this build.) Superseded by §10: the AT transports are
   closed on this firmware generation regardless; see the QMI/MBIM path.

## 9. Evidence index (species-level)

Bench router `/tmp` (volatile; archived copies lane-side):
`slot2-probe-*.log` (probe rounds 1–3), `slot2-round4-*.log` (B0/ICCID read +
channel facts), `slot2-cfun-*.log` (recycles), plus the lane-side window logs
for the lpac, encoding-discrimination, CFUN=0, and inline-FCI tests.
Lane-side durable archive: probe script (length-audited), runbook, GSMA root
bundle (fingerprint-verified), and all of the above logs.

## 10. Post-research addendum (2026-10-25, drives the plan revision)

Follow-up research (vendor application notes, Quectel forum field reports,
lpac upstream sources) established:

1. **`AT+QESIM` operations are vestigial on this firmware generation,
   family-wide**: multiple independent field reports (RM520N/RM510Q class)
   show `lpa_enable,1` succeeding while every other op returns `ERROR`;
   Quectel support's official answer is that profile operations require
   their separate tool. The ops also require `lpa_enable=1`, which our bench
   has a documented do-not-toggle wedge incident for. Even if the ops
   worked, this build does not advertise `add_profile` — downloads would
   still need an LPA.
2. **The vendor's own eSIM channel is QMI UIM**: the gated `quectel_lpad`
   tool drives profile operations exclusively through QMI UIM messages
   (its own debug logs), and only in usbnet (non-ECM) composition.
   Third-party product eSIM stacks for these modems converge on the same
   class of low-level SIM access (QMI/MBIM).
3. **lpac upstream carries `qmi` (cdc-wdm) and `mbim` (UICC low-level)
   APDU backends in-tree** — self-contained C, buildable for the router
   target by the same cross-build pipeline that produced the at_csim build.
4. **ECM composition (this unit's current state) exposes no `/dev/cdc-wdm*`**;
   QMI or MBIM composition does. The unit previously ran QMI composition as
   its factory/reference configuration.

**Revised plan (supersedes §6 Phase A)**: single deliberate composition
switch to QMI (`AT+QCFG="usbnet",0` + cold modem reset — the safe single-
switch pattern; the documented USB wedge class involved *rapid stacked*
switches), then on-router `lpac` with `LPAC_APDU=qmi` against `/dev/cdc-wdm0`:
`chip info` gate → disable the enabled profile (the §6 Phase-A write, now
on-device) → download + enable the new profile → verify → restore ECM (or
adopt QMI as the baseline, which is the factory configuration). External
reader remains the last-resort fallback only.
