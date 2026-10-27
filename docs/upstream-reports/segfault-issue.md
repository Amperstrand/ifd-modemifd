<!-- OWNER-GATED DRAFT — new issue for estkme-group/lpac. Do NOT submit
     without owner approval. Filing is an owner decision.
     Attach when filing:
       - 0001-applet-zero-initialize-all-_cleanup_-decorated-local.patch
         (git format-patch, already generated in this directory; author is a
         local placeholder — amend author / add Signed-off-by before filing)
       - mock_modem.py (self-contained stdlib-only mock AT modem; the
         --case-sensitive mode is the reproducer fingerprint)
       - mock-transcript.jsonl (replay transcript)
       - segfault-backtrace-gdb.log (gdb backtrace — scrub the local build
           path in frame #1 to "libeuicc.so.2" before upload, or rely on the
           sanitized copy below)
       - card-error-path-exit255.json / card-error-path-exit255-fixverify.json
         (fixed behavior: clean error JSON, exit 255, no crash)
     Cross-link after filing both: #428 second-data-point comment ↔ this issue. -->

# lpac CLI segfaults after a card-error SW on an ES10 exchange (error-path teardown walks uninitialized `_cleanup_` locals)

## Summary

When an ES10 exchange ends in a **card-error SW after a successful ISd-R
select**, the lpac CLI prints the correct error JSON and then **SIGSEGVs**
during scope teardown. The crash is in the applet layer (post-error output,
cleanup-attribute teardown), not in the transport, so it is reachable from
any APDU backend — verified on `at_csim`; nothing transport-specific is
involved.

## Version / environment

- lpac: `a43e46ae` (master at time of writing, 2026-10)
- backend in the reproducer: `at_csim` over a pty (the only local change on
  the reproducer binary is an unrelated at_csim hex-case workaround needed to
  get through the test modem's CSIM case sensitivity; the crash itself is
  patch- and transport-independent — applet-layer teardown)
- x86_64 Linux, glibc

## Steps to reproduce

Make an ES10 exchange fail with a card-error SW after ISD-R was successfully
selected; two equivalent ways:

1. **Any card/fixture that answers a card-error SW to the STORE DATA of an
   ES10 command** (observed with `6D00` answering the getEID BER-TLV `BF3E`
   STORE DATA after a successful ISD-R SELECT), run e.g.:

   ```
   lpac chip info
   ```

2. The attached self-contained mock AT modem in `--case-sensitive` mode
   behind a pty bridge, `LPAC_APDU=at_csim LPAC_APDU_AT_DEVICE=<pty>` — it
   answers the ISD-R SELECT with a valid FCI + 9000 and `6D00` to STORE DATA
   (transcript attached):

   ```
   > AT                                        < OK
   > AT+CSIM=?                                 < OK
   > AT+CSIM=10,"0070000001"                   < +CSIM: 6,"019000" / OK
   > AT+CSIM=42,"01A4040010A0000005591010FFFFFFFF8900000100"
   < +CSIM: 16,"6F048402A0009000" / OK        (ISD-R SELECT: FCI, 9000)
   > AT+CSIM=22,"81E2910006BF3E035C015A"       (ES10 STORE DATA, getEID)
   < +CSIM: 4,"6D00" / OK                      (card-error SW)
   ```

Observed behavior — the error is reported correctly, then the process dies:

```
{"type":"lpa","payload":{"code":-1,"message":"es10c_get_eid","data":""}}
timeout: the monitored command dumped core          # SIGSEGV / exit 139
```

## Root cause (gdb)

```
Program received signal SIGSEGV, Segmentation fault.
#0  __GI___libc_free (mem=0x3) at ./malloc/malloc.c:3375
#1  es10c_ex_euiccinfo2_free () from libeuicc.so.2
#2  applet_main.lto_priv ()                      ← src/applet/chip/info.c
#3  applet_entry ()
#4  applet_main.lto_priv ()
#5  applet_entry ()
#6  lpac_main ()
#7  main ()
```

The `_cleanup_*_` macros expand to `__attribute__((cleanup(...)))`, which
runs at scope exit **no matter how the scope exits**. In
`src/applet/chip/info.c`, `euiccinfo2` (and `addresses`, `ratList`) are
declared **uninitialized**; the owning getters only initialize their
out-params once the ES10x exchange succeeds. When the earlier
`es10c_get_eid()` call fails, `applet_main` returns −1 right after printing
the error JSON, and the compiler-emitted cleanup for `euiccinfo2`
(`es10c_ex_euiccinfo2_free`) walks the uninitialized stack struct —
`free(0x3)` here. In general this is undefined behavior: crash or silent
corruption depending on prior stack contents.

The same declaration pattern exists in the notification and profile applets,
so the crash is not `chip info`-specific (table below).

## Affected declarations

11 uninitialized `_cleanup_`-decorated declaration sites (15 locals) across
7 applet files at `a43e46ae`:

| File | Declarations |
|---|---|
| `src/applet/chip/info.c` | `addresses`, `ratList`, `euiccinfo2` |
| `src/applet/notification/dump.c` | `notification` (retrieve_notification); `notifications`, `rptr` (applet_main) |
| `src/applet/notification/list.c` | `notifications`, `rptr` |
| `src/applet/notification/process.c` | `notification` (_process_single); `notifications`, `rptr` (applet_main) |
| `src/applet/notification/remove.c` | `notifications`, `rptr` |
| `src/applet/notification/replay.c` | `notification` |
| `src/applet/profile/list.c` | `profiles` |

Every row has the same failure shape: a fallible ES10x call that can return
an error **before** the owning getter initializes its out-param. E.g.
`lpac profile list` on a card whose `es10c_list_profiles` exchange ends in a
card-error SW walks the uninitialized `profiles` list pointer the same way.

(`profile/list.c`'s neighboring local `rptr` carries no cleanup attribute
and is correctly left untouched by the patch.)

## Proposed fix

Zero-initialize all affected declarations (structs `= {0}`, pointers
`= NULL`) so error-path cleanup walks zeros. Attached as git format-patch.

- **Success-path neutral by construction:** on every path where the getters
  run, they overwrite/own the structures exactly as before; the zero-init
  only gives defined values to the paths that previously read stack garbage.
- **Verified on the reproducer:** identical wire flow now terminates cleanly
  (exit 255) with the same error JSON and no crash (attached).

## Additional context

- Discovered while testing an at_csim uppercase-hex workaround for #428 —
  the reproducer mock fingerprints that Qualcomm-class lowercase-CSIM
  rejection and answers `6D00` to STORE DATA, which trips this crash.
- Practical impact for automation: the correct error JSON reaches stdout
  before the crash, but the process then dies with SIGSEGV (exit 139), so
  wrappers/CI classify it as a crash rather than an error result.
