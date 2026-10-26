# modem-guard — NR7101 modem stability infrastructure

Two components, deployed together:

## modem-guard.init — /etc/init.d/modem-guard

**Shutdown hook** (bmork's fix, OpenWrt forum t/172413): asserts the modem
RESET line at shutdown so every router reboot starts the modem clean. The
NR7101's hardware design lets the host reboot WITHOUT resetting the modem —
this hook closes that gap.

**Boot health check**: if the modem hasn't enumerated after 20s, pulses the
RESET line once before giving up.

Install: copy to /etc/init.d/modem-guard, chmod +x, /etc/init.d/modem-guard enable

## modem-guard-watchdog.sh — /usr/bin/modem-guard-watchdog

Procd daemon: probes the modem's functional AT port (CPIN-based, not bare-AT)
every 60s. On 3 consecutive failures, escalates the refresh ladder from
docs/MODEM-REFRESH-POLICY.md:
- rung 1: settle-wait (90s — post-op APDU-surface instability class)
- rung 2: QMI UIM slot power-cycle (if cdc-wdm present; no module reset)
- rung 3: gentle CFUN 0→1 (USB stays up; functional port re-detect)
- rung 4: gpiopoke rail cycle (if available; register-level)
- exhaustion: STOP — human action required (20s physical cut)

Budget: 2 reset-class ops/hour (vendor RM502Q datum: ≤30/day for flash life).
Backoff: exponential from 300s, cap 1800s.

Install: copy to /usr/bin/, source atlib.sh from tools/modem/, add a procd
service or cron entry.

## Dependencies
- tools/modem/atlib.sh (on master) — port-safe AT I/O
- tools/gpiopoke/ (on master) — optional, for rung 4
