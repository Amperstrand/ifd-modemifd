# On-router read-only options — 9eSIM in the NR7101, NR7101-only tooling

**Date**: 2026-10-25 · **Author**: architecture-research lane · **Status**:
owner-constraint response; ordering verdict below is lane opinion, open to
external challenge (see "Review protocol").

**Purpose.** The owner requires a derisking milestone: **read-only operations
on the 9eSIM (ECP3 eUICC, physical slot 2, one enabled profile) using only
tooling compiled and installed on the NR7101 itself.** No external card
reader at any point (owner-final; the physical-reader plan in
ccid-firmware-rs#459 is struck by constraint, not by review). The card stays
seated in the RG502Q-EA modem. This document lays out every option considered,
the evidence base, the recommended order, and the open questions — so an
external agent can audit the reasoning.

**Success criteria for the milestone:** on the router, with no other machine
involved: `lpac chip info` returns the card's EID (×3, stable), `lpac profile
list` returns exactly the expected profile set (one enabled operational
profile), `lpac notification list` returns the pending-notification count. No
writes of any kind.

## Evidence base (graded)

| Evidence | Grade | Source |
|---|---|---|
| QMI UIM logical-channel + SEND_APDU + EF read works on this exact modem+firmware (NR7101's RG502Q-EA, AACR track) | **Exact-modem, wire-verified** | wwand `docs/STATUS.md` (2026-09-26, author's own NR7101) |
| Full SGP.22 provisioning via AT+CSIM worked on a **blank** eUICC through this driver family (one-shot, ~60 s, zero transport errors) | Exact-modem, but **blank-card state** | ifd-modemifd README evidence table (2026-10-23, v4.0) |
| With a profile **enabled**: `lpac` at_csim `euicc_init` failed ×3; MANAGE CHANNEL close AT-refused (opens leak, `6A81` after 2–3); inline-FCI `6A82` | Exact-modem, bench-stamped; **mechanism unproven** | `findings/csim-session-state` §2 + correction 3 (a46625c) |
| lpac ships in the official OpenWrt feed (2.3.0 on 25.12) with the **uqmi APDU backend as default**; community run-tested on QMI routers; post-enable UIM slot power-cycle recipe documented | Ecosystem, packaging-verified | openwrt/packages PR #24883 (merged 2024-08-28), feed Makefiles |
| MBIM UICC low-level access: open/APDU/close + EID + profiles hardware-proven on RM520N-GL (SDX62); eSIM over MBIM reported working on RM502Q-GL (SDX55) | **Same-family** | wwand STATUS (2026-10-15); lpac backend reports |
| One Quectel 5G modem (RG650E) **rejects MBIM_OPEN** entirely | Same-family cautionary | wwand STATUS "Known open" |
| pcscd + libpcsclite are packaged for OpenWrt mipsel (pcsc-lite 2.2.x; `libpcsclite_real` included; procd init; `/etc/reader.conf.d` supported) | Packaging-verified | openwrt/packages `utils/pcsc-lite` |
| CSIM pacing: v4.0 validated a 400 ms floor (300×/150× loads, zero failures) vs conwrt-bench #22 claiming 8 s needed | **Contradictory — unreconciled** | ifd-modemifd README; conwrt bench ledger |

Species-level record: no ICCIDs, EIDs, unit serials, or order tokens.

## The options

### Option 1 — lpac `at_csim` on-router (no state change)

- **How:** the lane's built+verified lpac artifact (pinned upstream a43e46ae,
  at_csim + curl/mbedtls) already runs on the router; `LPAC_APDU=at_csim`.
- **Cost:** zero (installed); minutes.
- **Transport evidence:** weakest of the AT-class family — see evidence table
  rows 3–4. lpac's AT backends are declared demo-class upstream ("FOR DEMO
  PURPOSES ONLY… Maximum Response Time is typically 300 ms"), and
  modem-firmware AT timeouts are documented aborting eUICC ops on Quectel
  EC25/EP06/RG520N (lpac PR #149).
- **Verdict:** run FIRST as a probe, with `AT_DEBUG` wire capture, right
  after a CFUN recycle (clears leaked channels — verified class). If it works,
  the milestone is achieved instantly. If it fails (expected per bench), the
  capture is a clean mechanism data point. Treat as experiment, not plan.

### Option 2 — lpac `uqmi` on-router after `usbnet=0` (factory composition)

- **How:** `AT+QCFG="usbnet",0` + cold reset → `/dev/cdc-wdm0` appears →
  `opkg install lpac` (25.12 feed; pulls uqmi) → `LPAC_APDU=uqmi` with the
  UIM slot left at the **default 1** (see flag 1 — the reviewer's exact-modem
  correction: both physical slots map to logical slot 1 on this firmware;
  the active card — our eUICC — *is* logical slot 1; a wrong value fails
  loudly `NOT_SUPPORTED(94)` and never switches cards).
- **Cost:** one deliberate composition switch + modem reboot (single-switch
  pattern is the safe class; the wedge history involves *rapid stacked*
  switches). For the read-only milestone no uplink is needed (chip info does
  no HTTP).
- **Transport evidence:** **the only exact-modem wire-proven channel** (wwand
  on this modem+firmware). Client-side caveat named: the uqmi CLI client is
  unmeasured on AACR firmware (modem side is the proven part); a read-only
  gate detects any client-side quirk before anything else happens.
- **End state:** factory/reference composition (`proto=qmi` known-good config
  documented for this unit) — but **caveat (reviewer A2)**: the unit's
  *current* baseline is ECM (`usbnet=1`) and the production WAN rides it;
  adopting the usbnet=0 end-state means a one-time WAN migration
  (`proto=qmi`, tooling present on the unit) or a deliberate switch back,
  with the VLAN200/tollgate discipline applying during the change.
- **Verdict:** primary switch-rung.

### Option 3 — lpac `mbim` on-router after `usbnet=2`

- **How:** second composition switch → MBIM UICC low-level access via the
  lane's **dually-verified artifact** (sha256-recorded; per-command timeout
  patched 10 s→30 s with the binary-level fingerprint on record;
  `SKIP_SLOT_MAPPING=true` or `UIM_SLOT=2` per the gate design).
- **Cost:** one deliberate switch; if attempted after option 2 fails, that is
  a second deliberate switch separated by verification — deliberately not the
  rapid-stacked wedge class.
- **Transport evidence:** same-family (RM520N-GL ✅ hardware-proven,
  RM502Q-GL reported ✅) plus one same-family rejection datum (RG650E rejects
  MBIM_OPEN) — weaker than option 2's exact-modem proof, stronger tooling
  provenance than anything else (our own forensic verification chain).
- **Verdict:** fallback rung.

### Option 4 — pcscd on-router + ifd-modemifd `.so` + loopback socat AT bridge

- **How:** the v4.0-proven chain collapsed on-device: router-local pcscd
  (`opkg install pcscd libpcsclite`; the "+librt" feed dependency resolves in
  the feed — it was a ghost only for our bare SDK builds), our IFD driver
  cross-built for mipsel (std-only Rust; requires nightly `build-std` — no
  prebuilt rust-std for mipsel-musl; recipe proven in the PR #30 agent work),
  reader.conf pointing at `tcp://127.0.0.1:7002` where the existing procd
  AT bridge (socat → modem tty) already listens, and lpac with the PC/SC
  backend (feed build option).
- **Cost:** the largest build effort of the five; no composition switch.
- **Transport:** AT+CSIM again — inherits option 1's transport risk **but
  through the disciplined driver** (echo-correlation, typed `AtOutcome`
  no-resend semantics, pacing floor). Diagnostic gold: if option 1 fails and
  option 4 works, the fault was lpac's AT driver, not the modem; a driver
  session is also genuine INS-`C0` traffic, feeding the session-state
  question.
- **Verdict:** medium-term rung, gated on PC/SC-any-app-on-router being a
  real requirement — it is a capability milestone, not the fastest path to
  this milestone.

### Option 5 — vpcd collapsed on-router (pcscd + libifdvpcd + modem-vpicc, all local)

Not recommended: vpcd exists to bridge a network gap; fully local there is
none. Inherits PR #30's filed defects (see pull/30#issuecomment-6394910718)
and still rides AT+CSIM. Only relevant if a *remote* PC/SC consumer returns
to scope (PR #30 disposition memo, pull/30#issuecomment-6395364117).

## Recommended ladder

**Version-lever note (reviewer-assessed):** the lane's pinned lpac a43e46ae
*is* upstream main (0 commits ahead; latest release v2.3.0) — there is no
upstream version lever to pull; the feed's uqmi surface is sufficient for
this milestone; a libqmi self-build stays deferred (driver-class, not
milestone-class); the libcurl-openssl variant is a write-time-only concern
(openwrt/packages#24963); modem firmware is explicitly **anti-derisk** — no
firmware changes.

**Per-rung preconditions (every rung, reviewer-endorsed):** debug-env
discipline with timestamps (`LPAC_APDU_UQMI_DEBUG=true`, at_csim `AT_DEBUG`),
window-start AT-level state battery (`AT+QUIMSLOT?`, `AT+QCCID`,
`AT+QSIMCFG="dual_slot_status"`, `AT+CPIN?`), opkg flash headroom + NTP
check (sane clock matters for TLS at write time and log forensics), and a
`sysupgrade -b` config backup before any composition switch.

**R1 → R2 → R0/R3a → R3b → R4** (concurs with the independent reviewer's
R1–R4 ladder, recorded in `docs/INDEPENDENT-REVIEW-9ESIM-20261025.md`; the
reviewer concurred with this lane's R3 split with amendments A1–A3, folded
in below):

0. **R0 (zero-cost pre-switch check, reviewer A3):** confirm the *installed*
   uqmi's UIM APDU surface from `uqmi --help` alone — no modem contact.
   Upstream-verified this session: OpenWrt uqmi `commands-uim.h` ships
   `uim-channel-open`/`uim-channel-close`/`uim-apdu-send` with
   `--uim-slot [1-2]`; lpac upstream `docs/backends/qmi.md` documents the
   uqmi backend for OpenWrt; openwrt/packages#24963 confirms the
   mbedtls/libcurl TLS caveat; openwrt/packages#26232 (MBIM-device vs
   uqmi-backend mismatch reports) reinforces pairing the uqmi backend with
   QMI composition specifically.
1. **R1** — at_csim probe with `AT_DEBUG` capture, post-CFUN recycle. Free.
   Decides the AT read path and feeds the mechanism question; its
   timestamped capture also resolves the pacing discrepancy (flag 3).
   **Amended (correction 4, fork-research §5): pre-registered prediction —
   the stock failures are client-side lowercase-hex rejection** (lpac
   `hexutil.c` emits lowercase; Qualcomm-class modems documented rejecting
   lowercase with fast bare `ERROR` — pySim gerrit 8e15c18, lpac #428; our
   UPPERCASE driver is this modem's proven success). A discrimination
   artifact is built and verified for the same rung:
   `lpac-mt7621-atcsimUC-a43e46ae.tar.gz` (pin + uppercase hexutil + PR
   #446 bounded deadlines; sha256 `fed4c6c1…0b4b6`; binary fingerprints:
   `0123456789ABCDEF` in libeuicc, `at_expect_with_deadline` +
   `AT command timeout (%d ms)` in driver_apdu_at_csim). Stock fails +
   uppercase flips → AT path revives with zero composition switch. Caveat
   even on success: lpac at_csim has no pacing — read-only scope plausible
   (short exchanges), write window stays on paced transports.
2. **R2** — corrected C0 battery (`tests/hardware/at_csim_c0_battery.sh`,
   issue #32). Free, read-only, under lane lock. Decides the `61xx`/`6A82`
   mechanism (correction 3 voided the old evidence; this re-establishes it
   with clean instruments).
3. **R3a** — `usbnet=0` (factory) + feed lpac `uqmi` read-only gate. The
   only rung with exact-modem transport proof; stock tooling; best end state.
   Pinned-source provenance artifact built+verified:
   `lpac-mt7621-uqmi-a43e46ae.tar.gz` (sha256 `e5e04429…8cef5d8`; driver set
   at_csim/at/stdio/uqmi + http curl/stdio; `--get-client-id uim`/
   `--uim-slot` fingerprints present; closure clean). **Expectation note
   (RM551 precedent):** reboot-state APDU flakiness on fresh composition —
   run a settle/wake-up probe before interpreting first-minute failures as
   an R3a verdict. **Caution:** the official 25.12 feed lpac ships a broken
   stdio backend (fork-research §4) — prefer pinned artifacts or the
   fildunsky `lpac-25.12.5-ramips-mt7621.apk` third binary.
4. **R3b** — `usbnet=2` + verified mbim artifact, if R3a's gate fails.
5. **R4** — pcscd-on-router build (option 4), goal-gated.

**Proposed R½ (owner-gated, pending reviewer assessment of the slot-cycle
class): card-level rehearsal on the slot-1 test USIM.** No sacrificial *unit*
exists (owner-stated), but the unit carries a plain inactive test USIM in
physical slot 1 (findings §"Slot architecture"). A reversible
`AT+QUIMSLOT=1` cycle (two modem resets: switch → exercise → switch back)
would let R1/R2/R3a *client mechanics* — uqmi CLI, lpac invocation, channel
open/APDU/close against an ordinary USIM — be rehearsed on a card where
mistakes are harmless, before the same mechanics run against the eUICC in
slot 2. Uncovered by design: ISD-R specificity (the rehearsal card has no
ISD-R). Cost: two extra modem resets and a temporary change to slot NVM (a
documented, reversible command — not in the do-not-toggle wedge class, which
was QESIM `lpa_enable`; but the wedge history counsels reviewer sign-off
before this rung is adopted). If declined, the ladder runs as R1→R4 directly
— every rung is read-only anyway; the rehearsal is comfort, not safety.

Every rung read-only up to this milestone; every gate = EID ×3 + expected
profile set + notification count, with stop-conditions on any mismatch.

**Instrument rehearsal (2026-10-25, mock-PTY):** `at_csim_c0_battery.sh`
now includes the hex-case control cell (correction-4 discriminator) and has
been rehearsed end-to-end against `tests/hardware/mock_c0_responder.py`
(raw PTY — a real modem port is raw, `stty raw -echo`; the first rehearsal
run's ECHO-feedback bug is itself a lesson in why): three scenarios all
PASS — c0-works → exit 0 `ALLOCATED-CHANNEL-OK` + `LOWERCASE-REJECTED`;
c0-fails → exit 1 `FAIL(6D00)` + `LOWERCASE-REJECTED`; case-tolerant →
`LOWERCASE-ACCEPTED(9000)`. Reproduce: see the responder's docstring.

## Cross-cutting flags

1. **Slot semantics differ per backend — do not transfer one to the other**
   (corrected after review; the lane's original flag was inverted for this
   modem):
   - **uqmi** (QMI UIM): `--uim-slot` is per-command against **logical**
     slots. On this exact modem+firmware, both physical slots map to logical
     slot 1 (wwand's measurement: SEND_APDU on logical slot 2 →
     `NOT_SUPPORTED(94)`; a channel "opened on slot 2" lands on the active
     card anyway). The eUICC is the **active** card → logical slot 1 →
     **leave the default**. A wrong value fails loudly; it never switches
     cards.
   - **mbim**: `UIM_SLOT` is 1-based **physical**, and the backend
     self-manages slot-mapping *writes* — the default (1) would actively
     re-map onto the wrong card, so set `UIM_SLOT=2` or
     `SKIP_SLOT_MAPPING=true` (source-verified in `driver/apdu/mbim.c`).
2. **TLS at future write time** (irrelevant to this read-only milestone — no
   HTTP): OpenWrt's default libcurl is mbedtls; the lpac package notes
   document an SM-DP+ TLS incompatibility class (Vodafone DE case). If the
   download window ever rides on-router HTTP and TLS fails, switch to a
   libcurl-openssl variant before suspecting the card.
3. **Pacing discrepancy, unreconciled:** v4.0's validated 400 ms CSIM floor
   vs conwrt-bench #22's 8 s claim. Different eras/conditions; R1's wire
   capture should record inter-command timing to help reconcile.
4. **Post-profile-switch quirk (future write window):** community-documented
   `uqmi --uim-power-off/--uim-power-on --uim-slot=2` slot power-cycle after
   enable (openwrt/packages PR #24883 notes) — matches wwand's power-cycle
   finding.
5. **The modem AT port shuffles across resets** — auto-detect only.
6. **libqmi UIM APIs "compiled out" = collection config, not version**
   (mechanism named by fork-research): the feed's libqmi meson `collection`
   option (minimal/basic vs full) prunes the UIM APDU message families;
   our SDK staging built a pruned collection. A full-collection libqmi
   build would carry them — native-qmi fallback stays deferred
   (driver-class), uqmi first.
7. **ECP-family enable quirks (future write window):** the official lpac
   manpage — "try AID, ICCID, refreshFlag with 1 or 0 to find out the
   working way for these chips" — plan the enable rung as an iterate-variants
   ladder, not a one-shot assumption.

## Open questions for external review

1. Is the R3a-before-R3b ordering right, given exact-modem transport proof
   (uqmi) vs verified-artifact provenance (mbim)? What evidence would flip it?
2. Does anyone hold direct evidence of the uqmi CLI client (as opposed to
   raw QMI UIM) exercising logical channels on SDX55/AACR-class firmware?
3. Can the 400 ms vs 8 s pacing contradiction be explained from the bench
   history (different card state? different firmware?), and does it affect
   R1 interpretation?
4. Is PC/SC-any-app-on-router (option 4 / R4) worth the nightly-`build-std`
   cross-build maintenance burden, given lpac-native backends cover
   provisioning and wwand covers modem+eSIM management?

## Review protocol

Challenge in place: comment on PR #33 (this document) or the tracking issues
(#32). Disagreements are recorded, not resolved by seniority; the standing
ledger lives in the review doc. Corrections to evidence citations are
especially welcome — this document's whole value is that its facts check out.
