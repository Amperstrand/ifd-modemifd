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
| lpac HTTP leg | `driver/http/curl.c` sets **no certificate verification** | chain trust is irrelevant to lpac; the only failure mode is handshake crypto — a plain `curl` round-trip is a faithful precheck |
| Remedy, prebuilt | feed `libcurl-gnutls4-8.14.1-r1.apk` (213 KiB + deps; community-proven ×2 on OpenWrt: presslab-us, kowi4); stock `libcurl4-8.23.0` coexists | no custom build needed; alternative = lane artifact rebuilt `TLS_BACKEND=openssl` |
| Router budget | NR7101: 128 MB NAND / 256 MB RAM; `lpac-2.3.0-r1.apk` = 39 KiB | no flash/RAM constraint |
| Session script | `onrouter-readonly-session.sh` @ `d0bdda3`, reviewer final verdict: runbook-ready | **runbook must set `EXPECTED_QCCID` + `EXPECTED_SLOT` privately** or the state-battery checks degrade to warn-only — the one step the script cannot enforce |

## A. Pre-switch (all reversible, no card-channel state)

1. `sysupgrade -b` → timestamped backup copied off-router.
2. Packages: `apk add libcurl-gnutls4 lpac` (uqmi already on unit — verify
   via R0 `uqmi --help | grep uim`); repoint
   `/usr/lib/libcurl.so.4 → libcurl-gnutls.so.4`; `df -h` headroom check
   after (expect a few MB total; target >20 MB free overlay).
3. **TLS precheck (router, with the repointed libcurl)**:
   `curl -sv https://rsp-3104.idemia.io/ -o /dev/null` —
   **PASS = any HTTP status line** (404/405 expected on `/`); **FAIL = any
   SSL error** → stop; remedy ladder: re-verify symlink → lane openssl
   artifact. The router `curl` links the same system libcurl lpac will
   dlopen, so this tests the exact code path.
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
   pending notifications (`lpac notification process <seq>`); if the queue
   is empty, `lpac profile nickname <ICCID> <alias>` instead. ES10-write
   class, zero card risk, proves the write path before the one-shot.

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
