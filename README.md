# ifd-modemifd

pcscd IFD handler exposing a **cellular modem SIM slot** as a PC/SC smartcard
reader. Any PC/SC application (pySim, OpenSC, cardpeek, lpac's stock pcsc
backend, ...) can then talk to the card seated in a router's modem — no
external reader hardware.

Status: **private research prototype** (Amperstrand/conwrt-bench #21/#22).
A working C reference implementation lives in conwrt-bench/pcsc/modemifd.

## Architecture

    PC/SC app → pcscd → libifd_modemifd.so (this crate)
      → TCP → socat bridge on the OpenWrt router → modem AT port
      → AT+CSIM (basic + logical channels) → seated SIM

Rust `cdylib`, std-only, no external crates. The IFDHandler 3.0 C ABI is
implemented in `src/lib.rs` (constants mirrored from ifdhandler.h in
`src/consts.rs`). Panic-safe: every export is wrapped in `catch_unwind`.

## Design decisions (from prior art)

- Prefix-matched AT responses (`at_expect`): on a shared modem port, foreign
  URCs (e.g. `+CPIN: READY`) arrive interleaved; early-exit on any `OK` eats
  them (bug found in our first Rust version and in any naive CSIM loop).
  Same pattern lpac uses (`at_expect` with expected prefix).
- Pacing: RG502Q-EA firmware thrashes SIM sessions when AT commands arrive
  back-to-back — each command can trigger a `+CPIN: READY` URC and the real
  response lags or is swallowed. 250 ms inter-command floor + idle recovery.
- Fake ATR: modems cannot report the real ATR over AT commands; a sysmo-class
  T=0 ATR is substituted (documented limitation).
- Logical channels: raw MANAGE CHANNEL passes through AT+CSIM on proven
  firmwares (no CCHO/CGLA interception needed). Fallback mode is on the
  stabilization checklist.

## Known open issues (see conwrt-bench #22)

- **Response-lag aliasing under modem thrash**: when the firmware enters the
  URC-flood state, responses lag by one command; prefix matching alone cannot
  prove which exchange a `+CSIM:` line answers. Needs exchange correlation
  (command echo mode or CCHO-handle validation) and a recovery path
  (idle/CFUN cycle). Demonstrated: correct ICCID round-trip when the modem is
  calm; degradation under rapid-fire sessions.
- Serial transport (`/dev/ttyUSBx` direct) not yet implemented (TCP only).
- Multi-reader LUN registry present but untested beyond one reader.

## Build & install

    cargo build --release
    # target/release/libifd_modemifd.so → /usr/local/lib/
    # /etc/reader.conf.d/modemifd.conf:
    #   FRIENDLYNAME NR7101_SIM_bench
    #   DEVICENAME  192.168.13.124:7002     (socat bridge to the AT port)
    #   LIBPATH     /usr/local/lib/libifd_modemifd.so
    #   CHANNELID   1

Router side: see conwrt-bench/pcsc/modemifd/README (procd `modem-at-bridge`).

## License

LGPL-2.1-or-later (CCID-driver precedent).
