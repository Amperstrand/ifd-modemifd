# FIRMWARE-TRACK-DOSSIER — NR7101 / RG502Q-EA wedge class & 13A04 upgrade track

**Status:** research dossier, consolidated 2026-10-26 (issue #53 step 1; feeds #38;
resolves the #52 ghost-reference — this file is the missing artifact AGENTS.md
points at). **Staged at `/tmp/opencode/wedge-dossier/` for owner review;
promotion into `docs/` is an owner action (repo was read-only for this task).**
Species-level record: no ICCIDs, EIDs, IMEIs, matching-ids, serials, or MACs.

---

## 1. What the wedge class IS

Firmware under study: **RG502QEAACR13A03M4G_ZYXEL** (live `AT+CGMR`
2026-10-26; the Zyxel-custom "AACR" track — not the Quectel-generic "AAAR"
track). Host: OpenWrt 25.12.6, kernel 6.12.112, MT7621 (xhci-mtk), modem
soldered on internal USB3 (2c7c:0800, 5 interfaces: 4× option tty + qmi_wwan).

### 1.1 The five documented mechanism–outcome pairs (2026-10-25, all live-tested)

| # | Mechanism | Outcome | Evidence |
|---|---|---|---|
| W1 | `AT+CFUN=1,1` 1 s after a successful profile enable (19:11Z) | **Off-bus hard wedge** — device flapped once, left USB entirely; zero attach events in any mode; `/dev/ttyUSB*` gone | MODEM-REFRESH-POLICY §1 |
| W2 | Gentle `CFUN 0→1` (19:55Z) | **Silent-but-on-bus wedge** — ports enumerate, nothing answers on any port | MODEM-REFRESH-POLICY §2 |
| W3 | Card-driven REFRESH (`lpac profile enable <ICCID> 1`, zero host resets, 20:41Z; enable itself returned `code:0`) | **Silent-but-on-bus wedge** — then 20 s physical cut → new profile ACTIVE and REGISTERED at next boot (the full enable→wedge→cut→new-profile-live arc is proven) | tb3-refreshflag-wedge.tar.gz + RECOVERY-QCCID-ARC-PROOF (lane archive) |
| C1 | Plain enable, no refreshFlag, + natural reboot | **No wedge** — canonical mechanism; session lands at next boot | AGENTS.md quick-start §5 |
| C2 | Card-driven REFRESH on a **fresh-boot** modem (sweep phase 0, ~22:15Z) | **No wedge** — `code:0`, absorbed cleanly (corroborates the probabilistic/reset-debt model) | sweep-p0-complete.tar.gz.note |

### 1.2 The deep-wedge refractory property (gpiopoke incident, 2026-10-25)

The W1 wedge survived, bit-level-verified, every software/semi-software rung:

- host reboot (full xHCI re-initialization),
- modem RESET_N pulses 1/3/30 s (sysfs gpio515),
- **30 s `lte-power` load-switch rail cut** (gpiopoke `mode=cycle`, bank0
  0x1e000600 bit18 — register-verified),
- **12 s PWRKEY force-off + boot press** (`mode=pwrkey`, bit4),
- (post-2026-10-26 policy) HC unbind/rebind and USBDEVFS_RESET are positioned
  as pre-physical rungs with only partial success on this class.

**Recovered only by the 20-second physical power cut** — the one intervention
that drops VBUS together with every rail (DTS truth: the board exposes only
`lte-power` and `lte-pwrkey`; USB power is hardwired, no VBUS switch, no hub,
no reset line). The 30 s rail cut is *longer* than the physical cut and still
failed ⇒ the differentiator is **which power domains drop**, not duration.
**The wedge state lives below the load-switch rail.**

### 1.3 Trigger correlation (what the counterfactuals C1/C2 prove)

- Wedges require **profile operation × reset-class event** (host reset, CFUN
  transition, or card REFRESH) on a modem carrying **reset debt** from earlier
  operations in the same session. Fresh-boot modems absorb the identical card
  REFRESH (C2).
- The differentiator is **not** the reset method (three methods, two wedge
  signatures, one day). It is this firmware's **USIM re-initialization after
  ANY profile change**.
- Field corroboration (policy §3): BG96 `CFUN=1,1` permanent-disconnect
  transcript (netdev 2022-12-12); Dell eSIM fleet reset-heavy automation
  bricking modems silent; RM551-class post-profile-op APDU instability;
  Quectel RG50xQ AT manual recommending hardware reset when CFUN misbehaves.
- Adjacent fragility spectrum (same firmware, milder classes): AT-port shuffle
  across resets (ttyUSB2→3→2 in one day); `+CME ERROR: 13` during USIM
  re-init; transient `euicc_init` failures immediately post-recovery that
  clear without intervention (post-incident-recovery tarball, lane archive);
  APDU-session thrash/URC flood under unpaced CSIM bursts; SIM illegal state
  after switch (#42).

### 1.4 Recovery outcomes (the complete ladder, all proven)

| Rung | Result |
|---|---|
| HC unbind/rebind, USBDEVFS_RESET | partial class-dependent success (host-side; ~2 min, no modem touch) |
| RESET_N pulse (sysfs gpio515) | ineffective on W1 |
| gpiopoke `mode=cycle` (30 s rail cut) | ineffective on W1 |
| gpiopoke `mode=pwrkey` (12 s force-off) | ineffective on W1 |
| **20 s physical power cut (drops VBUS + all rails)** | **fixed every observed wedge class (3/3)** |

Consequences: profile *switching* on 13A03 is unreliable by every software
path; the absorb-it runbook (plain enable + reboot, artifacts-before-cut,
DEFAULT-STOP on wedge signatures) is the working posture; **reliable switching
is believed to require the firmware track (13A04)**. Daily data stability
between switches is unaffected.

---

## 2. What 13A04 is believed to fix — and evidence strength

**Belief:** AACR13A04 fixes or strongly reduces the USIM re-init wedge class.

**Evidence, ranked:**

1. **Indirect (public, medium):** Zyxel's release notes pair router firmware
   V1.00(ABUV.10)C0 with module FW RG502QEAACR13A04M4G_ZYXEL
   ("optimized/tested for"); the Zyxel NR-series advanced-downloads article
   says the NR-series update line "address[es] … SIM card detection and other
   challenges" — i.e., the module-FW track's own changelog theme is SIM
   handling, the exact subsystem that wedges.
2. **Claimed, not reproducible from artifacts (weak):** MODEM-REFRESH-POLICY
   calls the 13A04 path "field-proven (owner research)" — lane-side research;
   no transcript or field report is present in the repo or the lane archive to
   reproduce it. Treat as **claimed-not-verified** until the owner surfaces
   the source.
3. **Circumstantial:** community reports credit 13A04-era updates with fixing
   region-specific connectivity problems; Mikrotik's RG502Q-based routers
   shipped a buggy modem build (same modem family, mixed vendor QA record).

**No public 13A04 changelog names USIM re-init or USB hang.** Nothing public
ties 13A04 to *this* wedge specifically. Overall strength: **moderate-weak,
indirect** — plausible, unproven. Note the baseline oddity: this unit's 13A03
is an *interim* build that does not appear anywhere on Zyxel's public ladder
(see §3), so "13A03 has the bug, 13A04 fixed it" is inference from version
ordering, not from changelogs.

**Discriminating test (pre-registered):** after any upgrade, re-run the
mechanism matrix — N≥5 profile switches via plain-enable+reboot and ≥1
card-REFRESH switch on reset-debt-free boots; wedge count 0 = fix confirmed.
A single clean switch proves little (the class is probabilistic).

---

## 3. Acquisition path for 13A04

### 3.1 Zyxel portal — the AACR (Zyxel-track) images are here, public

- **Article:** "Zyxel 5G Devices NR Series — Advanced Downloads Firmware and
  other resources" (Zyxel Support Campus EMEA, article 4403365084818;
  support.zyxel.eu / mysupport.zyxel.com mirror).
- **NR7101 section lists:** router firmware V1.00(ABUV.8/10/11)C0, the
  WebGUI modem-update SOP ("How to Update Cellular Module Firmware Via
  WebGUI"), and the sequential module ladder:
  `R01A04→R11A03→R11A06→R11A07→ZYXELR13A02→ZYXELR13A04`.
  The final step **"ZYXELR13A02 - ZYXELR13A04" Modem Code is a public
  download.** A Zyxel-EMEA SharePoint-hosted zip of that package has also
  surfaced in the OpenWrt forum thread "Zyxel NR7101 modem firmware upgrade
  process" (thread 167633).
- **Zyxel's stated posture (support ticket quote, community thread 15326):**
  the module update targets specific regional problems; if the device is
  stable, don't; when truly needed, Zyxel prefers their Taiwan team to
  install it remotely (on OEM units).

### 3.2 Quectel portal — generic track only, email-gated, wrong track

- Quectel's latest for RG502Q-EA is the **AAAR** (generic) line:
  RG502QEAAAR13A03M4G_02.002.02.002 (confirmed current by Quectel staff,
  Nov 2024), distributed by **email request** via the Quectel forum — not an
  open portal download.
- **No AACR (Zyxel-custom) firmware exists on Quectel's side.** Flashing an
  AAAR image moves the unit onto the generic track with **no documented way
  back** to Zyxel-track updates (explicit community warning). The Zyxel
  WebGUI delta path is also AAAR-hostile (2022 incident: a mispackaged zip
  produced "illegal image" / wrong-track results).

### 3.3 The base-version problem (the real blocker)

The public ladder's final delta **starts at 13A02**. This unit sits at
**13A03** — an interim version with **no published path starting from it**
(neither a 13A03→13A04 delta nor a 13A03→13A02 downgrade is on the article).
DFOTA-style packages typically base-version-check; whether the 13A02→13A04
package accepts a 13A03 base is **unknown**. Resolution options, in order:
1. Zyxel support ticket from the owner: request a 13A03-starting image (or
   confirmation that the 13A02→13A04 delta accepts 13A03).
2. Owner-gated attempt with the published delta and a documented abort path.
3. QFirehose full-image flash — but the only full images findable are
   generic-track (AAAR) or the NR5101-section AACR13A02/13A04 "Modem Code
   Files" (same module, other product line — compatibility to NR7101 units
   unverified; would need Zyxel confirmation before anyone trusts it).

---

## 4. Upgrade risk assessment

| Risk | Class | Notes |
|---|---|---|
| **Mid-flash abort / brick** | low-probability, **catastrophic**, not cut-recoverable | The 20 s physical cut fixes wedges, not bad flashes. QFirehose puts the modem in download mode; if that flow aborts, recovery is firehose-mode reflashing — undocumented on this bench, no sacrificial unit exists (AGENTS.md: no second unit). |
| **Mechanism risk on OpenWrt** | high friction | Zyxel's supported path is the OEM WebGUI push — **absent on OpenWrt**. QFirehose is the OpenWrt-world mechanism; community record: worked from a ROOter router, failed from the same user's Linux PC (host-environment sensitivity). Rehearse on nothing = first run is the live run. |
| **Base-version rejection (13A03)** | unknown | §3.3. A rejected delta may be a clean no-op — or may itself wedge; no data. |
| **Track divergence** | one-way door | Quectel AAAR flash = leaving the Zyxel track (no documented return). |
| **Vendor QA record on this line** | elevated | 2022 mispackaged update (AAAR-in-AACR-zip) caused field failures, fixed by a downgrade-to-R11A07 recovery procedure; one user reports post-13A04 cell-lock/band quirks (resolved only by router-firmware reflash dance); Mikrotik's RG502Q units shipped a buggy build. This vendor's module-update line has real scar tissue. |
| **Card safety** | procedural | Zyxel SOP: **remove the SIM during module updates.** The 9eSIM is the owner's operational card — it must be out before any flash window (rule: the 9eSIM is not sacrificial). |
| **Configuration loss** | expected, mitigable | Module FW update can reset modem config (APN/usbnet composition). Re-stage `AT+QCFG="usbnet",0` (QMI raw-IP, the measured-fastest mode) + APN after the flash; expect re-enumeration shuffle. |

**Net assessment:** GO-gated by the owner (AGENTS.md already marks #38
"blocked on owner GO"). Preconditions for any attempt: Zyxel ticket answered
(§3.3 option 1) or owner explicitly accepts the delta-rejection unknown;
SIM removed; artifacts pulled; lane lock; the silent.link credential nowhere
near the window; post-flash plan re-runs the wedge matrix (§2 test) to prove
the upgrade actually bought anything.

---

## 5. Register-baseline / diff protocol (harvesting the next wedge)

Arms issue #53 steps 3–4. **Wedges are harvested opportunistically — never
induced** (no CFUN experiments; the matrix is already complete).

### Phase A — rebuild gpiopoke (toolchain, in progress)

1. Toolchain: `make -j16 tools/install toolchain/install` then
   `make kernel/prepare` in `/home/ubuntu/openwrt`. Tree repairs made
   2026-10-26 (read-only task, build tree only): restored `target/` (was a
   working-tree deletion in git) and **replaced a wrong-target `.config`**
   (it selected octeon/EdgeRouter — the tree was NOT ramips-configured as
   assumed) with ramips/mt7621 + `DEVICE_zyxel_nr7101`, `make defconfig`'d.
   Build log: `/tmp/opencode/wedge-dossier/toolchain.log`.
2. **vermagic gate before any insmod:** the module must match the *running*
   router kernel (both sides 6.12.112 today; confirm `uname -r` on router vs
   `modinfo` vermagic of the fresh .ko — tree is v25.12.4 sources vs router
   25.12.6 build; if vermagic differs, diff the config, don't force-load).
3. Build per `tools/gpiopoke/README.md` (out-of-tree module, `M=$PWD`).

### Phase B — healthy-state register baseline

`insmod gpiopoke_k.ko` (dump-only, self-unloads) on the healthy router;
archive the bank0 dump (base 0x1e000600: DIR +0x00, DATA +0x20) **next to**
`docs/evidence/wedge/baseline-soft-20261026.txt`. From that moment the
diff-on-next-wedge protocol is armed.

### Phase C — pre-registered wedge-capture sequence (in this order, BEFORE any recovery)

1. **Soft snapshot** — exact command set of baseline-soft-20261026: date,
   `ip route`, `ip addr`, USB tree, `/dev/ttyUSB*`, `/dev/cdc-wdm*`, modem
   ident via functional port (atlib `at_func_port` — never a hardcoded port).
2. **Register dump** — gpiopoke dump-only insmod.
3. **USB device state** — full `/sys/kernel/debug/usb/devices`.
4. **dmesg tail** — ≥40 lines (attach/detach events are the W1 discriminator).
5. **xHCI state** — PORTSC via debugfs if exposed (#53 step 5; compare vs
   healthy capture below).
6. **USB-stick discriminator** — external USB-A port: works ⇒ host HC
   healthy ⇒ modem-side hang (rail-cut class); dead ⇒ host-controller shape
   (unbind/rebind becomes the candidate fix).
7. **Then** the ladder (`tools/modem/refresh.sh`), DEFAULT-STOP on either
   wedge signature. **Pull every artifact to the bench and sha256-archive
   BEFORE requesting any power cut** — `/tmp` is tmpfs and dies with it (a
   notification-evidence loss already happened exactly this way).

### Healthy reference points already captured

- Soft baseline 2026-10-26T19:21Z (repo evidence dir): modem on bus 2-1
  (SuperSpeed, 2c7c:0800, 5 interfaces), 4 ttys + cdc-wdm0, CEREG roaming,
  CSQ 20, default route via wwan0.
- This session's read-only snapshot (2026-10-26): same topology; dmesg tail
  additionally captured a **clean re-enumeration signature** — option/qmi
  disconnect burst followed 14 s later by full SuperSpeed re-attach — the
  healthy contrast shape for W1 (which shows the disconnect burst with **no**
  re-attach, ever).

---

## 6. MECHANISM ANALYSIS — where does the hang live?

### Established facts

- **F1:** W1 survived host reboot (⇒ full xHCI re-init doesn't clear it),
  RESET_N 1/3/30 s, a 30 s `lte-power` load-switch cut, and PWRKEY force-off.
- **F2:** A 20 s whole-board cut (drops VBUS + standby rails + everything)
  cleared all three wedge classes.
- **F3:** W2/W3 devices stayed enumerated while every command surface was
  silent (USB function layer alive, higher layers dead).
- **F4:** The identical card REFRESH is absorbed by a fresh-boot modem.
- **F5:** DTS truth — no VBUS switch, no hub, no reset line; only the two
  GPIO-controlled rails. No software path can equal the physical cut on this
  hardware.

### Ranked hypotheses

**H1 — VBUS-fed (or standby-fed) domain latch on the modem: USB-PHY/PMIC
state that no SoC reset reaches. (Top; explains W1 + the recovery pattern.)**
The 30 s rail cut beats the physical cut in duration and still failed ⇒ the
latched domain is *fed by a rail the load switch doesn't control* — prime
suspects are the modem's USB PHY LDOs/PMIC domain held up by VBUS, or a PMIC
latch on a standby rail. RESET_N not working is the telling datum: a SoC-wide
reset that leaves the fault in place means the fault sits **outside the
SoC's reset domain** — exactly where a PMIC/PHY latch lives. W1's zero-attach
signature is the PHY never presenting connect state to the host.
*Discriminating observation (next W1-class wedge):* (a) xHCI PORTSC after HC
reinit — CCS never asserting with the modem cabled ⇒ host never sees connect
⇒ modem-side PHY; (b) VBUS current draw at the modem during the wedge with
`lte-power` verifiably cut — persistent draw ⇒ VBUS-fed domain confirmed
(needs a metered hub / inline USB power probe on a future bench mod); (c) the
register baseline diff: if bank0 DATA shows expected levels yet behavior is
absent, host-side GPIO control is exonerated conclusively.

**H2 — Apps-processor hang with USB function layer alive. (Explains W2/W3.)**
Enumeration is served below the AP OS on Qualcomm-class modems; the AP is
deadlocked in USIM re-init (F4's trigger correlation), so ports enumerate but
no parser answers. Distinct from H1's domain: here the *hang* is a software
deadlock; the USB hardware never left.
*Discriminating observation (next W2/W3 wedge):* probe **QMI
(`/dev/cdc-wdm0`, uqmi signal/dms) alongside the AT ports** — QMI alive + AT
dead ⇒ AP task-level hang (AT parser wedged); both dead ⇒ whole apps core.
Combined with dmesg showing clean enumeration (today's healthy re-attach
signature as contrast), host side is exonerated either way.

**H3 — Host xhci-mtk controller/port latch (co-factor only).**
The unbind/rebind partial success keeps this alive, but W1 survived a full
host reboot — which re-initializes xhci completely — so a purely host-side
latch is **ruled out for W1 by evidence**. H3 survives only as (a) the
explanation for the unbind/rebind-responsive class of pre-physical rungs, and
(b) a co-factor: a misbehaving device can leave the host port state abnormal
(held resume/SE1, over-current latch) that only HC-level reset or power
removal clears.
*Discriminating observation:* the USB-stick discriminator (external port
works ⇒ HC healthy), plus PORTSC comparison healthy-vs-wedge.

**H4 — The baseband/USIM-init deadlock as the common upstream cause.**
This is the "why" behind H1/H2 rather than a distinct "where": profile
change → USIM session torn down and re-initialized → under reset debt the
re-init path deadlocks (W2/W3), and in the harder case takes the USB
function/PHY down with it (W1). Supported by the field corpus (BG96/Dell/
RM551 classes) and by C2's fresh-boot absorption.
*Discriminating observation:* any surviving command surface during W2/W3
(localizes which subsystem outlives the hang); and the ultimate test — the
§2 post-13A04 matrix re-run: if the wedge class vanishes on 13A04, the
defect was modem firmware (USIM re-init), not host or board.

### What the ranked model predicts

- No host-side software rung will ever clear a W1-class wedge on this board
  (F5) — the pre-physical rungs exist for the host-co-factor and
  silent-but-on-bus shapes only. A VBUS switch (bench hardware mod, e.g.
  metered/per-port-power hub wired in) would convert the physical cut into a
  software rung — owner decision, explicitly out of current scope.
- The register baseline + PORTSC capture on the *next* wedge is designed to
  separate H1 from H3 in one shot, and the QMI probe separates H2's depth.

---

## 7. Sources

**Repo (read-only):** `AGENTS.md` (session history, rules of engagement);
`docs/MODEM-REFRESH-POLICY.md` (wedge matrix, ladder, process rules);
`docs/ARCHITECTURE-ROADMAP.md` (Q-knob wedge history, RM520N R01/R03 datum);
`tools/gpiopoke/README.md` (register map, 2026-10-25 incident runs);
`docs/evidence/wedge/baseline-soft-20261026.txt` (healthy soft baseline);
issue #53 (workstream definition, pre-registered protocol), #38, #42, #52.

**Lane-side archive** (`~/.esim-reseller/night-20261024/evidence/`):
`ta-window/tb3-refreshflag-wedge.tar.gz` (W3, enable `code:0` then wedge);
`ta-window/RECOVERY-QCCID-ARC-PROOF.txt` (W3→cut→new-profile-live arc);
`ta-window/post-incident-recovery.tar.gz` (notification processing +
transient `euicc_init` class after recovery); `ta-window/rb2-final.tar.gz`
(clean rollback enable); `ta-window/sweep-p0-complete.tar.gz.note` (C2
fresh-boot REFRESH absorbed + routing root-cause note);
`QUECTEL-APDU-DECRYPT-TICKET.md` (Q-knob wedge-history warning);
slot2-cfun logs 2026-10-24 (CFUN recycling benign pre-profile-ops era).

**Public web (2026-10-26 searches):** Zyxel Support Campus EMEA article
4403365084818 "Zyxel 5G Devices NR Series — Advanced Downloads" (NR7101
module ladder incl. ZYXELR13A02→R13A04, WebGUI SOP, troubleshooting
downgrade); Zyxel community thread 15326 "NR7101 Module Firmware Upgrade
Fail" (regional-purpose quote, 2022 mispackaging, track-divergence warning);
OpenWrt forum thread 167633 (13A02→13A04 package link); OpenWrt forum
"Flash on Zyxel NR7101" (QFirehose behavior); Quectel forum "RG502Q-EA
Latest firmware update" p.7 (AAAR track latest, email-gated, Mikrotik
custom-build note); theodric NR7101 gist (SIM-out-during-update rule);
OpenWrt wiki NR7101 techdata (board facts).

**Live router (read-only snapshot 2026-10-26):** USB tree + dmesg tail per
§5; unit identifiers withheld by policy.
