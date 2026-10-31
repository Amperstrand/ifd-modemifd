# SGP.22 uplink proof — what works today (evidence-linked)

One page for a stranger (or a future session): what this repo's
modem-on-PC/SC stack and the validated on-router lpac chain
**demonstrably do on hardware today** — the SGP.22 provisioning chain onto
the 9eSIM, the NR7101 stable-uplink state, and the honest boundary of what
is **not yet demonstrated**. Every claim is sourced in-repo; numbers appear
only as recorded. Milestone tracker: #49.

## Evidence table

| Capability | Demonstrated how | Where the proof lives |
|---|---|---|
| Full SGP.22 provisioning through the PC/SC stack (lpac → pcscd → `libifd_modemifd.so` → AT+CSIM) | Real profile download + enable + notification on a 9eSIM (ECP3) in an RG502Q-EA, single one-shot attempt, ~60 s wall clock, zero transport errors (2026-10-23, v4.0); the profile then registered and passed live traffic | README.md evidence table; docs/RELEASE-GATE.md v4.0 row (PASS); docs/INDEPENDENT-REVIEW-9ESIM-20261025.md |
| Driver load tolerance on hardware | pySim-shell full-card soak (~60 EFs) PASS exit 0; SELECT MF ×300 (300/300, p50 400 ms); 100-byte STORE DATA ×150 (150/150); logical-channel lifecycle clean | README.md evidence table; docs/RELEASE-GATE.md evidence registry (2026-10-23 runs) |
| Transport invariants (software) | No-resend after full delivery, AtOutcome classification, echo correlation, strict CSIM framing — mock-modem battery 15/15 PASS | tests/mock_at.rs; docs/RELEASE-GATE.md software registry |
| APDU-log security | Logging off by default, 0600 creation, legacy-0644 tightening, symlink refusal (fail-closed) — 4/4 suites PASS | tests/logging.rs, tests/logging_enabled.rs, tests/logging_repair.rs, tests/logging_symlink.rs; README "Debug logging" |
| On-router provisioning chain (lpac `at_csim`, the canonical path) | Real SM-DP+ download onto the 9eSIM, TLS verification ON against the GSMA RSP2 Root CI1 trust bundle, ~24 s (T-A, 2026-10-25) via the fingerprinted Gate-2 artifact (sha256 `b587d46f…`) | docs/KNOWN-GOOD-ONDEVICE-PROVISIONING.md §3, §5, §6; docs/evidence/gate2-verification/ (sha table + four-point gate) |
| Enable arc + rollback | Enable-by-ICCID `code:0`; the staged `CFUN=1,1` refresh wedged the modem; recovery via 20-second physical power cut; **rollback COMPLETE and verified** (known-good state restored exactly) | docs/KNOWN-GOOD-ONDEVICE-PROVISIONING.md §8 (T-B, 2026-10-25); docs/MODEM-REFRESH-POLICY.md |
| Wedge-free profile switching (canonical mechanism) | Plain enable (no refreshFlag) + reboot — session lands on the new profile at next natural boot; cycle-1 of the pre-registered QMI series showed a native-QMI enable going silent-but-on-bus ~53 min, then **self-recovering** (n=1) | docs/MODEM-REFRESH-POLICY.md; docs/QMI-SWITCH-SERIES-PLAN.md; #67; docs/evidence/wedge/wedge-qmi-cycle1-20261027-analysis.txt |
| Stable uplink data path | Mode sweep (2026-10-26, clean 25.12.6): QMI raw-IP **~9.4 Mbps**, stability 5/5; ECM **~3 Mbps**, 5/5 (3.1× delta) | docs/MODE-SWEEP-RESULTS.md; #48 (closed) |
| Wedge recovery ladder | HC unbind/rebind and USBDEVFS_RESET pre-rungs; 20-second physical power cut fixes every observed wedge class (proven 5×); register-level instrument (dump-first) | docs/MODEM-REFRESH-POLICY.md; docs/WEDGE-RECOVERY-ANALYSIS.md; tools/gpiopoke/; docs/evidence/wedge/register-baseline-20261027.txt |
| Automated stability tooling | modem-guard: shutdown hook (lte-reset GPIO), boot health check, procd watchdog (CPIN probe → refresh-ladder escalation with budget+backoff) | tools/modem/modem-guard/; #44 (closed); PR #50 (commit 52b01c0), audit stamp in d183ca6 |
| Modem AT/CSIM behaviour (RG502Q-EA, AACR firmware) | Lowercase-hex rejection (bare `ERROR`), basic-channel SELECT-by-AID filter, 5-byte channel-close requirement, 400 ms pacing floor — each pinned by regression tests | README "Modem findings" section; docs/CSIM-SESSION-FINDINGS.md; tests/mock_at.rs |
| Hardware matrix | NR7101 (MT7621, OpenWrt 25.12.6) + soldered Quectel RG502Q-EA (firmware `RG502QEAACR13A03M4G_ZYXEL`) + 9eSIM (ECP3-class eUICC, slot 2) + bench host over SSH | AGENTS.md "Hardware you need"; docs/KNOWN-GOOD-ONDEVICE-PROVISIONING.md §1 |

## Narrative

Two provisioning transports are proven, and they are complementary. The
PC/SC stack — this repo's reason to exist — carried a complete real-world
SGP.22 provisioning end-to-end in one shot (~60 s, zero transport errors,
2026-10-23), with sustained-load and full-card-soak hardware evidence
behind it. The on-router lpac `at_csim` chain then repeated the class on
live hardware (2026-10-25) with a TLS-verified ~24 s download from a real
SM-DP+ onto the production 9eSIM, followed by a verified enable → wedge →
power-cut → rollback arc. That incident is the reason the refresh policy
exists: `CFUN=1,1` after profile operations is forbidden, plain-enable +
reboot is the canonical switch, and the wedge matrix plus recovery ladder
(docs/MODEM-REFRESH-POLICY.md) encode what five real wedge incidents paid
for.

The uplink itself is in its fastest measured state: QMI raw-IP at
~9.4 Mbps with a 5/5 stability gate (3.1× the ECM baseline), all on stock
OpenWrt 25.12.6 packages. Profile switching is workable but not yet
fast: cycle-1 of the pre-registered QMI switch series hit a ~53-minute
selective USIM settle that self-recovered — cut-free switching is
supported at n=1, and the wait-out watchdog (≥60 min settle rung) that
would make it unattended is the open follow-up (#67). Firmware AACR13A03
remains the wedge-class root cause; the 13A04 upgrade track is researched
and owner-gated (#38, docs/FIRMWARE-TRACK-DOSSIER.md).

The silent.link objective (#43) is done: executed and verified in the
2026-10-25/26 windows and closed 2026-10-27; the standing rule since is
rehearse-on-test-cards first, real paid eSIMs only with every gate checked
(docs/SILENTLINK-ONESHOT-GATES.md).

## Not yet demonstrated (honest boundary)

- **QMAP throughput** — staged, never run (#40; docs/MODE-SWEEP-RESULTS.md row "not yet run").
- **MBIM mode** — parked: package-level block lifted on 25.12.6, but composition switch is wedge-class and QMI is faster (#39).
- **Firmware 13A04 upgrade** — blocked on owner GO (#38, docs/FIRMWARE-TRACK-DOSSIER.md).
- **Post-v3.4 hardware round through the PC/SC driver on the RG502Q** — recorded BLOCKED in docs/RELEASE-GATE.md (software evidence green; hardware lane owns the modem).
- **Live URC-flood thrash aliasing test** — pending (README "Known limitations"; docs/RELEASE-GATE.md).
- **Serial `/dev/ttyUSBx` direct transport** — TCP/socat only today (README "Known limitations").
- **Multi-reader against two physical modems** — mock-tested only (README "Known limitations").
- **Speedtest eSIM sustained data** — carrier-policy blackhole, unusable for measurement (#41; docs/MODE-SWEEP-RESULTS.md).
- **Unattended cut-free switching at scale** — self-recovery shown once (#67); watchdog settle-rung not yet deployed.

## Tracker drift (recorded, not resolved)

#49's sub-goal checkmarks lag the repo state in three places, stated here
faithfully: A lists "Owner GO → execute the one-shot (#43)" and
"Reissue policy resolved" as unchecked while #43 is **closed** (executed
and verified 2026-10-25/26; AGENTS.md session history; truth-stamp commit
71df5c4); B lists "Modem-guard watchdog deployed (#44)" as unchecked while
#44 is **closed** and modem-guard landed (PR #50, commit 52b01c0; tools
table stamped in d183ca6). Resolution belongs to the owner; this page
follows the repo evidence, not the checkboxes.
