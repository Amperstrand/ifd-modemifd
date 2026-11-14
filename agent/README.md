# modem-vpicc — vpcd agent for a modem SIM

Connects OUT to a vpcd listener (vsmartcard virtual reader, e.g. behind
`libifdvpcd-amperstrand.so` in a pcscd lane) and plays the smartcard
backed by a modem SIM. The modem side reuses the `ifd-modemifd` crate's
`Reader` — echo-correlated AT+CSIM, pacing floor, strict parse — so the
wire discipline is identical to the pcscd driver path. Architecture and
security requirements: `../tests/hardware/VPCD-ARCHITECTURE.md`.

## Usage

    modem-vpicc --vpcd HOST:PORT --target tcp://HOST:PORT|tty:/dev/ttyUSBx \
        [--atr-hex HEX] [--reconnect-ms N] [--log PATH]

The agent opens no listening sockets; default ATR is the driver's
FAKE_ATR for parity. `--reconnect-ms` floor is 100 ms (bounded
reconnect). Logging is opt-in, 0600, no-symlink, fail-closed (v3.4.1
doctrine).

## Host build

    cargo build --release -p modem-vpicc      # target/release/modem-vpicc

## OpenWrt cross-compile (mipsel, soft-float musl)

There is no prebuilt rust-std for `mipsel-unknown-linux-musl` on any
toolchain channel (tier-low), so `build-std` on nightly is forced. This
is a router artifact only — the pcscd `.so` stays on the fleet-pinned
stable toolchain.

    rustup toolchain install nightly --profile minimal
    rustup component add rust-src --toolchain nightly
    # musl.cc toolchain (x86_64 host): https://musl.cc/mipsel-linux-muslsf-cross.tgz
    tar xzf mipsel-linux-muslsf-cross.tgz -C <tools>
    CARGO_TARGET_DIR="$PWD/target" \
    CARGO_TARGET_MIPSEL_UNKNOWN_LINUX_MUSL_LINKER=<tools>/mipsel-linux-muslsf-cross/bin/mipsel-linux-muslsf-gcc \
      cargo +nightly build -Z build-std=std --release \
        --target mipsel-unknown-linux-musl -p modem-vpicc

The binary is dynamic against musl (`interpreter /lib/ld-musl-mipsel-sf.so.1`,
matching OpenWrt) and needs `libgcc_s.so.1` (OpenWrt base provides it;
verify on the target before one-shot nights).

### Routes that do NOT work (2026-10-24 ledger)

- `cargo-zigbuild` + zig 0.17: mipsel musl target renames — the mapped
  target string fails libc provisioning for both `-shared` and static.
- zig 0.14: bundles no mipsel musl at all (glibc variants only).
- zig 0.17 `cc -target mipsel-linux-musleabi` as a direct linker: its
  own `zigc.a` bridge has undefined internal symbols in link-only mode.
- `rustup target add mipsel-unknown-linux-musl` (stable or nightly): no
  prebuilt artifacts.

### Emulated smoke (proves the exact router artifact)

    # qemu root with real musl (the musl.cc sysroot's ld-musl symlink is
    # absolute to the host glibc — build a clean prefix):
    mkdir -p qemu-root/lib
    cp -L <tools>/mipsel-linux-muslsf-cross/mipsel-linux-muslsf/lib/libc.so qemu-root/lib/
    ln -sf libc.so qemu-root/lib/ld-musl-mipsel-sf.so.1
    cp -L <tools>/mipsel-linux-muslsf-cross/mipsel-linux-muslsf/lib/libgcc_s.so.1 qemu-root/lib/

    qemu-mipsel-static -L qemu-root target/mipsel-unknown-linux-musl/release/modem-vpicc ...

Verified 2026-10-24: ATR parity, APDU round-trip, OFF silence, RESET,
drop+reconnect — 5/5 against mock modem + mock vpcd.

## Router deployment (hardware-gated)

Run under procd next to the modem, wrapped in the per-tty flock and
coordinated through the router-side `modem-lane` lock (see
VPCD-ARCHITECTURE.md security section); `--target tty:/dev/ttyUSBx`.
The AT port shuffles across modem resets — re-detect before start.
