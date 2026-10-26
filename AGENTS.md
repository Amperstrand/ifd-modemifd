# AGENTS.md — NR7101 + 9eSIM Provisioning: Pickup Point for Agents & Testers

## The mission (owner's words)
> Make my NR7101 into a stable internet uplink using one or more eSIMs
> provisioned to a 9eSIM card. Without tollgate — whatever gets to the most
> reliable, robust state where the NR7101 just provides internet and can
> provision and swap between eSIM profiles.

**The first person this is for:** someone who wants their NR7101 + 9eSIM to
work with silent.link. You should be able to follow this document and the
referenced procedures to go from a stock NR7101 to a working eSIM router.

## Hardware you need

| Component | What | Notes |
|---|---|---|
| Router | Zyxel NR7101 (OpenWrt 25.12.6, ramips/mt7621) | MediaTek MT7621 SoC, no WiFi |
| Modem | Quectel RG502Q-EA (soldered) | firmware RG502QEAACR13A03M4G_ZYXEL (live AT+CGMR 2026-10-26; earlier "13A02" strings here were stale) — see #38 for upgrade track |
| SIM | 9eSIM (ECP3-class eUICC) | removable, sits in modem slot 2 |
| Bench | Any Linux host with SSH to the router | this repo assumes 192.168.1.10 on br-lan |

## What works right now (validated on hardware 2026-10-25/26)

### ✅ eSIM profile download (the provisioning core)
- **lpac** cross-built for mipsel with uppercase hex fix (the one real defect:
  stock lpac emits lowercase AT+CSIM hex; Qualcomm modems reject it)
- TLS-verified download through the on-router Gate-2 artifact (24 seconds,
  real SM-DP+, real eUICC — proven end-to-end)
- Artifact: `b587d46f…` (sha256-verified, self-contained openssl+curl+GSMA bundle)

### ✅ Profile switching (enable/disable/rollback)
- **Plain enable** (no refreshFlag): works, does NOT wedge, session lands at
  next natural reboot — **this is the canonical mechanism**
- Rollback: plain-enable the old profile, reboot, verify
- Wedge matrix: 5 mechanisms tested, all documented in
  `docs/MODEM-REFRESH-POLICY.md` — the wedge is a firmware defect, not a
  tooling defect; fresh-boot modems mostly absorb everything

### ✅ QMI raw-IP data mode (the fastest measured mode)
- `AT+QCFG="usbnet",0` → proto qmi on /dev/cdc-wdm0
- ~9.4 Mbps down (3.1× ECM mode) — mode-sweep measured
- All kernel+userland tools stock on the OpenWrt image

### ✅ Recovery from modem wedges
- 20-second physical power cut: fixes every wedge class (proven 5×)
- Pre-physical rungs (try first): HC unbind/rebind, USBDEVFS_RESET — see
  `docs/MODEM-REFRESH-POLICY.md` for the full ladder
- Register-level debug tool: `tools/gpiopoke/` (kernel module, dump-first)

## What doesn't work yet

| Blocker | Issue | Impact |
|---|---|---|
| Modem firmware wedge class | #38 | Profile switches are probabilistic; firmware 13A04 likely fixes |
| MBIM mode | #39 | No kmod/umbim in feed; QMI is available and faster |
| Speedtest eSIM data | #41 | Carrier policy blocks sustained TCP; profile management only |
| Bench UFW | #45 | Router has no internet via bench host unless owner opens UFW |

## Quick start: provision silent.link onto a 9eSIM in an NR7101

### Prerequisites
1. OpenWrt 25.12.6 on the NR7101 (sysupgrade if needed)
2. SSH access to the router (192.168.1.10 or whatever you configure)
3. The Gate-2 lpac artifact (build from this repo or use `b587d46f…`)
4. A silent.link activation code (LPA string or SM-DP+ + matching-id)

### Step 1: Deploy the tooling
```sh
# On the bench host:
scp lpac-mt7621-gate2-mipsel_24kc.tar.gz root@ROUTER:/tmp/
ssh root@ROUTER 'mkdir -p /tmp/g2root && tar -xzf /tmp/lpac-mt7621-*.tar.gz -C /tmp/g2root'
# Copy lpac driver system files (the loader needs the system path):
ssh root@ROUTER 'mkdir -p /usr/lib/lpac/driver && cp /tmp/g2root/usr/lib/lpac/driver/*.so /usr/lib/lpac/driver/'
# Build the TLS trust bundle:
ssh root@ROUTER 'cat /etc/ssl/certs/ca-certificates.crt gsma-rsp2-root-ci1.pem > /etc/ssl/certs/augmented.pem'
```

### Step 2: Find the modem's AT port
```sh
# The AT port SHUFFLES across reboots — always detect, never hardcode
ssh root@ROUTER 'for t in /dev/ttyUSB*; do
  printf "AT+CPIN?\r" > $t; sleep 2; timeout 2 cat $t | grep -q "CPIN:" && echo "AT port: $t" && break
done'
```

### Step 3: Verify the card
```sh
ssh root@ROUTER 'TTY=<detected-port>
LPAC_APDU=at_csim LPAC_APDU_AT_DEVICE=$TTY \
LD_LIBRARY_PATH=/tmp/g2root/usr/lib \
/tmp/g2root/usr/bin/lpac chip info'
# You should get an EID and card info. If euicc_init fails:
# → check AT port detection (above)
# → check the card is seated and the modem is registered (AT+CPIN? → READY)
```

### Step 4: Download the silent.link profile
```sh
ssh root@ROUTER 'TTY=<port>
LPAC_APDU=at_csim LPAC_APDU_AT_DEVICE=$TTY \
LPAC_HTTP_SSL_VERIFY=true CURL_CA_BUNDLE=/etc/ssl/certs/augmented.pem \
LD_LIBRARY_PATH=/tmp/g2root/usr/lib \
/tmp/g2root/usr/bin/lpac profile download -a "LPA:1$smdp.example.com$MATCHING-ID"'
# Takes ~24 seconds for a real profile. Watch for {"code":0,"message":"success"}
```

### Step 5: Enable the new profile
```sh
# Plain enable (no refreshFlag) — the wedge-free mechanism:
ssh root@ROUTER 'TTY=<port>
LPAC_APDU=at_csim LPAC_APDU_AT_DEVICE=$TTY \
LD_LIBRARY_PATH=/tmp/g2root/usr/lib \
/tmp/g2root/usr/bin/lpac profile enable <NEW-PROFILE-ICCID>'
# Then REBOOT the router — the session lands on the new profile at boot
```

### Step 6: Verify
```sh
ssh root@ROUTER 'reboot'
# ... wait ~3 min ...
ssh root@ROUTER 'AT+QCCID → should show the new ICCID; AT+CEREG? → should register'
```

### If the modem wedges
1. Try HC unbind/rebind: `echo 1e1c0000.xhci > /sys/bus/platform/drivers/xhci-mtk/unbind; sleep 3; echo 1e1c0000.xhci > .../bind`
2. If that fails: 20-second physical power cut (the only fix for deep wedges)
3. See `docs/MODEM-REFRESH-POLICY.md` for the full recovery ladder

## Architecture (current understanding)

```
                    ┌─────────────────┐
                    │   9eSIM card    │ (ECP3 eUICC, slot 2)
                    │  nadanada: ON   │
                    │  silentlink: ○  │
                    │  speedtest: ○   │
                    └────────┬────────┘
                             │ SIM interface
                    ┌────────┴────────┐
                    │  RG502Q-EA modem│ (AACR13A03 firmware)
                    │  QMI or ECM mode│
                    └────────┬────────┘
                             │ USB (option/cdc_ether/qmi_wwan)
                    ┌────────┴────────┐
                    │  NR7101 router  │ (MT7621, OpenWrt 25.12.6)
                    │  br-lan: .10/24 │
                    │  wan: usb0/wwan0│
                    │  lpac on-router │ ← the provisioning tool
                    └────────┬────────┘
                             │ ethernet (PoE)
                    ┌────────┴────────┐
                    │  Bench host     │ (Linux, this repo)
                    │  ssh → router   │
                    └─────────────────┘
```

## Key documents (read in this order)

1. **This file** — orientation
2. `docs/KNOWN-GOOD-ONDEVICE-PROVISIONING.md` — the validated procedure
3. `docs/MODEM-REFRESH-POLICY.md` — the wedge matrix + refresh ladder (read before touching profiles)
4. `docs/FIRMWARE-TRACK-DOSSIER.md` — the 13A04 upgrade research (blocked on owner GO)
5. `docs/ARCHITECTURE-ROADMAP.md` — the broader decisions record
6. `docs/SILENTLINK-ONESHOT-GATES.md` — the one-shot gate checklist

## Tools in this repo

| Tool | Location | Purpose |
|---|---|---|
| `tools/modem/atlib.sh` | master | Port-safe AT I/O (functional-port detection, reader-backed exchanges) |
| `tools/modem/refresh.sh` | master | Executable refresh ladder with budget enforcement |
| `tools/gpiopoke/` | master | Register-level modem power/reset control (recovery instrument) |
| `tools/modem/modem-guard/` | branch tools/modem-guard | Shutdown hook + boot health + watchdog (half-built) |
| `tests/hardware/at_csim_c0_battery.sh` | master | Corrected GET RESPONSE probe (the instrument that caught the 2C/C0 bug) |
| `tests/hardware/onrouter-readonly-session.sh` | master | Read-only session-in-a-box (rehearsed, reviewer-amended) |

## Open issues (known unknowns)

| Issue | Title | Milestone-relevant |
|---|---|---|
| #38 | Firmware upgrade track (the wedge fix) | ✅ critical for reliable switching |
| #39 | MBIM mode blocked | no (QMI available and faster) |
| #40 | Vendor QMAP mode untested | nice-to-have (may unlock more throughput) |
| #41 | Speedtest eSIM data blackhole | no (use nadanada for data) |
| #42 | SIM illegal state after switch | ✅ needs automation in modem-guard |
| #43 | **silent.link one-shot** | ✅ **THE milestone operation** |
| #44 | Modem-guard tooling | ✅ stability infrastructure |
| #45 | Bench UFW blocks router internet | workaround exists (offline install) |
| #46 | ECP3 enable variants | refinement |
| #47 | Profile nicknames | nice-to-have |

## Session history (context for why things are the way they are)

This project went through a rapid discovery arc on 2026-10-25/26:

1. **"Is CSIM dead?"** → No: the GET RESPONSE probe used wrong INS (2C not
   C0), and lpac emitted lowercase hex that Qualcomm modems reject. The modem
   and card were compliant on every count. (Corrections 3–6 in
   `findings/csim-session-state`.)

2. **"Which transport?"** → AT+CSIM with uppercase hex works for everything:
   profile download (24s verified), profile switching, notification
   processing. The custom pcscd/IFD driver approach (this repo's original
   purpose) is no longer the recommended path for provisioning.

3. **"Why does the modem wedge?"** → Firmware AACR13A03 has a USIM re-init
   fragility: any profile-change + modem reset (CFUN, REFRESH, or natural
   reboot) can hang the modem's USB controller. Probabilistic, correlates
   with reset debt, only recovers via physical power cut. The refresh
   policy (`docs/MODEM-REFRESH-POLICY.md`) encodes all the lessons.

4. **"What mode should the router run?"** → QMI raw-IP (usbnet=0) measured
   3.1× faster than ECM (9.4 vs 3 Mbps). The stock image ships QMI-native.
   MBIM is blocked (missing packages). Vendor QMAP is staged but untested.

## Rules of engagement for agents

1. **Read the refresh policy before touching profiles.** The wedge matrix
   was paid for in real incidents. `CFUN=1,1` after profile operations is
   forbidden without explicit force + budget.
2. **Artifacts before power cuts.** `/tmp` is tmpfs — pull everything off
   the router BEFORE requesting a power cut.
3. **Never hardcode the AT port.** Use `tools/modem/atlib.sh:at_func_port()`
   or the CPIN-based detection pattern.
4. **The 9eSIM card is not sacrificial.** It holds the owner's operational
   profile (nadanada). No destructive operations without owner GO.
5. **The one-shot credential (silent.link) is unconsumed.** It gets exactly
   one attempt. Every step must be rehearsed first (they have been).
6. **Species-level records in public docs.** No ICCIDs, EIDs, matching-ids,
   or unit identifiers in the repository.
7. **External-project activity is human-gated EVERY round.** Any comment,
   push, edit, or deletion on a non-Amperstrand repo (e.g.
   estkme-group/lpac) requires owner approval of the exact text/diff
   first. Approvals do not carry across review rounds, and "routine"
   workflow steps (applying reviewer suggestions, "Applied in \<sha\>"
   notes, acknowledgments) are no exception. Default to silence — the diff
   speaks. When a reply is functionally required, it reads like a
   changelog entry: no thanks, no agreement, no human persona (we are an
   AI-development account; social-performance comments read as spam).
   Incident record: 2026-10-26 lpac#466 round 2 — two un-gated comments
   posted, one with "thank you" politeness; ack deleted, reply trimmed to
   "Applied in 083d080." (docs/UPSTREAM-LPAC-AT-HEX-CASE.md).
