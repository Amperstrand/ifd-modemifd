# Firmware track dossier — RG502Q-EA (AACR) on the NR7101 (pre-decision research)

Owner decision pending; this dossier assembles the research the owner
prescribed. No firmware action is authorized by this document.

## The candidate: ZYXELR13A02 -> 13A04 (direct, our exact version)

Zyxel's official downloads page ships a **direct one-step modem update for
our exact firmware** (`RG502QEAACR13A02M4G_ZYXEL` → `ZYXELR13A04`), plus
the WebGUI upgrade SOP. Official procedure notes: sequential steps only,
up to 10 min each, reboot + version-verify between steps, and **remove the
SIM card during the module update** (card protection, aligns with our
doctrine). A recovery route exists for our exact version (troubleshooting
doc: 13A02 → R11A07 → back up the chain).

## Evidence for (why we would)

- **Field-proven on our hardware:** EskildP (Quectel forum, Oct 2025):
  NR7101 + OpenWrt, disconnects 0–4/day on 13A02-era firmware; Zyxel-path
  upgrade to 13A04 → "10 days, one single disconnect."
- **Our completed wedge matrix** (docs/MODEM-REFRESH-POLICY.md): all
  profile-switch refresh mechanisms + natural-reboot-after-flip wedge this
  firmware generation; profile switching requires this track (or the
  absorb-it posture).
- **Family changelog direction** (RM502Q-AE release notes, shared R13
  lineage): the A02→A03→A04 progression includes USB-port-recognition and
  USB-hot-swap fixes; A02's DFOTA entry literally fixes "USB port could
  not be recognized after the port was disconnected."

## Evidence against / cautions (why we might not)

- **Zyxel's own guidance:** avoid radio-module updates unless necessary —
  regional tailoring; device firmware alone suffices "if stable." Ours is
  stable *between* switches; the necessity is profile switching.
- **One-way-door risk:** Quectel RM502Q notes forbid downgrades below
  A03_01.200 for that line (Zyxel's NR7101 track retains a downgrade
  route, but treat upgrade as effectively irreversible).
- **FOTA power-stability warning:** flash damage risk if power drops
  mid-upgrade (small probability, catastrophic outcome) — bench the
  upgrade on wired power with fresh UPS/checks.
- One community instability report after R11A07→AACR13A02 was root-caused
  to 5G station switching, not firmware — no confirmed 13A02-specific
  instability in the wild on our track; no 13A04-specific complaints
  found yet (search continues before any decision).

## Vendor corroboration of the reset-budget doctrine

RM502Q-AE release notes, "Matters Needing Attention" item 4: *"the total
number of operations related to powering on/off the module, CFUN
switching, SIM card hot swapping, dual-SIM switching, or repeated
execution of NVM commands should not exceed 30 times per day."* — the
vendor documents reset debt. Our policy (2/hour) is stricter and stands.

## Open research items before any decision

1. AACR13A04-specific field reports (Zyxel community + Quectel forum,
   deeper pass; headless-Chrome forum reading per owner's toolkit note).
2. Confirm the AACR 13A04 changelog itself (Zyxel release note PDF vs the
   RM502Q sibling notes).
3. The eSIM-specific question for Quectel support: USIM re-init hang after
   eUICC profile change on AACR13A02 — known? fixed in 13A04?
   (Natural extension of the existing owner-gated support thread.)
