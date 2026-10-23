# Hardware validation gate (pre-provisioning)

Before any **one-shot** operation through this reader (eSIM profile
download being the canonical case — a failed BPP install can burn an
activation code's download window server-side), run:

```sh
tests/hardware/validate.sh            # uses ROUTER_SSH/AT_TTY env overrides
```

Exit 0 = green light. The gate exercises the exact failure classes that
have actually killed a provisioning attempt:

| Phase | What it proves | Historical failure it guards against |
|---|---|---|
| 1. Direct-AT channel probe (`at_channel_probe.sh`, runs on the router) | `AT+CCHO`/`CGLA`/`CCHC` logical-channel path works and cycles cleanly | Quectel test commands answer bare `OK` (no `+CMD:` range line) — capability probes must not expect the range form |
| 2. PC/SC small-APDU load ×300 | steady-state transport + driver pacing hold up under sustained sessions | the original C spike died under load (response-lag aliasing, conwrt-bench #22) |
| 3. BPP-sized APDUs ×150 | command sizes and response parsing at profile-package scale (224-byte commands; card answers `6A82` by design) | first real BPP install failed at store command #1 through the unpaced C driver |
| 4. `lpac chip info` ×5 (optional) | full ISD-R logical-channel sessions end-to-end | — |

## Doctrine (borrowed from ccid-firmware-rs, paid for there)

1. **Never hammer secure-channel-class commands.** A transport-fidelity
   hammer once sent ~450 INITIALIZE UPDATE commands against a GP card and
   exhausted a small NON-RENEWABLE error budget — permanent loss of secure
   channel on that card (ccid-firmware-rs AGENTS.md, 2026-10-12). Every
   probe in this gate is non-authenticating: SELECTs, a public-AID channel
   open, filesystem reads. Authentication (`STORE DATA`, server
   challenges) happens exactly once — in the real download.
2. **Exclusive access per phase.** pcscd's presence poller writes
   `AT+CPIN?` to the modem tty every few seconds through the shared socat
   bridge. Any direct-AT probe run while pcscd is live will capture
   interleaved traffic and produce false failures. `validate.sh` stops
   pcscd for phase 1 and restarts it after — keep that ordering.
3. **One shot per activation code.** If the gate is green and the download
   still fails, STOP. Diagnose before retrying: each cancel can close the
   SM-DP+ download window (observed: "Time to Live Expired" on every
   subsequent retry of the same matching ID).
4. **Re-detect the AT port after any modem power cycle.** The RG502Q AT
   port shuffles across ttyUSB0-3 between boots; `AT_TTY` defaults are
   only valid until the next power cycle.

## Files

- `at_channel_probe.sh` — POSIX sh, staged to the router by validate.sh
- `validate.sh` — host orchestrator (phases, verdict, exit code)
- `examples/loadtest.rs` — PC/SC load tester (`pcsc` crate as dev-dep);
  also usable standalone: `cargo run --release --example loadtest -- --count 500 --mode mixed`
- `examples/direct.rs` — driver-core test bypassing pcscd entirely
  (takes `<ip:port>` argument)
