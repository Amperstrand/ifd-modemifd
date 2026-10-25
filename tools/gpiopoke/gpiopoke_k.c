// gpiopoke_k: dump MT7621 GPIO bank0 window; optional lte-power(18) rail cycle.
// mode=dump (default): read-only. mode=cycle: DCLR b18, hold, DSET b18.
#include <linux/module.h>
#include <linux/moduleparam.h>
#include <linux/io.h>
#include <linux/delay.h>
static char *mode = "dump";
module_param(mode, charp, 0444);
static int hold_ms = 30000;
module_param(hold_ms, int, 0444);
static int __init gp_init(void)
{
    void __iomem *b = ioremap(0x1e000600UL, 0x100);
    int i;
    if (!b) return -ENOMEM;
    for (i = 0; i < 0x50/4; i += 4)
        pr_info("gpiopoke: +%02x=%08x\n", i*4, ioread32(b + i*4));
    pr_info("gpiopoke: +0x50..+0x64: %08x %08x %08x %08x %08x %08x\n",
        ioread32(b+0x50), ioread32(b+0x54), ioread32(b+0x58),
        ioread32(b+0x5c), ioread32(b+0x60), ioread32(b+0x64));
    if (!strcmp(mode, "pwrkey")) {
        // Quectel force-off: press (drive low) >=10s -> release -> short press (2.5s) to boot
        pr_info("gpiopoke: pwrkey force-off press (b4 low 12s)\n");
        iowrite32(1u<<4, b+0x40);
        msleep(12000);
        iowrite32(1u<<4, b+0x30);
        pr_info("gpiopoke: pwrkey released; wait 3s then boot press 2.5s\n");
        msleep(3000);
        iowrite32(1u<<4, b+0x40);
        msleep(2500);
        iowrite32(1u<<4, b+0x30);
        pr_info("gpiopoke: pwrkey boot press done\n");
    }
    if (!strcmp(mode, "cycle")) {
        pr_info("gpiopoke: DATA(b18) pre=%d — CLR rail\n", (ioread32(b+0x20)>>18)&1);
        iowrite32(1u<<18, b+0x40);
        msleep(hold_ms);
        iowrite32(1u<<18, b+0x30);
        pr_info("gpiopoke: rail SET — DATA(b18) post=%d\n", (ioread32(b+0x20)>>18)&1);
    }
    iounmap(b);
    return -EINVAL;  // always unload after run
}
module_init(gp_init);
MODULE_LICENSE("GPL");
