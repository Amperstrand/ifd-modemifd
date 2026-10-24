# ifd-modemifd

pcscd IFD handler exposing a **cellular modem SIM slot** as a PC/SC smartcard
reader. Any PC/SC application (pySim, OpenSC, cardpeek, lpac's stock pcsc
backend, ...) can then talk to the card seated in a router's modem — no
external reader hardware.

## Status: production-proven (Rust). The C reference implementation is retired and removed.

This driver has carried a **complete real-world SGP.22 eSIM provisioning**:
a 9eSIM (ECP3 eUICC) seated in a Quectel RG502Q-EA modem was profile-
downloaded and enabled end-to-end through this driver (lpac → pcscd →
ifd-modemifd → modem AT+CSIM), single one-shot attempt, ~60 s wall clock,
zero transport errors. The 1 GB roaming data profile then registered and
passes live traffic through the modem.

Evidence base:

| Test | Result |
|---|---|
| pySim-shell full-card filesystem soak (operator USIM, ~60 EFs) | PASS, exit 0 |
| Sustained small-APDU load (SELECT MF ×300) | 300/300, p50 400 ms |
| BPP-install-shaped load (100-byte STORE DATA ×150 on logical channel) | 150/150, p50 400 / p99 401 ms |
| Logical-channel lifecycle (open → AID select → close) | clean, FCI answered |
| Real SGP.22 profile download + enable + notification | success, one shot |

## Architecture

    PC/SC app → pcscd → libifd_modemifd.so (this crate)
      → TCP → socat bridge on the OpenWrt router → modem AT port
      → AT+CSIM (basic + logical channels) → seated SIM

Rust `cdylib`, std-only, no external crates. The IFDHandler 3.0 C ABI is
implemented in `src/lib.rs` (constants mirrored from ifdhandler.h in
`src/consts.rs`). Panic-safe: every export is wrapped in `catch_unwind`.

## Modem findings baked into the driver (Quectel RG502Q-EA, Zyxel AACR firmware)

Measured with `tests/hardware/validate.sh` + the APDU shape probes — useful
to anyone driving this modem family as a smartcard reader:

- **Basic-channel filter**: the modem silently drops SELECT-by-AID/DF-name
  (A4 with P1=04) and READ BINARY (B0) on the basic channel (channel 0).
  The same APDUs on logical channels pass instantly. Silence shows up as a
  driver timeout returning SW 6F00.
- **No command-size ceiling**: well-formed STORE DATA and SELECT commands
  of 100+ bytes transport fine; command SIZE is not the BPP-load failure
  mode. Malformed APDUs (bad Lc) ARE silently dropped.
- **AT+CGLA responses are swallowed** on this firmware (CCHO/CCHC work);
  AT test commands (`AT+X=?`) answer bare `OK` with no range line —
  capability probes must not require the `+X:` prefix form.
- **CSIM pacing is mandatory**: back-to-back AT+CSIM bursts trigger SIM-
  session thrash (URC flood, lagging/swallowed responses). The driver
  enforces a **400 ms inter-command floor** (`TRANSMIT_GAP_MS` in
  `src/reader.rs`) between paced transmits; presence probes (`AT+CPIN?`)
  are deliberately exempt so pcscd's presence poller stays cheap. Under
  this floor, the v4.0 hardware validation measured **p50 400 ms
  round-trips** (SELECT MF ×300 and 100-byte STORE DATA ×150, zero
  failures) — the pacing gap dominates the modem's own response time.
- **The AT port shuffles across resets** (ttyUSB0-3): never hardcode it.
  The reference socat bridge init script auto-detects at start.

## Validation gate (run before any one-shot operation)

`tests/hardware/validate.sh` — phased gate: direct-AT channel probe (with
pcscd stopped for tty exclusivity), sustained small-APDU load, BPP-sized
load, optional ISD-R session loop. Exit 0 = green light. The gate encodes
every failure class that has actually burned a one-shot eSM activation
code, plus the card-handling doctrine (never hammer secure-channel-class
commands — one-shot authentication budgets on GP/eUICC cards are small and
non-renewable). See `tests/hardware/README.md`.

## Design decisions

- Prefix-matched AT responses (`at_expect`): on a shared modem port, foreign
  URCs (e.g. `+CPIN: READY`) arrive interleaved; early-exit on any `OK` eats
  them (bug found in the retired C implementation and in any naive CSIM
  loop). Same pattern lpac uses.
- Pacing: see CSIM pacing above.
- Fake ATR: modems cannot report the real ATR over AT commands; a sysmo-class
  T=0 ATR is substituted (documented limitation).
- Logical channels: raw MANAGE CHANNEL passes through AT+CSIM on proven
  firmwares (no CCHO/CGLA interception needed).

## Known limitations

- Response-lag aliasing under extreme modem thrash: when the firmware enters
  the URC-flood state, responses can lag by one command; prefix matching
  alone cannot prove which exchange a `+CSIM:` line answers. Recovery is
  idle/CFUN cycle. (Not observed under paced operation, including the full
  provisioning run.)
- Serial transport (`/dev/ttyUSBx` direct) not yet implemented (TCP only).
- Multi-reader LUN registry present but untested beyond one reader.

## Build & install

    cargo build --release
    # target/release/libifd_modemifd.so → /usr/local/lib/

    # /etc/reader.conf.d/modemifd.conf
    FRIENDLYNAME NR7101_SIM_dut
    DEVICENAME <router-ip>:7002        # socat: TCP-LISTEN:7002,reuseaddr,fork FILE:/dev/ttyUSBx,raw,echo=0,b115200
    LIBPATH /usr/local/lib/libifd-modemifd.so
    CHANNELID 1

Non-root PC/SC clients may need a polkit rule allowing `access_pcsc` /
`access_card` for their group (SSH sessions are not polkit-active seats).

## License

LGPL-2.1-or-later
