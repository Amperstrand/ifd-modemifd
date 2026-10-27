<!-- OWNER-GATED DRAFT v2 (recalibrated 2026-10-27 per owner direction:
     strict epistemics — what we did / what we tested / what we think, no
     unverified claims). Comment for estkme-group/lpac issue #419.
     Do NOT post without owner approval. Suggested timing: after #467 or
     #468 merges (single coherent arc, references merged work). -->

We don't have qmi_qrtr hardware, so this is a **code-read account, not a
repro** — but while working on two sibling uninitialized-variable crashes in
this codebase (the applet-layer one in #467, the at_csim leak close in
#468) we read the qmi teardown path, and **every line of your transcript
maps to a specific code site at current main**, forming one consistent
chain. Offering it as a candidate root cause for the 8-month-old symptom,
not as a confirmed fix.

**What we did:** read `euicc/euicc.c`, `src/lpac.c`,
`driver/apdu/qmi_qrtr.c`, `driver/apdu/qmi_common.c` at `a43e46ae`
(upstream main at the time); the relevant lines were cross-checked by a
second reader independently. **What we did not do:** run anything on
qmi_qrtr hardware, or capture a backtrace of your crash — the segfault
frame below is *inferred from control flow*, not observed.

**The transcript-to-code mapping** (all four of your lines, in order):

| your output | code site |
|---|---|
| `error: open logical channel operation failed: QMI protocol error (94)` | `qmi_common.c:82`, the `get_result` failure return |
| `{"code":-1,"message":"euicc_init",...}` | `src/lpac.c:133` |
| `Cleaning up leaked APDU channel 1662901524` | `qmi_common.c:147`, inside `qmi_cleanup` |
| `Segmentation fault` | the statement immediately after the print: `qmi_apdu_interface_logic_channel_close()` → libqmi |

**The proposed chain** (why those lines connect):

1. `qmi_qrtr.c:82` allocates `struct qmi_data` with plain `malloc`, no
   zeroing — `lastChannelId` starts as heap garbage (yours held `1662901524`
   = `0x63000054`, plausible malloc filler).
2. `lastChannelId` is assigned only on a *successful* open
   (`qmi_common.c:90`, past both `get_result` and `get_channel_id`). Your
   error-94 return at `:81-84` precedes it — the garbage survives.
3. `euicc_init`'s failure path calls `disconnect` before returning
   (`euicc/euicc.c`), which releases the UIM client and sets
   `qmi_priv->uimClient = NULL` / `qmi_priv->context = NULL`
   (`qmi_common.c:137-141`).
4. At exit, the driver fini (`src/lpac.c` → `qmi_qrtr.c:110`) calls
   `qmi_cleanup`: garbage `> 0` passes the `<= 0` guard → your print → the
   close call runs into libqmi with the already-released client → SIGSEGV.
   The print and the crash are adjacent in your transcript, and the close
   call is the only statement between the print and process exit.

This would also explain the probabilistic shape you reported ("almost
every try, not 100 %"): whenever the malloc garbage happens to be `<= 0`,
`qmi_cleanup` returns before printing and nothing crashes.

**A one-line hardening that is safe regardless of whether this chain is
the actual crash cause** — the printed garbage value on its own proves
`lastChannelId` is read uninitialized, and initializing it costs nothing
(same sentinel the code already uses after a successful close, and the
same pattern `gbinder_hidl.c` uses statically):

```diff
--- a/driver/apdu/qmi_qrtr.c
+++ b/driver/apdu/qmi_qrtr.c
@@ libapduinterface_init(struct euicc_apdu_interface *ifstruct)
     qmi_priv = malloc(sizeof(struct qmi_data));
     if (!qmi_priv) {
         fprintf(stderr, "Failed allocating memory\n");
         return -1;
     }
+    qmi_priv->lastChannelId = -1;
```

(Alternatively `calloc`.) Related, same shape: `driver/apdu/mbim.c` also
mallocs its private struct unzeroed and has a `last_channel_id`-driven
leaked-channel print — likely worth the same treatment.

**Family context, for weight not proof:** we validated this
uninit-on-error-teardown *family* empirically on the transports we do have —
the at_csim member (leaked logical channels on aborted sessions, A/B tested
against a scripted mock, #468) and the applet-layer member (deterministic
under ASan + `-ftrivial-auto-var-init=pattern`, #467). The qmi member above
is the one we can only read, not run — if anyone with a Pixel-class device
can reproduce, a backtrace would confirm or kill the chain in one step.

Happy to send the qmi (+ mbim) init patch as a PR if wanted.
