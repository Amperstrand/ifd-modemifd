# silent.link One-Shot — Final Gate List (post-T-A reality)

> **RECORD CLOSED 2026-10-27 (audit): the one-shot EXECUTED successfully in
> the 2026-10-25/26 windows** — silent.link profile downloaded onto the 9eSIM,
> present on-card by the 2026-10-26 QMI singlemode window (enable/disable
> exercised over QMI, G5b/c PASS), data check on APN `internet` run (session
> record `2026-10-26-nr7101-qmi-singlemode-window.md`), procedure validated
> as the AGENTS.md quick start; #43 closed. G2 was resolved by owner policy
> (2026-10-27): reissue questions moot — rehearsal-first is standing law.
> This document is now the historical gate record.

**Supersedes the D2–D6 gate structure** in the review record's §D,
which was written for the abandoned MBIM-composition path. Every
composition-era gate is moot: the provisioning transport is **plain
AT+CSIM in stock ECM composition**, proven end-to-end by T-A (full
SGP.22 download, mutual auth + BPP install, ~24 s, TLS verification on,
operational profile untouched). This is the complete gate list the
one-shot window runs on.

## Gates (all must be green before the download command)

| # | Gate | Status / owner |
|---|---|---|
| G1 | T-B complete and recorded (known-good §8): enable + rollback verified (the speed-test measurement itself was attempted three ways and is blocked on modem firmware — see the wedge matrix in MODEM-REFRESH-POLICY and tb3 evidence 0a681623); operational profile re-enabled and registering (QCCID+CEREG verified); notification queue emptied (rc=0). CLOSED 2026-10-25 — record: known-good §8 + 379044d; refreshFlag rehearsal landed post-closure (wedge #3, archived) | executing lane (CLOSED) |
| G2 | **Reissue policy answered**: owner confirms with silent.link whether a failed download yields a fresh matching-id. This single answer sets the retry posture of the whole window | **owner** |
| G3 | TLS evidence: **earned** — T-A's live download through the Gate-2 openssl-libcurl build against the truphone endpoint (the documented #24963 failure class) is the router-side proof; no further router precheck exists or is needed (router has no curl; host-side bench chain verified CI1-rooted) | closed |
| G4 | Notification queue empty at window open (G1's `notification process -a` closes it; a T-A-class install re-queues one per download, processed in-window) | executing lane |
| G5 | Credential staged privately: matching-id from the order store only at window time, single-quoted LPA string, never in any repo/log path | executing lane |
| G6 | Free NVM re-checked (`chip info` freeNonVolatileMemory sufficient for one more profile of silent.link's size class) | executing lane |
| G7 | Rollback pre-staged verbatim in the private runbook: enable-failure → leave new profile disabled, nothing lost; registration-failure post-enable → re-enable the displaced operational profile by ICCID (the T-B-rehearsed one-liner) | executing lane |
| G8 | **Owner GO** on the window, with the no-retry discipline restated: any mid-download failure → STOP, capture, diagnose; retry only with G2's answer in hand | **owner** |

## Explicitly NOT needed anymore

Composition switch (usbnet), slot-mapping protocol (UIM_SLOT /
SKIP_SLOT_MAPPING), MBIM/QMI artifacts, WAN migration, external reader,
mid-window NTP/package installs. The toolchain is the Gate-2 artifact
(sha `b587d46f…`, stack `d68be3b←da047ad←2c65cf6←1982d5e←a43e46ae`)
plus the augmented CA bundle already on the router.

## Execution shape (unchanged disciplines)

modem-lane lock + heartbeats · state battery with enforced expected
values · CFUN recycle before the session · `LPAC_APDU_AT_DEBUG=1`
capture throughout · stamped artifact dir · one-shot download command
shape per the preflight doc · post-download: profile list diff, enable
ladder (ICCID → AID → refreshFlag), notifications, registration verify,
record (private ops DB for identifiers; species-level in public docs).
