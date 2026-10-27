# lpac #419 fix — A/B evidence archive (2026-10-27)

Supports draft PR estkme-group/lpac#469 (`fix/driver-lastchannel-init` @
`4922eb3`, on upstream `a43e46ae`). Full context: Amperstrand/lpac#6.

## Verdict matrix (files: `<tree>-<driver>.out`)

| tree | driver | rc | evidence |
|---|---|---|---|
| `pr419-old` (= stock `a43e46ae`) | qmi_qrtr | **139** | `Cleaning up leaked APDU channel 286331153` → SIGSEGV — the #419 field signature |
| `pr419-old` | mbim | 255 | clean (walk needs connect-success stage — see below) |
| `pr419-old` | qmi | 255 | clean — `qmi.c` uses `calloc` (control) |
| `pr419` (= fix `4922eb3`) | qmi_qrtr | 255 | clean error JSON, no crash |
| `pr419` | mbim | 255 | clean |
| `pr419` | qmi | 255 | clean |

## Reproduction recipe

1. **libqmi 1.38.0 local prefix** (Ubuntu ships 1.35.2 < lpac's ≥1.36 floor):

```sh
sudo apt install libqmi-glib-dev libmbim-glib-dev libglib2.0-dev libgudev-1.0-dev meson ninja-build
curl -sL -o libqmi.tar.gz https://gitlab.freedesktop.org/mobile-broadband/libqmi/-/archive/1.38.0/libqmi-1.38.0.tar.gz
tar xzf libqmi.tar.gz && cd libqmi-1.38.0
meson setup build-local --prefix=/tmp/opencode/libqmi-install -Dintrospection=false -Dman=false -Dgtk_doc=false
ninja -C build-local && meson install -C build-local
```

2. **Build both trees with every backend** (`PKG_CONFIG_PATH=/tmp/opencode/libqmi-install/lib/x86_64-linux-gnu/pkgconfig cmake -B build -DUSE_SYSTEM_DEPS=ON -DLPAC_WITH_APDU_AT=ON -DLPAC_WITH_APDU_QMI=ON -DLPAC_WITH_APDU_QMI_QRTR=ON -DLPAC_WITH_APDU_MBIM=ON`) — old at `a43e46ae`, new at the fix commit.

3. **Deterministic heap garbage** (`fill.c` here — glibc `MALLOC_PERTURB_` does
   NOT cover tcache/fresh chunks, which is why a plain env-var attempt fails):

```sh
gcc -shared -fPIC -o fill.so fill.c
env LD_PRELOAD=$PWD/fill.so \
    LD_LIBRARY_PATH=/tmp/opencode/libqmi-install/lib/x86_64-linux-gnu \
    LPAC_DRIVER_HOME=<tree>/build/driver LPAC_APDU=qmi_qrtr \
    timeout 25 <tree>/build/src/lpac chip info
```

Fill byte `0x11` → `lastChannelId = 0x11111111` (positive int — passes the
`<= 0` guard; note `0xA5` would be sign-negative and NOT trigger).

## Why mbim cannot crash host-side

mbim's walk lives in `mbim_apdu_interface_disconnect` — reachable only when
connect *succeeded* and the channel-open stage then failed (`euicc_init`'s
failure path calls disconnect). Without MBIM hardware, connect fails at
device creation → disconnect never runs. The `-1` init is provably
sufficient regardless: the `> 0` guard can never see garbage.

## Why qmi_qrtr DOES crash on the fini path (no device needed)

qmi_qrtr's driver `fini` → `qmi_cleanup` walks `lastChannelId`
unconditionally at exit — reached even when connect itself fails ("find
QRTR node with UIM service failed"), which is exactly the host-side shape
(and the #419 reporter's: transport up, UIM open failed → disconnect → fini
walk).
