<!-- OWNER-GATED DRAFT — comment for estkme-group/lpac issue #419 ("Segfault
     running lpac chip", qmi_qrtr, Pixel 3a/Mobian). Do NOT post without owner
     approval. Posting to estkme-group/lpac is human-gated EVERY round.
     Basis: teardown code-read 2026-10-26 at a43e46ae (= upstream main at
     read time), cross-checked against this issue's public transcript.
     No qmi_qrtr hardware was used — the chain below is a code-read match
     against the reporter's output; the maintainers/reporter can confirm with
     the one-line fix. See docs/evidence/lpac-segfault/FINDINGS.md §9. -->

We recently root-caused and filed a sibling uninitialized-variable crash in
this family — #467 (applet-layer `_cleanup_`/SBRM locals walked on ES10
error exits). While reading the teardown layer for that investigation we
root-caused **this** one too. Short version: **same meta-family
(uninitialized value walked on the error path), but a different layer and a
different variable — #467 does not fix this, and this is not the #467
crash.** The full chain at `a43e46ae`, matching your transcript line for
line:

1. `libapduinterface_init` (`driver/apdu/qmi_qrtr.c:82`) allocates
   `struct qmi_data` with plain **`malloc`** — no zeroing. `lastChannelId`
   starts as heap garbage (yours held `1662901524` = `0x63000054`, which
   looks like string residue — typical malloc filler).
2. `lastChannelId` is assigned **only on a successful open**
   (`driver/apdu/qmi_common.c:90`, after `get_result` AND
   `get_channel_id`). Your run failed earlier — QMI error 94
   'NotSupported' returns at `qmi_common.c:81-84` (your first stderr line)
   — so `lastChannelId` keeps its garbage.
3. `euicc_init`'s failure path **disconnects before returning**
   (`euicc/euicc.c`: `logic_channel_open` fails → `disconnect(ctx)`),
   which releases the UIM client and sets `qmi_priv->uimClient = NULL` /
   `qmi_priv->context = NULL` (`qmi_common.c:137-141`). Your
   `{"code":-1,"message":"euicc_init"}` JSON is `src/lpac.c:133`.
4. At exit, the driver fini (`lpac.c:225` → `qmi_qrtr.c:110`) calls
   `qmi_cleanup` (`qmi_common.c:144`): garbage `lastChannelId > 0` passes
   the `<= 0` guard → prints **"Cleaning up leaked APDU channel
   1662901524"** (your third line) → `logic_channel_close` calls into
   libqmi with the already-released `uimClient`/`context` → **SIGSEGV**
   (your fourth line).

This also explains the probabilistic shape you reported ("almost every
try, not 100 %"): whenever the malloc garbage happens to be `<= 0`,
`qmi_cleanup` returns early and nothing crashes.

Minimal fix (one line, mirrors the `gbinder_hidl.c:28` precedent of
`static int lastChannelId = -1;`):

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

(or `calloc` instead of `malloc`). Optional hardening: `qmi_cleanup` /
`logic_channel_close` could also treat a NULL/released `uimClient` as
"nothing to clean", so a close-after-disconnect can never reach libqmi.

Related observation for the maintainers: `driver/apdu/mbim.c` has the same
shape — `mbim_priv = malloc(sizeof(struct mbim_data))` (unzeroed) plus a
`last_channel_id`-driven "Cleaning up leaked APDU channel" print — worth
the same initialization treatment. (`gbinder_hidl.c` is safe: static
storage with an explicit `-1`.)

Happy to send the qmi (+ mbim) init patch as a PR if wanted. For
completeness: #467 covers the applet-layer crash that happens *after* a
successful `euicc_init` when an ES10 exchange ends in a card-error SW —
different layer, different fix; this one needs its own one-liner.
