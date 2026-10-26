# Mode sweep — measured results (2026-10-26, clean slate 25.12.6)
Instrument: pinned-route cachefly 10MB bounded passes from bench (LAN client,
via NR7101). Minimal-data discipline per owner (nadanada, roaming). Each
state's number = best of bounded passes; stability = N/5 consecutive fetch gate.

| Mode | Stability gate | Throughput (down) | Notes |
|---|---|---|---|
| ECM (usbnet=1) | **5/5** | **~3 Mbps** | stock kmod needed (cdc-ether absent from image); both offload states identical |
| QMI raw-IP (usbnet=0) | **5/5** | **~9.4 Mbps** | 3.1x ECM; qmi_wwan+uqmi+proto-qmi ALL stock on image; HC unbind/rebind needed once post-switch (SIM illegal-state recovery — pJ's rung 2, worked) |
| MBIM (usbnet=2) | — | — | BLOCKED: no cdc-mbim kmod in feed (checked, absent); mbim-utils present but proto-mbim absent (no umbim in feed) |
| vendor QMAP | — | — | staged (SDK build pJ); not yet run |

Data consumed: ~35 MB total (gate + 2 throughput passes + boot probes).
Speedtest Travel eSIM: carrier-policy blackhole (one-grace-flow-per-context;
genuine Ookla protocol also blocked) — unusable for measurement. All sweep
traffic rode nadanada per owner's minimal-data directive.
