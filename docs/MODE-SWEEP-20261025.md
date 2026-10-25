# Mode sweep — systematic benchmark (2026-10-25/26)

Owner-directed: test **all four modes** with speedtests run from the
bench host (LAN client behind the NR7101), producing a measured
per-mode table to replace the community-researched estimates in
known-good §10/§11.

## Instrumentation

- Bench host: L2-adjacent on the router LAN (`192.168.1.127/24` →
  `192.168.1.10`); per-destination routing via
  `sudo ip route add <target>/32 via 192.168.1.10 dev enx00e04c3e7b58`
  (removed after each pass — host's own uplink untouched).
- Tool: `iperf3 3.16` (host). Bounded quick-pass per mode: 2×10 s down,
  2×10 s up against a public iperf3 server, best-of recorded.
  Optional canonical pass: Ookla CLI on sponsored data (see budget).
- Data budget (owner to confirm, default in brackets): bounded
  quick-pass ≈ 300 MB/mode → ~1.2 GB total worst case, **through the
  active operational profile** [default: proceed]. Alternative: enable
  the Speedtest Travel profile first (sponsored data) at the cost of
  two planned profile-ops (wedge-expected + 20 s cuts).

## Phases (each: switch → WAN verify → settle → measure → record)

| Phase | Mode | Change required |
|---|---|---|
| 1 | **ECM** (baseline) | none — measure current state first |
| 2 | **MBIM** (`usbnet=2`) | reboot; kmod-usb-net-cdc-mbim + umbim install (fits overlay); proto mbim |
| 3 | **in-kernel QMI** (`usbnet=0`) | reboot; qmi_wwan + uqmi (present); proto qmi |
| 4 | **vendor q_drivers QMAP** | SDK cross-build (July 2026 gist, kernel-6.12-patched); qmi_wwan_q kmods + quectel-CM |
| 5 | **revert to ECM** | final state = known-good; full verify + report |

Discipline: modem debt is freshly cleared (post cut-#3) and the sweep
is data-path only (no profile ops → no enable-wedge class); every
composition flip is a cold reset — budget ≈ 4 resets + reboots,
wedge recovery standing by (owner's hand). Lane lock per phase;
artifacts-before-power-cuts; tollgate on VLAN200 during switch windows.

## Output

§12 of the known-good doc: the measured table (per mode: down/up best,
CPU load if observable, stability notes, switch cost), superseding the
community estimates, with the verdict re-affirmed or revised on data.
