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
| ~~GET RESPONSE (`0n 2C 00 00 21`)~~ | `6D00` at 1.5 s and 8 s gaps, both ch1- and ch2-class CLAs | **VOID (correction 3)** — INS `2C` is RESET RETRY COUNTER, not GET RESPONSE (`C0`); `6D00` is the card correctly rejecting a mis-encoded command. No genuine `C0` was ever issued by any lane (verified across all surviving artifacts) |
| ISD-R SELECT by AID, ISO channel encoding (`0n A4 04 0C 10 <ISD-R AID>`) | `61 21` deterministic | bytes nominally pending — **but collected with CLA=01 while allocation returned 02/03 (provenance flaw, §3)**; the responder identity is unresolved |
| ISD-R SELECT inline-FCI form (`P2=00, Le=00`) | `6A82` | on an opened channel |
| ISD-R SELECT after a *refused* open (post-exhaustion) | still `61 21` | the SELECT "answer" is decoupled from channel accounting — modem-intercepted context |
| lpac (at_csim build `2.3.0.r466.a43e46ae`) `chip info` / `profile list` / `notification list` | `euicc_init` failure ×3 | **LEADING HYPOTHESIS (correction 4): client-side lowercase-hex rejection** — lpac emits lowercase AT+CSIM PDUs; Qualcomm-class modems are documented rejecting lowercase with fast bare `ERROR` (pySim gerrit 8e15c18 transcript; lpac #428, one-line uppercase fix, unmerged); our own UPPERCASE driver is this same modem's proven success |
| `AT+CFUN=0` then CSIM | `+CME ERROR: 13` on CPIN, `+CME ERROR: 0` on CSIM | matches the hardware design doc: CFUN=0 makes the SIM *invalid* |
| `AT+QUIMSLOT=2` state | already active | QUIMSLOT selects the baseband's active interface; it is not a host-owned card bus |

## 3. Mechanism (DOWNGRADED TO HYPOTHESIS — third correction, 2026-10-25; see correction log entry 3)

**Correction entry 3 (independent reviewer + lane audit; verified against
artifacts): every "GET RESPONSE" probe in this investigation — ours AND the
round-4 variant — issued INS `2C` (RESET RETRY COUNTER), not INS `C0` (GET
RESPONSE). A card answering `6D00` to `2C` with invalid P1/P2 is CORRECT
behavior, not transport blockage. "GET RESPONSE never works" is UNPROVEN:
zero genuine `C0` probes exist in any surviving artifact from any lane
(verified: probe scripts + logs, router-side and lane-side archives — the
slot-1 allowlist round's instruments survive; none contain `C0`, so its
recorded "C0 blocked" claim also lacks instrumented basis). Additionally
the archived `61 21` SELECT response was collected with CLA=01 while the
allocation returned 02/03 (CLA-provenance flaw). The T=0-absorption
hypothesis (§8.5) loses its main motivator — the `6D00` pattern is now
fully explained as correct rejection of a mis-encoded command.**

**Correction entry 4 (fork-research lane w19:pK, reviewer-endorsed): the
`euicc_init` ×3 failures now have a leading CLIENT-SIDE hypothesis — lpac's
lowercase-hex AT+CSIM emission.** Evidence: pySim gerrit `8e15c18` (lowercase
PDU → bare `ERROR`, same PDU uppercased → `61 2F`/`OK`, Sierra EM7565 — fix
shipped in pySim years ago); lpac issue #428 (Foxconn T99W696, same
`euicc_init` signature, one-line `hexutil.c` digits→uppercase fix, reporter
told "Qualcomm based WWAN modules may only accept AT+CSIM commands with
uppercase" — no fix merged upstream or in any fork as of 2026-10-25); our
own driver's uppercase table and full SGP.22 success on this modem. The
fast-fail (not hang) shape of the bench failures matches the pySim
transcript. Consequences: (a) the enabled-profile session-state mechanism
drops to secondary; (b) §10's "the AT transports are closed on this
firmware generation" is no longer established; (c) the decisive experiment
is the amended R1 discrimination build (pin + uppercase hexutil, + #446 to
bound the stock infinite `poll(-1)`) plus the C0 battery's hex-case control
cell — both pre-registered before any hardware runs.**

What the evidence now supports (INS-independent survivors only):
- MANAGE CHANNEL opens allocate (02/03) — real; closes are AT-refused — real
  (open-leak → `6A81` exhaustion, all rounds).
- Basic-channel SELECT-by-FID (`9000` ×3) and READ BINARY (real EF data
  read back byte-exact) — real, the strongest surviving card-access datum.
- Inline-FCI SELECT (P2=00/Le=00) → `6A82` — real, correctly-encoded INS.
- On-device lpac (at_csim) `euicc_init` failure ×3 — real, and INS-independent.
  **Correction 4:** "correctly encoded" is true syntactically but false
  on-the-wire CASE — lpac emits lowercase hex in `AT+CSIM` PDUs
  (`euicc/hexutil.c` digits table, unchanged upstream and in every fork we
  inspected), and Qualcomm-class modems are documented rejecting lowercase
  PDUs with a fast bare `ERROR` (pySim gerrit 8e15c18, Sierra EM7565, exact
  transcript; lpac #428, Foxconn T99W696, identical `euicc_init` signature,
  one-line uppercase fix — unmerged upstream). Our v4.0-proven driver emits
  UPPERCASE (`src/at.rs`). The WHAT stands; the WHY now has a leading
  hypothesis — **client-side case rejection**, demoting the enabled-profile
  session-state mechanism to secondary. Pre-registered prediction: the
  failure is fast (`ERROR`), not a hang — a stock-vs-uppercase discrimination
  build decides it at zero card risk (see FORK-RESEARCH §3/§5).
- Hypothesis (unproven, post-window probe class): the enabled-profile
  session-state mechanism — a corrected battery (genuine `C0` + allocated
  channel CLA + paced pairs) is drafted for post-window execution.

Corrections issued during the session (kept for the record; reviewers may
meet the same traps):
1. *"The inactive slot-1 USIM holds channel 1"* — RETRACTED. Channels are
   per-card; the second physical card cannot hold a channel on the eUICC.
2. *"CLA `0x4X` is allowlist-blocked"* — RETRACTED. `0x41`/`0x42` are the ISO
   7816-4 encodings of **channels 5/6**; probes addressed channels never
   opened; refusals were no-such-channel.
3. *"GET RESPONSE never works (6D00 in every form)"* — VOID (this entry):
   probes never issued `C0`. INS-byte selection must be mechanically
   audited against the 7816-4 table before any future probe round.

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
   are available to the MT, so the T=0 hypothesis was live until correction 3 removed its motivator (the 6D00 pattern is explained by INS mis-encoding). Both alternatives
   keep the §6 plan intact but imply a patched host LPA cannot help.
6. Would a patched lpac at_csim (correct ISO channel encoding + explicit
   response-retrieval) behave differently, or is the block strictly
   modem-side? (§2 suggests modem-side; a wire-level lpac debug was not
   obtainable with this build.) Superseded by §10: the AT transports are
   closed on this firmware generation regardless; see the QMI/MBIM path. **[Update, correction 4: no longer established — client-side lowercase-hex rejection is the leading hypothesis; see correction entry 4.]**

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
switch away from ECM to expose `/dev/cdc-wdm0`, then on-router `lpac` with
the cdc-wdm backend: `chip info` gate → notification processing (first,
low-stakes ES10 write) → download + enable of the new profile → verify →
restore ECM (or adopt the new composition as baseline). External reader
remains the last-resort fallback only. Profile-disable is demoted to an
enable-ladder contingency: SGP.22 download is ISD-R-scoped and does not
require an empty enabled-profile state (vendor tooling operates on
enabled-profile cards) — the disable-first requirement was an artifact of
the blocked AT transport, not of the card.

**Build outcome (2026-10-25)**: the `qmi` backend cannot build against the
distribution feed's libqmi 1.36.0 — the UIM logical-channel APIs are
compiled out of that build (header `HAVE_QMI_MESSAGE_UIM_LOGICAL_CHANNEL`
markers commented). The `mbim` backend compiles clean against libmbim
1.32.0; a verified mipsel artifact exists lane-side (sha256 recorded in the
window runbook; independent cross-build validation pending). Composition
target is therefore MBIM (`usbnet=2`). Driver facts, source-verified:
slot selection via `UIM_SLOT` env (1-based, default 1 — an implementation
targeting the eUICC in a secondary slot MUST set it explicitly, or it
silently addresses the wrong card); MBIM command timeouts are 15 s open /
10 s per command / 20 s close (no short-timeout hazard for BPP-class
installs; hung APDUs fail loudly rather than wedging); disconnect closes
the APDU channel explicitly (no logical-channel leak). Timeout margin: the
window artifact raises the per-command timeout 10 s → 30 s (one-line patch,
build-script-baked) — every *measured* APDU class on this card family is
sub-second (p99 ≈ 400 ms), but the eUICC final-install single-APDU duration
is unmeasured and an independent implementer of the same transport chose
30 s; on a one-shot profile download the false-failure cost exceeds the
slower-hang-detection cost. Pair integrity of the two artifacts is
self-verified (only the patched driver plus a non-runtime archive-metadata
delta differ; all linked artifacts byte-identical).
