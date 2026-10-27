# 13A04 field research — addendum (2026-10-27, web-only per operator; no ticket filed)

Updates dossier §2/§3 with three findings from Zyxel community/support-campus:

1. **DFOTA is base-version-checking (confirmed behavior).** A modem at
   AAAR13A02 rejected the mispackaged AACR delta with `+QIND:
   "FOTA","END",505` (version mismatch) — community transcript, thread 15326
   p.4. Consequence: the published `ZYXELR13A02→ZYXELR13A04` delta almost
   certainly expects an AACR13A02 base; whether it accepts our **13A03**
   base remains unknown and unverifiable from public data (the ticket
   question stands). Mitigating: our unit is on the same **AACR** track as
   the delta — the known failure class was the AAAR/AACR cross-track
   mismatch, which does not apply.
2. **13A04 field reports are mixed-to-negative for a subset of users**
   (thread 26363 "Downgrade R13A04 to R13A02", Oct 2024–): 24 h IP-renewal
   connectivity drops, cell-lock failures needing reboots, cell-band
   workaround required ("select at least one N band"); one user: "R13A04
   basically killed all the connectivity". An **official downgrade
   (AACR13A04→AACR13A02) exists but is distributed by Zyxel support only**.
   Net: 13A04 fixed regional connectivity/SIM-detection classes but
   introduced a new instability class for some units — the dossier's
   "moderate-weak, indirect" belief-grade for "13A04 fixes the wedge" must
   be further downgraded: 13A04 is **not unambiguously better**; it trades
   bug classes.
3. **SIM-in/SIM-out contradiction across Zyxel's own guidance:** the
   advanced-downloads article says remove the SIM during module update; the
   basic-troubleshooting article (4404106191890) says leave it inside.
   Unresolved; if a flash ever happens, prefer removal (majority + gist
   guidance) and record the contradiction in the ticket.

**Strategic consequence (consistent with the operator's call):** the
firmware track stays parked; the QMI switch-reliability series
(QMI-SWITCH-SERIES-PLAN.md) is now even more clearly the deciding
experiment — if switching is reliable on 13A03 via QMI + fresh boots,
13A04's mixed field record makes *not* flashing the better default. Any
future flash decision must weigh the 13A04 instability reports against the
wedge class it might fix, and pre-stage the support-only downgrade path.
