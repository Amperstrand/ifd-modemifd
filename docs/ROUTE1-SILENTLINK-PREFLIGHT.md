# Route 1 silent.link deployment — preflight & window checklist

Research-lane deliverable, 2026-10-25 (companion to the roadmap addendum
`e7cd7ad` and `docs/ONROUTER-READONLY-OPTIONS.md`). Route 1 = factory QMI
composition + stock feed lpac/uqmi for the silent.link one-shot.

**Credential discipline:** the SM-DP+ hostname is infrastructure (public in
every activation QR) and appears below because the precheck must be
executable. The **matching ID and any confirmation code never enter this
repo** — they live in the owner's order store (`lpac-openwrt-lane/provisioning/`
flow) and are pasted at window time only.

## 0. Facts locked this session (bench-verified 2026-10-25)

| Fact | Value | Consequence |
|---|---|---|
| silent.link SM-DP+ | `rsp-3104.idemia.io` (IDEMIA-hosted) | Idemia/Thales class = the documented mbedtls-failure class (#24963) → TLS variant swap is a **hard pre-gate**, not optional |
| TLS profile of that host | TLS 1.3 `TLS_AES_256_GCM_SHA384` / X25519; TLS 1.2 `ECDHE-ECDSA-AES256-GCM-SHA384`; cert `*.idemia.io` ← **GSMA RSP2 Root CI1** (ECDSA) | openssl handshake **PASS from bench**; ECDSA+GSMA-root is exactly the #24963 mbedtls complaint class |
| lpac HTTP leg | `driver/http/curl.c:92-93` @ `a43e46ae`: `CURLOPT_SSL_VERIFYPEER=0` + `VERIFYHOST=0` — **HIGH (reviewer AMEND 2026-10-25): the entire HTTP leg incl. the one-shot download is unauthenticated TLS.** | descriptive fact below ("precheck is faithful") stands; as *posture* it is unacceptable → **write-window HARD GATE: the download artifact MUST carry #452-class TLS verification ON against a GSMA-augmented bundle** (see §A0). Read-only milestone unaffected (no HTTP). Nuance, source-verified: SGP.22's own CI-signed server authentication is transported (`es9p.c` server/smdp certificates) and the security-critical key binding is verified by the eUICC per spec — a network attacker cannot substitute profile crypto — but all LPA-visible data (profile metadata, confirmation-code prompts, flow control) rides the unauthenticated leg |
| Remedy, prebuilt | feed `libcurl-gnutls4-8.14.1-r1.apk` (213 KiB + deps; community-proven ×2 on OpenWrt: presslab-us, kowi4); stock `libcurl4-8.23.0` coexists | no custom build needed; alternative = lane artifact rebuilt `TLS_BACKEND=openssl` |
| Router budget | NR7101: 128 MB NAND / 256 MB RAM; `lpac-2.3.0-r1.apk` = 39 KiB | no flash/RAM constraint |
| Session script | `onrouter-readonly-session.sh` @ `d0bdda3`, reviewer final verdict: runbook-ready | **runbook must set `EXPECTED_QCCID` + `EXPECTED_SLOT` privately** or the state-battery checks degrade to warn-only — the one step the script cannot enforce |

## A. Pre-switch (all reversible, no card-channel state)

**A0. Write-window lpac artifact (HARD GATE, reviewer AMEND):** the feed
`lpac` apk stays fine for the read-only R3a gate (no HTTP involved), but the
**download rung uses a lane-built artifact = pin `a43e46ae` + lpac PR #452
cherry-pick** (TLS certificate/hostname verification configurable + trust
store; open upstream, already carried by fildunsky `p9` as commit `c1ae078`
— cherry-pick source), built with verification **ON** against a
GSMA-augmented CA bundle, and stamped with the same binary-fingerprint
discipline as `atcsimUC` (`fed4c6c1`). Stock feed lpac is **not** used for
the one-shot.

1. `sysupgrade -b` → timestamped backup copied off-router.
2. Packages: `apk add libcurl-gnutls4 lpac` (uqmi already on unit — verify
   via R0 `uqmi --help | grep uim`); repoint
   `/usr/lib/libcurl.so.4 → libcurl-gnutls.so.4` (**system-wide — revert is
   on the §F restore list**); `df -h` headroom check
   after (expect a few MB total; target >20 MB free overlay).
3. **TLS precheck (router, with the repointed libcurl, explicit chain
   validation — never `-k`)**: stage an augmented bundle
   (`cat /etc/ssl/certs/ca-certificates.crt gsma-rsp2-root-ci1.pem >
   /tmp/augmented.pem`; the GSMA RSP2 Root CI1 public cert is staged from
   the bench — bench evidence: system store alone returns verify code 19
   because the GSMA root is not in public stores), then
   `curl -sv --cacert /tmp/augmented.pem https://rsp-3104.idemia.io/ -o /dev/null` —
   **PASS = clean verify + any HTTP status line** (404/405 expected);
   **FAIL = any SSL/verify error** → stop; remedy ladder: re-verify symlink
   and bundle → lane openssl artifact. The router `curl` links the same
   system libcurl lpac will dlopen, so this tests the exact code path.
   Optional level-2: `curl -s -X POST -H 'X-Admin-Protocol: gsma/rsp/v2.2.0'
   -H 'Content-Type: application/json' -d '{}' https://rsp-3104.idemia.io/gsma/rsp2/es9plus/initiateAuthentication`
   → expect an application-level JSON error (proves full HTTP+JSON routing;
   no card, no server-side state). Abort-class only; never with real
   transaction data outside the window.
4. Clock/NTP sane; AT state battery with `EXPECTED_QCCID`/`EXPECTED_SLOT`
   set in the private runbook (see §0).
5. `proto qmi` netif staged per the A2 migration note; D5/VLAN200 discipline
   reviewed; rollback tree (back to ECM) walked on paper.

## B. Switch + settle (per runbook; no verdicts from the first minutes)

1. `usbnet=0` + cold reset; TTY auto re-detect (never hardcode).
2. WAN via `proto qmi` up — the download rides the modem itself; no second
   interface needed.
3. **Settle probe** (RM551 reboot-flakiness datum): loop
   `uqmi --get-data-status` + `--uim-get-sim-state` until identical ×3 over
   ~2 min. First-minute APDU flakiness is not evidence.
4. **R3a read-only gate**: `LPAC_APDU=uqmi lpac chip info` ×3 (EID via the
   JSON-aware grep), `lpac profile list` — expect exactly `[nadanada:
   enabled]` — and `lpac notification list` (count pending: rehearsal
   ammunition for B5).
5. **Reversible-write rehearsal on the same transport**: process nadanada's
   pending notifications (`lpac notification process <seq>`) — **preferred**.
   Fallback `lpac profile nickname <ICCID> <alias>` is **also a persistent
   card write** (profile NVM metadata survives power cycles): use a test
   value, record the previous nickname first, and clean up immediately after
   the rehearsal (`profile nickname <ICCID> <old-or-empty>`). ES10-write
   class either way; proves the write path before the one-shot.

## C. Download window

- Env: `LPAC_APDU=uqmi LPAC_APDU_UQMI_DEBUG=true`, everything timestamped.
- Shape: `lpac profile download -a 'LPA:1$rsp-3104.idemia.io$<MATCHING_ID>'`
  (**single quotes** — `$` expansion has burned users; add `-c` only if the
  order carries a confirmation code).
- **No-retry discipline:** on failure lpac issues its own
  `es9+`/`es10b` cancel-session; do not re-invoke mid-session. One clean
  abort → diagnose from capture → a deliberate retry only with server-side
  state confirmed clean (fresh matching-id if the order system reissues).
- Progress watch: `es10b_get_euicc_challenge_and_info` →
  `es9p_initiate_authentication` → … → `es10b_load_bound_profile_package`.
  `HTTP transport failed` at `es9p_initiate_authentication` = the TLS class
  — should be impossible after the §A3 gate; if it happens, stop and
  capture, do not hammer.

## D. Enable + post

- **ECP quirk ladder** (documented ECP behavior): `lpac profile enable
  <ICCID>` → `<AID>` → refreshFlag `1`/`0` variants. On enable-failure:
  parking state (nadanada still enabled — never double-disable).
- Post-enable refresh: `uqmi -d /dev/cdc-wdm0 --uim-power-off --uim-slot=1`;
  ~1 s; `--uim-power-on --uim-slot=1` (feed-documented recipe, #24883;
  logical slot 1 = active card on this modem per A1).
- Verify: EID ×3; `profile list` ×3 (nadanada **disabled**, silent.link
  **enabled**); registration + signal via uqmi; tollgate WAN + balance
  (`parse_balance.sh`); send the new profile's install/enable notifications.
- **nadanada is displaced, never deleted.**

## E. Remaining unknowns (accepted, with mitigations)

| Unknown | Mitigation |
|---|---|
| mbedtls vs this exact host on-router | assumed fail (class evidence); the §A3 gate decides in 10 s — that's why it's a gate, not an assumption |
| RM551-class settle duration on this unit | bounded by the settle loop; no verdicts from first minutes |
| ECP3 final-install latency | bounded timeouts; no-retry discipline |
| Idemia SM-DP+ mid-BPP timeout behavior | unknown; single clean abort + capture is the whole strategy |
| atcsimUC's wolfssl vs this host | irrelevant to route 1 (AT path not used for the one-shot); noted only if the fallback routes ever activate |

## F. Post-window restore list

- **Restore the `/usr/lib/lpac`-adjacent staging and the
  `/usr/lib/libcurl.so.4` symlink to the stock mbedtls target** (the repoint
  is system-wide: every curl consumer flips with it — harmless for the
  router's own use, but restore anyway to keep the unit at stock shape).
- Archive the timestamped session capture + `session.log` off-router.
- Post-restore spot-check: `curl -s https://downloads.openwrt.org/ -o
  /dev/null -w '%{http_code}\n'` on the restored stack.
