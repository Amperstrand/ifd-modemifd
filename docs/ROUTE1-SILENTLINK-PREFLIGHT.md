# Route 1 silent.link deployment — preflight & window checklist

Research-lane deliverable, 2026-10-25 (companion to the roadmap addendum
`e7cd7ad`). **Amended post-Gate-1 (same day): the AT+CSIM path is the CHOSEN
provisioning transport — no composition switch.** T-A/T-B test-profile
phasing added per owner directive; Gate-2 artifact spec amended per reviewer
(atcsimUC base + #452 + segfault fix).

**Credential discipline:** the SM-DP+ hostnames are infrastructure (public in
activation QRs); the **matching IDs and confirmation codes never enter this
repo** — they live in the owner's order store and are pasted at window time
only. **silent.link reissue policy = owner-side item to resolve BEFORE the
one-shot** (know whether a burned code can be reissued before relying on
single-attempt discipline).

## 0. Facts (bench + live, 2026-10-25)

| Fact | Value | Consequence |
|---|---|---|
| **Gate-1 LIVE (correction 5, `bc03ee3`)** | stock lowercase ISD-R SELECT → bare `ERROR` on the real modem (R1a); **atcsimUC exit=0, FULL chip info over AT+CSIM with nadanada enabled** (R1b, capture reviewer-verified); genuine C0 returns the real ISD-R FCI; closes 4/4 | hex-case mechanism live-confirmed; AT path chosen; the read-only milestone is DONE |
| **Card facts (Gate-1 bonus)** | `rspCapability` includes **testProfileSupport**; CI PKID = **RSP2 Root CI1 SKID**; free NVM **409 335 B** | Speedtest T-A phasing is card-supported; the card's CI == the root that signs the SM-DP+ TLS chain — the augmented bundle anchors both layers; ~400 KB free fits test + silent.link profiles |
| silent.link SM-DP+ | `rsp-3104.idemia.io` (IDEMIA-hosted) | Idemia/Thales class = the documented mbedtls-failure class (#24963) → TLS variant swap is a **hard pre-gate** |
| TLS profile of that host | TLS 1.3 `TLS_AES_256_GCM_SHA384`/X25519; TLS 1.2 `ECDHE-ECDSA`; cert `*.idemia.io` ← GSMA RSP2 Root CI1 | openssl handshake PASS from bench; explicit `--cacert` needed (GSMA root not in public stores — system store alone = verify 19) |
| **T-A endpoint bonus** | bam80's #24963 mbedtls failure was against **rsp.truphone.com itself** | the Speedtest rehearsal doubles as TLS-remedy validation on the exact endpoint class that failed |
| lpac HTTP leg | `curl.c:92-93` @ pin: `VERIFYPEER=0`+`VERIFYHOST=0` — HIGH (reviewer AMEND) | **write-window HARD GATE: download artifact carries #452-class verification ON** vs GSMA-augmented bundle. Nuance: SGP.22 CI-signed server auth is card-verified (key binding untouchable by MITM), but all LPA-visible data rides the unauthenticated leg |
| **Gate-2 artifact (LANDED, two-party verified)** | `lpac-mt7621-gate2-mipsel_24kc.tar.gz` sha256 `b587d46fbdcbb6f21d48337651247eaf69d0e6b9a1ccbb3ac3c8250007aa9b44`; branch `route1-gate2` = pin → uppercase (`1982d5e`) → #446 (`2c65cf6`) → #452 (`da047ad`) → segfix (`d68be3b`); drivers at_csim/at/stdio/**uqmi** + http curl/stdio; DT_RPATH `/usr/lib/lpac` + plugins at `/usr/lib/lpac/driver/` (loader-convention match, source-verified) | Gate passed independently by two parties (pH build + 4-point; w19:pK re-verify: sha exact, ABCDEF-in-libeuicc, `timerfd_create` import = #446 structural, SSL_VERIFY marker, NEEDED closure zero-unresolved across 16 objects; inner shas recorded in session). **Bundled openssl-flavored `libcurl.so.4` REPLACES the router system libcurl — deliberate (§F restores); the gnutls repoint becomes unnecessary for T-A (alternative retained); `LPAC_HTTP_SSL_VERIFY=true` + `CURL_CA_BUNDLE=/etc/ssl/certs/augmented.pem` still required** |
| Packages | feed `libcurl-gnutls4-8.14.1-r1` (213 KiB + deps, community-proven ×2) + repoint symlink; `lpac` apk 39 KiB; NR7101 128 MB NAND / 256 MB RAM | no flash/RAM constraint; runtime TLS flavor = router's `libcurl.so.4` (build-time flavor is link-only) |
| Session script | `onrouter-readonly-session.sh` @ `d0bdda3`, runbook-ready | runbook must set `EXPECTED_QCCID` + `EXPECTED_SLOT` privately or state-battery checks degrade to warn-only |

## A. Pre-window (all reversible; no composition switch)

**A0. Gate-2 artifact (HARD GATE):** deploy only the fingerprinted
`route1-gate2` mipsel build (hexutil-ABCDEF table present in `libeuicc.so`,
`LPAC_HTTP_SSL_VERIFY` marker in `driver_http_curl.so`, zero-init segfix
linked). Stock feed lpac is NOT used for any write rung.

1. `sysupgrade -b` → timestamped backup off-router.
2. Packages: `apk add libcurl-gnutls4` (+`lpac` only if its read-only
   commands are wanted); repoint `/usr/lib/libcurl.so.4 →
   libcurl-gnutls.so.4` (**system-wide — §F restores it**); `df -h` headroom
   (>20 MB overlay target).
3. **TLS precheck (router, repointed libcurl, explicit chain validation —
   never `-k`)**: stage augmented bundle
   (`cat /etc/ssl/certs/ca-certificates.crt gsma-rsp2-root-ci1.pem >
   /etc/ssl/certs/augmented.pem`; GSMA cert staged from bench), then for
   **both** hosts: `curl -sv --cacert /etc/ssl/certs/augmented.pem
   https://rsp-3104.idemia.io/ -o /dev/null` (and `rsp.truphone.com`).
   **PASS = clean verify + any HTTP status**; **FAIL = SSL error → stop**.
   Optional L2 ES9+ endpoint probe (empty-JSON POST, abort-class, window
   time only).
4. Clock/NTP sane; AT state battery (session-script form, private
   `EXPECTED_*` set); CFUN-recycle discipline active between rungs
   (leak-channels hygiene; TTY auto re-detect after every recycle).

## T-A. Rehearsal download — Speedtest test profile (owner-approved)

Free, diagnosable, retryable; validates the ENTIRE write path (AT transport
+ TLS leg + BPP install + notification) on the card that just declared
`testProfileSupport`.

- Target: `lpac profile download -s rsp.truphone.com -m QRF-SPEEDTEST`
  (osmocom test-profile list; free, no credential).
- Env: `LPAC_APDU=at_csim`, `LPAC_APDU_AT_DEVICE=<auto-detected>`,
  `LPAC_HTTP_SSL_VERIFY=true`, `CURL_CA_BUNDLE=/etc/ssl/certs/augmented.pem`,
  `AT_DEBUG=1`, timestamped capture. Gate-2 artifact only.
- **Free-NVM gate** before (≥ 200 KB free after install target) and after;
  `profile list` must show Speedtest **present and disabled**, nadanada
  untouched+enabled; `notification list` count recorded, then
  `notification process` (the reversible-write rehearsal).
- Failures are FREE: diagnose from capture; no activation code at risk.
  Expected trap removed: if the run dies after printing an ES10 error JSON —
  that was the segfault class; the Gate-2 build must not crash there
  (mock-verified; if it crashes live, artifact regression → stop and
  re-fingerprint).

## T-B. Enable + speed test + rollback (SEPARATELY owner-gated)

Enable displaces nadanada → tollgate runs on VLAN200 for the duration →
reverse ladder restores.

- Enable via ECP quirk ladder (ICCID → AID → refreshFlag 1/0) on the
  Speedtest profile; parking state on failure (nadanada still enabled —
  never double-disable).
- Post-enable refresh on the AT path: CFUN recycle + TTY re-detect (the
  R-½ class), state battery re-run (`QUIMSLOT?`/`QCCID` unchanged).
- Verify: registration on Speedtest profile, run the actual speed test
  (owner's goal), QCCID matches the Speedtest ICCID.
- **Rollback:** re-enable nadanada via the reverse ladder, verify
  registration + QCCID back to nadanada, `notification process` both
  profiles' pending items. Optional cleanup: delete Speedtest profile
  (frees NVM; it is re-downloadable for free).

## C. silent.link one-shot (after T-A/T-B pass; owner GO)

- Same env as T-A + `lpac profile download -a 'LPA:1$rsp-3104.idemia.io$<MATCHING_ID>'`
  (**single quotes**; `-c` if the order carries a confirmation code).
- **No-retry discipline:** lpac cancel-sessions on failure; one clean abort
  → capture → deliberate retry only with confirmed server-side state
  (reissue policy resolved owner-side beforehand).
- Free-NVM gate; progress-watch vocabulary; `HTTP transport failed` at
  `es9p_initiate_authentication` = TLS class (impossible post-A3; if seen,
  stop and capture).

## D. Enable + post (silent.link)

- ECP quirk ladder; parking state on failure; nadanada **displaced, never
  deleted**.
- Post-enable refresh: CFUN recycle + TTY re-detect (AT path) — the uqmi
  power-cycle recipe applies only to the fallback annex.
- Verify: EID ×3; `profile list` ×3 (nadanada disabled, silent.link
  enabled); registration + signal; tollgate WAN + `parse_balance.sh`;
  notification send for install/enable.

## E. Remaining unknowns (accepted, with mitigations)

| Unknown | Mitigation |
|---|---|
| mbedtls vs these hosts on-router | assumed fail (class evidence + bam80's truphone case); A3 gate decides in 10 s |
| ECP3 install latency over unpaced at_csim | **rehearsed by T-A first** — the exact measurement, on the exact transport, for free |
| Idemia SM-DP+ mid-BPP timeout behavior | unknown; single clean abort + capture; T-A exercises the same class on truphone |
| post-Gate-1 close behavior under load | Gate-1 saw closes 4/4 after successful sessions; CFUN recycle between rungs keeps leak state zero |
| RM551-class reboot flakiness | applies to the fallback annex only (no switch on the chosen path) |

## F. Post-window restore list

- Restore `/usr/lib/libcurl.so.4` → stock mbedtls target; remove
  `/etc/ssl/certs/augmented.pem` if desired.
- Archive timestamped captures + `session.log` off-router.
- Spot-check: `curl -s https://downloads.openwrt.org/ -o /dev/null -w '%{http_code}\n'`.

## G. Fallback annex — uqmi/composition-switch path (NOT the chosen route)

Preserved from the pre-Gate-1 plan in case the AT write path fails T-A
irrecoverably: `usbnet=0` + cold reset → WAN `proto qmi` → settle probe
(RM551 datum) → read-only gate over `LPAC_APDU=uqmi` → same T-A/T-B/C/D
ladder with `--uim-power-off/--uim-power-on --uim-slot=1` as the post-enable
refresh. Full A2 WAN-migration caveat applies.
