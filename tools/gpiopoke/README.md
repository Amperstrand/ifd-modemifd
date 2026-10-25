# gpiopoke — MT7621 GPIO register instrument (NR7101 modem power/reset lines)

Kernel module reaching the modem's control lines when every userspace path
is closed (DTS hogs block sysfs export; /dev/mem compiled out; driver
unbind leaves no chip). Built and exercised live during the 2026-10-25
modem-wedge incident; register map confirmed against debugfs hog states
before any write (dump-first discipline).

## Register map (bank0, base 0x1e000600, verified live 2026-10-25)
| offset | register | note |
|---|---|---|
| +0x00 | DIR | bit set = output |
| +0x20 | DATA | readable level |
| +0x30 | SET | write-1-to-set (reads 0xffffffff) |
| +0x40 | CLR | write-1-to-clear (reads 0xffffffff) |

Lines (NR7101 DTS, sysfs base 512): **bit18 = `lte-power` rail** (sysfs
gpio530), **bit4 = `lte-pwrkey`** (sysfs gpio516); the modem RESET_N line
lives on gpio3/sysfs gpio515 (uci `system.lte_reset`, not hogged — sysfs
works for it directly).

## Usage
```
insmod gpiopoke_k.ko                 # dump-only: prints the window, self-unloads
insmod gpiopoke_k.ko mode=cycle      # DCLR bit18, 30s, DSET (load-switch rail cut)
insmod gpiopoke_k.ko mode=pwrkey     # 12s force-off press + boot press (Quectel semantics)
```
Build: OpenWrt SDK kernel tree (`make -C $KDIR ARCH=mips CROSS_COMPILE=… M=$PWD modules`).
The module always returns -EINVAL from init so it never stays loaded.

## Incident context (2026-10-25)
Both modes executed bit-level-verified against a CFUN-wedged RG502Q-EA —
neither revived it (the wedge lived below the load-switch rail; the
20-second physical cut, which also drops VBUS, did). Recorded here because
the instrument itself is sound and is this hardware family's last software
rung before physical recovery.
