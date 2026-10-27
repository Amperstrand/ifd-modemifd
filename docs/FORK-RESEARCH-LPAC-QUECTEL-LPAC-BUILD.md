# Fork & Field Research: `pipe/lpac-quectel` and `fildunsky/lpac-build`

Research lane deliverable, 2026-10-25 (session: lpac-quectel & lpac-build repo
research). Sources: full clones + commit-level diffs of both repos and
`fildunsky/lpac` @ `v2.3.0-fm350-p9`; upstream `estkme-group/lpac` at our
pinned `a43e46ae` (files fetched verbatim); GitHub API for authoritative
upstream PR states; Quectel/Zyxel/OpenWrt forums, osmocom gerrit, lpac issue
tracker, 9eSIM ecosystem pages. Everything below is source-verified unless
marked "report". Companion to `INDEPENDENT-REVIEW-9ESIM-20261025.md` §F
(fork-ecosystem findings) — this document supersedes/extends that section.

## TL;DR — five findings that change decisions

1. **Hex-case smoking gun (new, strongest):** lpac encodes `AT+CSIM` PDUs in
   **lowercase** hex at our pin `a43e46ae` — and at upstream `main` today, and
   in fildunsky's patched stack. Two independent field reports document
   Qualcomm-class modems **rejecting lowercase CSIM PDUs** (bare `ERROR`),
   fixed by uppercasing: osmocom pySim (Sierra EM7565, exact AT transcript in
   their commit message) and lpac #428 (Foxconn T99W696, `euicc_init` failure,
   fix = 1-line `hexutil.c` digits change). Our own production-proven driver
   sends **uppercase** (`src/at.rs` table `0123456789ABCDEF`) and carried the
   full 9eSIM provisioning on this exact modem. The bench's ×3 `euicc_init`
   failures with lpac `at_csim` fit this mechanism exactly, and it is
   testable for free in R1's already-planned `AT_DEBUG` wire capture.
2. **The reviewer's #447 hypothesis does not apply to the CSIM path.** #447
   ("tolerate silent AT capability probes") patches `at_etsi.c` (the
   CCHO/CGLA `at` backend), not `at_etsi_csim.c`; and the at_csim probe is
   `at_expect(…, NULL, NULL)` after `AT+CSIM=?`, which **bare `OK`
   satisfies** — our modem's documented probe behavior passes stock connect.
   The proposed stock-vs-#446/#447 discrimination build is superseded by a
   cheaper, better-targeted one: stock vs **+uppercase-hexutil** (1 line).
3. **libqmi tension resolved — our finding confirmed, mechanism named.**
   OpenWrt's libqmi package has three message *collections*
   (minimal < basic < full). lpac's native QMI driver needs
   `qmi_message_uim_open/close_logical_channel_*`, which exist **only in the
   FULL collection**; `defconfig` otherwise picks **basic**, so UIM APIs are
   compiled out — exactly what we measured on the SDK-staged libqmi. The feed
   binary is likewise basic unless something (e.g. selecting modemmanager)
   forces full. fildunsky's fix: explicit `CONFIG_LIBQMI_COLLECTION_FULL=y`
   (commit `d27874e`). Version floor is a non-issue on 25.12 (1.36.0 ≥ 1.35.5).
4. **fildunsky publishes ready-made artifacts for our exact target** —
   `lpac-25.12.5-ramips-mt7621.apk` (release `v2.3.0-r4`, 2026-09-21) with
   AT(+CSIM)+QMI+UQMI+MBIM+stdio backends, and a 24.10 ipk. Not a substitute
   for our controlled builds (they still lowercase-encode CSIM), but a free
   third binary for discrimination tests. Their companion
   `luci-app-5gmodem` documents a **broken stdio backend in the official
   OpenWrt 25.12 lpac 2.3.0 package** (why they ship their own).
5. **Someone already ran lpac + 9eSIM cards on a Quectel modem** (Quectel
   forum RM551 thread, stich86): works, but APDU over AT *and* QMI go dead
   after modem reboot "until some attempts" — a reboot-state-dependence datum
   for our post-switch expectations, and independent confirmation that 9eSIM
   cards behave as standard eUICCs under lpac.

---

## 1. `pipe/lpac-quectel` (2024, stale — but its lesson is current)

Fork of `estkme/lpac`, exactly **2 commits ahead** of then-upstream (~v2.0.2,
June 2024): `2b5d94e` "Stepped down to raw io…" and `bddf841` "Added stty
stuff." Full diff read (188 insertions in `driver/apdu/at.c`):

- Replaced the stdio `fgets`-based `at_expect` with raw fd + `termios`
  (115200 8N1, raw mode, no flow control) and a `FIONREAD`+`usleep(100ms)`
  poll loop — because **upstream's line-blocking read hung forever when the
  modem stayed silent** on a probe or response.
- **Per-character writes with `usleep(10000)` between bytes** — a
  slow-write discipline predating our 400 ms inter-command floor; same
  family of "modems need pacing" evidence.
- Whole-buffer `strstr("OK"/"ERROR")` matching instead of line compares
  (tolerant to interleaved output, fragile to URCs containing "OK").
- Sends `AT+CMEE=2` at connect (upstream didn't); defaults `AT_DEVICE` to
  `/dev/ttyUSB2`.

The Soprani.ca wiki (its documentation home) pairs the at.c merge with
setting `ctx->es10x_mss = 60` in `euicc.c` — **chunk-size reduction as a
Quectel transport relief**; our pin's default is 120 (`euicc.c:159`), so the
lever still exists today. The wiki's 2026 banner: lpac via QMI/MBIM has made
these AT instructions "no longer relevant" — community consensus matches our
R3a-over-R1 ordering *for unpatched lpac*.

Relevance to us: historical root-cause corroboration (silent-line hangs,
pacing needs) + the mss-60 knob. Not actionable directly; superseded upstream
by the at-driver rewrite (#311, in our pin) and fildunsky's #446/#447/#448.

## 2. `fildunsky/lpac-build` + `fildunsky/lpac` + `luci-app-5gmodem` (active, 2026)

**What it is:** OpenWrt packaging + CI for a patched lpac 2.3.x, built for 8
targets including **ramips/mt7621 (mipsel_24kc — NR7101)**, both apk (25.12.5)
and ipk (24.10.7). Source: their lpac fork @ `v2.3.0-fm350-p9` = upstream main
+ curated patches. Origin story: eSIM on a **Fibocom FM350-GL** (Lenovo 5G
laptop modem) via lpac's *native AT driver*; the LuCI app
(`luci-app-5gmodem`) automates transport picking (AT for FM350,
MBIM/QMI for SDX55-class: Foxconn T99W175, Thales MV31-W, Dell DW5930e).

**Patch stack, mapped to authoritative upstream PR states** (GitHub API,
2026-10-25). Our pin `a43e46ae` = upstream main; anything *open* below is
**absent from our builds**:

| Patch | Upstream state | What it does | In our pin? |
|---|---|---|---|
| #446 bounded `at_expect` deadlines | **open** | deadline-bounded poll (timerfd); `AT_DEFAULT_DEADLINE_MS=10s`, recovery 30 s, probe 3 s, poll slice 500 ms | no |
| #447 tolerate silent probes | **open** | `at_etsi.c` connect probes become best-effort (FM350 answers nothing to `AT+CCHO=?` etc.) | no |
| #448 CGLA/CCHC deadlines | **open** | `AT_CGLA_DEADLINE_MS=240 s` for BPP-scale APDU chains; 30 s for CCHC cleanup | no |
| #449 bare-decimal `+CCHO` | **open** | FM350 answers `AT+CCHO` with a bare channel number, no `+CCHO:` prefix | no |
| #427 responses without prefix | **open** | general form of the above | no |
| #422 es8+ profileClass Operational | **open** | metadata default for parse compatibility | no |
| #450 QMI SIM power-cycle post-toggle | **open** | **opt-in** `LPAC_APDU_QMI_SIM_REFRESH_AFTER_PROFILE_TOGGLE=1` (+ delay env) — QMI UIM power off/on after profile enable/disable so the modem re-reads the (U)SIM without a reboot; native `qmi` backend only | no |
| #452 TLS verification configurable | **open** | trust-store / verification knobs for curl backend | no |
| LPAC_DRIVER_HOME loader | fork-only | OpenWrt strips ELF RUNPATH; 2.3.x dlopen plugins need an env fallback (`/usr/lib/lpac/driver`) | no (feed wrapper solves it differently) |
| #457 ICCID 18-digit padding | merged | even-length ICCIDs keep 0xFF padding in output | yes |
| #416 MBIM single-slot, #438 MBIM UICC compat, #429 seq-0 notifications, #420/#423/#440/#443/#444/#451/#453/#455 | merged | robustness/compat batch | yes |

Curator: Aliaksandr Babrykovich (callstack.com), July 2026. Tested-modem
table (from luci README): FM350-GL full cycle over `at`; T99W175/MV31-W
**read-only verified over `mbim`** ("EID, chip info, profile list, free
memory. Profile download not confirmed yet") — a same-class-but-different-
modem precedent for our read-only milestone shape. Also documented there: on
those MBIM modems the eUICC is reachable **regardless of which SIM slot is
active** — cannot be assumed for RG502Q (gate D2/D3 still decides), but it
shows single-slot MBIM eUICC access is a real modem class, not a fluke.

**Why this matters for R3b (MBIM rung):** someone else has already walked
"MBIM UICC + lpac 2.3.x on OpenWrt" end-to-end at read-only scope; their
robustness patches for it (#416/#438) are already in our pin.

**glib2/host-leak saga** (their commit trail `eb3c2b2`→`b63eec8`→`60a16c2`):
native QMI/MBIM pull glib2; in the OpenWrt SDK, glib2's meson build leaked
host zlib/pcre2 (`-I/usr/include`, host `libz.a`) until glib2's *target deps*
were installed as feed packages. Anyone attempting the self-built-libqmi
option (deferred driver-class lever) will hit this; the recipe is in their
CI. 24.10 keeps native QMI off solely due to libqmi 1.34.0 < 1.35.5.

## 3. The hex-case evidence in full

| Witness | Source | Content |
|---|---|---|
| osmocom pySim | gerrit `8e15c18` "transport/AT: Make sure PDU has upper case hex digits" | `AT+CSIM=14,"00a40004023F00"` → `ERROR` (no error code); same PDU uppercased → `+CSIM: 4,"612F"` / `OK`. Tested: Sierra EM7565. Fix shipped in pySim years ago. |
| lpac issue #428 | estkme-group/lpac, 2026-03-31, open | Foxconn T99W696: `at_csim` → `euicc_init` failure; fix = `hexutil.c` digits → `"0123456789ABCDEF"` (SlarkXiao fork). Reporter told "Qualcomm based WWAN modules may only accept AT+CSIM commands with uppercase". Maintainer septs: may add `LPAC_APDU_AT_HEX=UPPER` flag; "AT and AT+CSIM driver are still NOT RECOMMENDED FOR NEW DESIGN — modems have different interpretations of the ETSI TS 127 007". **No fix merged upstream or in fildunsky as of today.** |
| our own driver | `ifd-modemifd/src/at.rs` | uppercase table; full SGP.22 provisioning over AT+CSIM on RG502Q-EA, zero transport errors |
| lpac @ `a43e46ae`, upstream `main` today, fildunsky `p9` | `euicc/hexutil.c:29` | `digits[] = "0123456789abcdef"` — lowercase, used by `at_etsi_csim.c` transmit |

Mechanism check against our pin's code: `at_csim` connect sends `AT`, then
`AT+CSIM=?` with `at_expect(…, NULL, NULL)` — **bare `OK` returns 0**, so
stock connect passes on our modem (README: probes answer bare OK). Transmit
then sends `AT+CSIM=<len>,"<lowercase>"`; a pySim-class rejection (`ERROR`)
makes `at_expect` return −1 → `apdu_interface_transmit` fails → channel
open/`euicc_init` fails **fast** — matching the bench's observed failure
shape (failures, not hangs). If the modem instead stays silent, our pin's
`at_expect` uses `poll(…, -1)` — an **infinite hang** (the dead
`rv == 0` "timeout" branch never fires with timeout −1); only #446 bounds
it. R1's timestamped `AT_DEBUG` capture decides which shape this modem
produces, at zero extra cost.

**R1 amendment (supersedes the #446/#447 discrimination proposal):** build a
second artifact = pin + 1-line uppercase `hexutil.c` (optionally + #446 for
hang-bounding). If stock fails and uppercase flips `euicc_init` to success on
the wire, the AT path revives for the read-only milestone **without any
composition switch** — and the explanation for the whole bench history is a
single character case. Note for expectations even on success: lpac `at_csim`
has **no pacing**; our README documents back-to-back CSIM bursts triggering
SIM-session thrash on this firmware. `chip info`/`profile list` are short
exchanges (v4.0 soak: p50 400 ms round-trips), so read-only scope is
plausible; the write window should stay on paced transports (our pcscd
driver chain, or an mss/pacing-hardened lpac build).

### Offline rehearsal — prediction CONFIRMED at wire level (w19:pJ, 2026-10-25)

Both artifacts rebuilt x86 AT-only and run against
`mock_modem.py --case-sensitive` (new Qualcomm-class lowercase-rejection
fingerprint mode, committed `c33e7f7`, default off) over a pty bridge.
Captures: `/tmp/opencode/r1-{stock,upper}-{atdebug,out}.log`.

- **STOCK `a43e46ae`:** `AT`/`AT+CSIM=?` probes pass (the #447 refutation,
  now live-confirmed); `AT+CSIM=10,"0070000001"` → `+CSIM: 6,"019000"`
  (channel opens — digits only, case-irrelevant);
  `AT+CSIM=42,"01a4040010a000…"` (ISD-R SELECT, lowercase) → **bare
  `ERROR`** → `{"code":-1,"message":"euicc_init"}` — the exact bench
  signature with the mechanism on the wire. The opened channel is **never
  closed** (close lives in the success path).
- **PIN + uppercase:** same open succeeds; uppercase SELECT
  `02A404…A000000559…` → `+CSIM: 16,"6F048402A0009000"` — **FCI answered**,
  ISD-R selected; proceeds into ES10 `getEID` STORE DATA
  (`82E2910006BF3E035C035A` class) → mock answers `6D00` (mock implements
  no STORE DATA) → lpac prints the `es10c_get_eid` error JSON and then
  **segfaults** in the post-error path (see below). Real ISD-R answers
  STORE DATA, so R1 live is not expected to hit it.
- **RX safety:** lpac's receive-side hex parsing case-folds (`| 0x20`) —
  the uppercase TX patch cannot break RX decoding of modem responses
  (which come back uppercase anyway).
- **Defect model (reviewer-corrected 2026-10-25; supersedes the single
  causal chain first written here):** THREE independent defects are now in
  evidence, none requiring the enabled-profile theory:
  1. **lowercase CSIM rejection** — client-side (lpac `hexutil`), primary
     for the bench ×3;
  2. **channel-CLOSE refusal** — an **independent firmware defect**: the
     bench's correctly-encoded *uppercase* MANAGE CHANNEL closes were also
     refused; do **not** demote this to a case-bug symptom;
  3. **exhaustion / `6A81` after 2–3 opens** — downstream of #2 plus leaked
     channels (lpac's close lives in the success path, so every failed
     init — stock or uppercase — leaks one more channel).
  Only the `61 21` / `6A82` anomalies remain unexplained — R2 scope.
- **Status change for R1:** on-router R1 is now a **confirmation run, not
  exploration** — the mock fingerprint (pySim/#428 shape) predicts: bare
  `ERROR` on the lowercase SELECT from stock, `+CSIM:` FCI from the
  uppercase build.

**New upstream lpac bug observed (candidate report — filing is an owner
decision, external publish):** on a card-error SW for an ES10 STORE DATA
exchange (mock `6D00` after a successfully selected ISD-R), the CLI prints
the correct `es10c_get_eid` error JSON and then crashes
(`timeout(1)`: "the monitored command dumped core") — i.e. the failure is
in the post-error cleanup/exit path, reachable any time an ES10 exchange
ends in a card-error SW. Reproducer: stock-or-upper x86 AT build +
`mock_modem.py --case-sensitive` pty bridge + `r1-upper-*` transcript
(above). Suggested report title: "lpac CLI segfaults after card-error SW
on ES10 STORE DATA (at_csim path)".

## 4. Field reports (forums/issues) — challenges and solutions

| Report | Where | Relevance |
|---|---|---|
| **lpac + 9eSIM + Lenovo eUICCs on Quectel RM551E-GL** — AT and QMI APDU dead after reboot/CFUN until "some attempts", then works; Quectel: eSIM unsupported on that module → "unpredictable problems" | Quectel forum "RM551 LPAC Issue" (stich86, 2025-07) | Independent lpac+9eSIM-on-Quectel precedent; reboot-state dependence of APDU surfaces — budget a wake-up/settle probe after our composition switch + cold reset before interpreting R3a/R3b results |
| RM520N-GL: lpac at backend dead until **modem firmware update** (04.204→04.209), then download works (quoting-error red herring documented) | lpac #182, Quectel forum 44479 | Same-vendor-family firmware sensitivity of AT APDU; supports "don't generalize across firmware generations" already in our record. Our firmware string matches the v4.0-proven unit — no action |
| Khadas forum RM500Q-GL: pipe/lpac-quectel listed/select/enable/disable/delete OK over AT but **not download**; official `quectel_lpad` worked only with newer firmware + QMI-aware option driver + usbnet mode; `AT+QESIM` covers most management on RG520N-class firmware | Khadas thread 21975 (steely-glint) | Already cited by reviewer; adds: QESIM-native management as vendor path we deliberately don't take (wedge class), and AT-download failures being the norm on this vendor family |
| uqmi backend origin: "APDU operations were aborted due to exceeding the timeout imposed on the AT interface by the modem firmware" — uqmi backend created *because* AT was a dead end; tested EC25/EP06/RG520N | lpac PR #149 (blocktrron) | The canonical statement of the AT-timeout wall; our R3a rests on this lineage |
| Feed uqmi package: **power-cycle recipe documented verbatim** (`uqmi --uim-power-off/--uim-power-on --uim-slot=1` after profile switch) + mbedtls/Vodafone-DE TLS note; uqmi is the feed **default** backend since 2024-08 | openwrt/packages #24883 (merged), #24963 | Confirms both cross-cutting flags in ONROUTER-READONLY-OPTIONS.md; #450 is the in-tree, opt-in generalization (native QMI only) |
| MBIM-vs-uqmi mismatch: MBIM-composition devices can't use the uqmi backend (needs `-m`); resolved by adding the MBIM backend | openwrt/packages #26232 → #26237 | R3b's tooling premise (MBIM backend exists in feed since 2025-04, in our pin) |
| Zyxel NR7101 eSIM: "eSIM module is present in the hardware, but usage is reserved for SP-customized appliances" (Zyxel support); community asks "Maybe with OpenWrt will work?" | Zyxel community thread 15352 | Vendor confirmation of hardware-present/SP-gated eSIM on NR7101 — our project is the answer to that thread |
| NR7101 QMI-composition USB-bus crashes at high throughput (2022 claim) vs other users' stable QMI | OpenWrt forum "Flash on Zyxel NR7101" p.4 (Stiel vs others) | Watch-item for R3a's factory-QMI end state (throughput stress class, not APDU class); our own unit's factory-QMI baseline and tollgate evidence outrank a 2022 report, but record as known community datum |
| lpac AT driver vs TS 27.007: `+CCHO` returns a *handle*, not a channel number; modem patches CLA; lpac assumed otherwise (MC7455) | lpac #138 (laf0rge) | Why upstream calls AT drivers "NOT RECOMMENDED FOR NEW DESIGN" — spec-divergence across vendors; our driver avoids the class by raw CSIM |
| ECP removable eUICCs (9eSIM's OS family) have **profile-enable quirks** — official lpac manpage: "try AID, ICCID, refreshFlag with 1 or 0 to find out the working way for these chips" | lpac(1) Debian manpage | Directly de-risks our future enable rung: on ECP3, expect to iterate enable variants rather than assume one-shot |
| 9eSIM card ecosystem: lpac 2.1.0 over PC/SC does chip info / profile list / download fine (V0-class ECP card; Kigen-backed SM-DP+ default address) | neilzone.co.uk 2025-01 | Card-side behaves as standard eUICC under lpac — the modem transport, not the card, is our risk surface |
| RM500Q/RG50xQ vendor eSIM path: `AT+QESIM` family (300 ms max response per command) + NDA-gated `quectel_lpad` | Quectel "eSIM LPA Application Note" PDF (forum-uploaded) | The vendor's own 300 ms AT response-time class and closed LPA — context for why the open path is lpac+uqmi |

## 5. Impact on the ladder (proposed amendments, for the lanes' verdicts)

- **R1 (at_csim probe):** keep exactly as planned (free, wire-captured).
  ~~Pre-register the hex-case prediction~~ — **prediction confirmed offline**
  (§3 rehearsal): stock fails at the lowercase ISD-R SELECT with bare
  `ERROR` (bench signature); the uppercase build gets the FCI answered.
  On-router R1 is a confirmation run against the real modem. Keep the
  **second artifact** (pin + uppercase `hexutil.c`, + #446 recommended to
  bound any silent hangs) — a live success re-ranks the whole AT path for
  the read-only milestone without touching composition. #447/#448 are
  `at`-backend patches and do not gate this rung. One rehearsal caveat to
  carry: the uppercase build's ES10 path segfaults on card-error SWs
  (§3) — real ISD-R answers STORE DATA, but if the live run dies *after*
  printing an ES10 error JSON, that's this bug, not a transport finding.
- **R2 (C0 battery):** unchanged; its uppercase framing now doubles as the
  hex-case control experiment.
- **R3a (factory QMI + feed lpac/uqmi):** unchanged and further corroborated
  (PR #149 lineage; power-cycle recipe in #24883 for the future write
  window). Add one expectation note: RM551-class reboot-state flakiness —
  don't interpret first-minute APDU failures on a fresh composition as
  R3a-verdict data without a settle/wake-up probe. Optional third binary for
  discrimination: fildunsky's `lpac-25.12.5-ramips-mt7621.apk`.
- **R3b (MBIM):** unchanged; note the fildunsky read-only-over-MBIM
  precedent (different modem) and that #416/#438 robustness is already in
  our pin. Slot-independence of MBIM eUICC access is modem-class — D2/D3
  gates remain.
- **R4 / pcscd milestone:** unchanged.
- **Write window (future):** record #448's 240 s BPP deadline class, #450's
  opt-in refresh env (native qmi), the uqmi manual power-cycle recipe, the
  ECP enable-quirk manpage note, and the TLS/libcurl-openssl flag (#24963) —
  all already consistent with the options doc's cross-cutting flags.
- **Version-lever note update:** still true that our pin = upstream main and
  no release lever exists; but "nothing newer to adopt" now has two
  one-line-adjacent exceptions worth naming: upstream-open #446 (hang
  bounding) and the #428 uppercase fix (neither merged anywhere as of
  today). Both are cherry-picks into our own build, not version bumps.

## 6. Cheap follow-ups if the lanes want them

1. R1 wire capture review vs the mock/pySim fingerprint — now a
   confirmation checklist (minutes, after R1).
2. Uppercase-patched build via the lane's existing `build-lpac-openwrt.sh`
   (one-line sed into the pinned tree or a patch file in the build repo);
   mock-mode dry-run of the mipsel artifacts via `mock_modem.py
   --case-sensitive` before deployment.
3. If R3a is reached: install-check fildunsky's mt7621 apk alongside the
   feed package (different `LPAC_DRIVER_HOME` wrapper shape — good packaging
   cross-reference for our own artifacts).
4. Upstream reports — **TWO separate owner-gated artifacts, no bundling**
   (reviewer decision 2026-10-25): (a) a comment on existing lpac #428 —
   RG502Q-EA as a second Qualcomm-class data point + the rehearsal wire
   captures + the maintainer's own unimplemented
   `LPAC_APDU_AT_HEX=UPPER` suggestion; (b) a **new** segfault issue
   (draft title/transcript in §3), cross-referenced to (a). Sequencing:
   file **after** R1's live wire capture so the upstream claim carries
   live confirmation; species-level content only (mock artifacts + public
   AIDs; no bench context or identifiers).

## Source index

- `pipe/lpac-quectel` clone @ `bddf841` (2024-06-28); diff `d508b28..bddf841`.
- `fildunsky/lpac-build` clone @ `5907363`; `fildunsky/lpac` @ `v2.3.0-fm350-p9`.
- Upstream `estkme-group/lpac` @ `a43e46ae`: `driver/apdu/at_etsi_csim.c`,
  `driver/apdu/at_cmd_unix.c`, `euicc/hexutil.c`, `euicc/euicc.c` (fetched
  verbatim); `main` hexutil checked 2026-10-25.
- GitHub API PR states (estkme-group/lpac, #411–#459), 2026-10-25.
- osmocom gerrit pySim `8e15c18`; lpac issues #35/#73/#94/#138/#149/#175/
  #182/#221/#428; openwrt/packages #24883/#24963/#26232/#26237.
- Forums: Quectel (RG502Q-EA eSIM firmware request 18852; RM551 LPAC Issue
  48986; Esim Provisioning RM510Q 22278; eSIM/eUICC hardware 1953; eSIM
  Introduction PDF; RG50xQ eSIM LPA App Note PDF), Khadas 21975, Zyxel
  community 15352, OpenWrt forum "Flash on Zyxel NR7101" 116006.
- 9eSIM: 9esim.com FAQ/tooling pages; neilzone.co.uk 2025-01 review; lpac(1)
  manpage (Debian).
- Soprani.ca wiki "lpac via USB modem / PatchingForQuectel" (via search
  snapshot; hub page fetched 2026-10-25).

---

## Lane closure — final status (2026-10-27, crash-relaunch charter)

This research lane's arc completed end-to-end between 2026-10-25 and
10-27: `pipe/lpac-quectel` decoded (2024 raw-io/pacing workaround,
historical corroboration), `fildunsky/lpac-build` decoded (curated
AT-robustness stack + the libqmi collection mechanism + ready-made
mipsel artifacts), and the two forks' combined evidence produced the
**lowercase-hex smoking gun** — which became correction 4, the
`route1-gate2` artifact (`b587d46f`), the 24 s T-A rehearsal download,
the executed-and-verified silent.link provisioning (#43 closed), and
upstream filings lpac #428-comment / PR #466 / PR #467 (+ the #419
driver-layer teardown root-cause by successor lanes, draft owner-gated).
Verification records archived: `docs/evidence/gate2-verification/`.

**Final external states at closure (2026-10-27):** lpac PRs #466, #467,
#452 all **open awaiting maintainer action** (approvals in hand on
#466/#467; #467 format pass landed; post-merge steps parked at
Amperstrand/lpac#4); lpac issue #419 comment **owner-gated** (recalibrated
draft at `docs/upstream-reports/issue-419-crossref-comment.md`, tracker
Amperstrand/lpac#6); lpac upstream tag still `v2.3.0`, zero merged drift
affecting our pin; `fildunsky/lpac-build` unchanged at `v2.3.0-r4`
(2026-09-21). **No actionable drift since the research; watch items live
on the Amperstrand/lpac trackers, not in this doc.**

Open-question disposition (§6 follow-ups): R1 wire-capture review — done
(Gate-1 live); uppercase mipsel build — done (`route1-gate2`/`b587d46f`);
fildunsky apk install-check — moot (R3a composition switch never
happened; AT path won).
