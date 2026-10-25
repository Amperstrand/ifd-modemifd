# Known-Good State & Behaviour — On-Device 9eSIM Provisioning (NR7101 / RG502Q-EA)

**Status: KNOWN GOOD (T-A verified 2026-10-25).** This document records
the verified-working configuration, toolchain, runtime environment,
wire behaviour, and the freely-reusable test service used to prove it —
so the working state is reproducible and drift is detectable. Species
level for production identifiers (no EIDs, no production ICCIDs, no
bench addresses); the test-service references are public by design.

Companion evidence: `INDEPENDENT-REVIEW-9ESIM-20261025.md` (full
evidence ledger and correction history, corrections 1–6), lane-side
stamped artifacts (`gate1-20261025/`, `ta-window/`), findings doc
corrections on `findings/csim-session-state`.

## 1. Hardware / firmware context (species level)

- Host: Zyxel NR7101 (MT7621, OpenWrt 25.12.x), modem AT port on
  `/dev/ttyUSB*` (shuffles across resets — always re-detect).
- Modem: Quectel RG502Q-EA, firmware `RG502QEAACR13A03M4G_ZYXEL`
  (AACR track — same class as the v4.0-proven unit).
- Card: removable ECP3-class eUICC, seated in the active slot
  (single-standby, `AT+QUIMSLOT` NVM-persisted), one enabled
  operational profile throughout all verified operations.
- Composition: **ECM (stock)** — no composition switch is needed for
  any operation in this document. `/dev/cdc-wdm` does not exist in
  this state and is not required.

## 2. The one real defect (and its fix)

The modem's `AT+CSIM` parser **rejects lowercase hex payloads with a
bare `ERROR`** (Qualcomm-class behaviour; pySim gerrit 8e15c18 and
lpac issue #428 document the class). Upstream lpac (incl. `a43e46ae`
= main at time of writing) emits lowercase from
`euicc/hexutil.c` — which is why stock lpac `at_csim` builds fail
`euicc_init` on this modem family while correctly-encoded traffic
(anything uppercase, incl. this project's driver) works.

Historical note: three earlier "modem defects" recorded during the
2026-10-24/25 investigation were all probe-construction artifacts —
the `INS 2C` "GET RESPONSE" test, the `0x4X` CLA encodings, and the
4-byte channel-close form. Correctly-encoded APDUs (5-byte close with
P3=00, allocated-channel CLAs, INS `C0`) all behave per ISO 7816-4.
See the review doc's correction log.

## 3. Known-good toolchain

| Item | Value |
|---|---|
| lpac build | `route1-gate2` stack: `a43e46ae` (pin) → `1982d5e` (uppercase hexutil) → `2c65cf6` (lpac PR #446: bounded AT poll deadlines) → `da047ad` (lpac PR #452: TLS verification configurable + trust store) → `d68be3b` (zero-init `_cleanup_` locals — fixes post-error teardown segfault) |
| Artifact | `lpac-mt7621-gate2-mipsel_24kc.tar.gz`, sha256 `b587d46fbdcbb6f21d48337651247eaf69d0e6b9a1ccbb3ac3c8250007aa9b44` |
| Binary fingerprints | uppercase table `0123456789ABCDEF` in `libeuicc.so`; `LPAC_HTTP_SSL_VERIFY` in `driver_http_curl.so`; `at_expect_with_deadline` (+ `timerfd_create` import) in `driver_apdu_at_csim.so` |
| TLS stack | bundled `libcurl.so.4` bound to openssl (`libssl.so.3`/`libcrypto.so.3` bundled) — self-contained; extraction **replaces** the system (mbedtls) libcurl at the same path (deliberate; restore = reinstall feed libcurl) |
| Trust store | GSMA-augmented `ca-certificates.crt` on the router (123 certs incl. RSP2 Root CI1); verification **ON** |

## 4. Known-good runtime environment (on-router)

```sh
LPAC_APDU=at_csim
LPAC_APDU_AT_DEVICE=/dev/ttyUSB2        # re-detected, never hardcoded
LPAC_APDU_AT_DEBUG=1                    # pin renamed from AT_DEBUG
LPAC_HTTP_SSL_VERIFY=...                # per #452 semantics — ON
```

Session hygiene that is now known unnecessary-but-cheap (see §6):
CFUN recycle before sessions clears any channel state; state battery
(`ATI`/`AT+CPIN?`/`AT+QCCID`/`AT+QUIMSLOT?`/`AT+QSIMCFG="dual_slot_status"`)
before windows, with expected values enforced privately.

## 5. The free test service (public, reusable)

- **Registry**: osmocom eUICC manual, "Known Test Profiles"
  (`euicc-manual.osmocom.org/docs/rsp/known-test-profile`) — freely
  downloadable profiles, valid for any eUICC with GSMA Live CI
  certificates, **any number of downloads, any number of cards**.
- **The one used here (T-A, verified 2026-10-25)**: the Speedtest
  profile — `LPA:1$rsp.truphone.com$QRF-SPEEDTEST` (provider
  "Speedtest Travel" / profile "BetterRoaming"; endpoint now operated
  by 1GLOBAL; chain terminates at GSM Association RSP2 Root CI1).
  Designed to register and carry data for actual speed testing, unlike
  the BetterRoaming subscription profiles in the same registry.
- **Why it matters**: (a) downloads are free-failure — diagnose and
  retry at will, the exact rehearsal the one-shot credential forbids;
  (b) the endpoint is the documented mbedtls-incompatibility class
  (openwrt/packages#24963), so a successful download validates the
  TLS remedy on the failing class itself; (c) test-profile installs
  are additive and deletable (`lpac profile delete`).
- Download shape (window discipline): single-quoted LPA string (the
  `$` separators expand under double quotes — known trap); no retry
  mid-session on failure; capture everything.

## 6. Known-good wire behaviour (reference fingerprints)

Verified live on the production card, enabled profile in place:

| Exchange | Behaviour |
|---|---|
| `AT+CSIM=10,"0070000001"` (MANAGE CHANNEL open) | `+CSIM: 6,"0x9000"` — channel allocated (2, then 3, upward) |
| ISD-R SELECT by AID, allocated-channel CLA, P2=00, case-3 | `61 21` — FCI pending, 33 bytes |
| ES10 STORE DATA → `61xx` → **chained INS C0** (`xxC00000nn`) | full data blobs (516-byte profile-info verified) + `9000` — lpac's ES10x auto-chaining works unmodified |
| ISD-R SELECT P2=0C | `61 21` (FCI offered even when not requested; fetch via C0) |
| **Channel close, 5-byte form** (`007080xx00`, P3=00) | **`9000`** — closes cleanly (lpac's form) |
| Channel close, 4-byte form (`007080xx`) | AT-level `ERROR` — malformed-APDU drop (the historical "close-refusal" artifact) |
| Any lowercase CSIM payload | bare `ERROR` (the one real defect; §2) |
| `AT+CSIM=?` | bare `OK` (no range line) |
| `AT+CFUN=1,1` | clears all channel state; ports re-shuffle; ~50 s |

Timing reference: full SGP.22 download (challenge → mutual auth → BPP
install → notification queued) **~24 s** on this transport, paced
naturally by lpac; the v4.0-era full provisioning class ran ~60 s.
Inter-command pacing is not required for lpac-driven single sessions
(the driver's 400 ms floor remains good practice for pcscd-lane
traffic).

## 7. Card state after T-A (known good)

- Operational profile: **enabled, untouched** (displacement has not
  occurred — T-B pending at time of writing).
- Speedtest/BetterRoaming test profile: **installed, disabled**,
  additive, deletable.
- Pending notifications: 3 (the two queued at the card's original
  provisioning era + the T-A install) — to be processed at T-B open.
- Free NVM after install: verified sufficient at gate time
  (pre-install reading 409,335 B class).

## 8. T-B (pending section — to be appended on execution)

Enable → speed test → rollback → verify, under the five binding
conditions (live failover check, pre-staged rollback one-liner,
time-boxed supervised session, full post-verification, default-stop).
Results and wire captures to be recorded here by the executing lane.

## 9. Reproduction checklist (condensed)

1. Verify artifact sha `b587d46f…` + the three fingerprints (§3).
2. Deploy (note the system-libcurl replacement, §3); re-detect TTY.
3. State battery with expected values enforced (privately).
4. `lpac chip info` (must return EID + `testProfileSupport`).
5. `lpac profile download -a 'LPA:1$rsp.truphone.com$QRF-SPEEDTEST'`.
6. `lpac profile list` — expect operational enabled + test disabled.
7. Compare every wire shape against §6; any drift is a finding.
