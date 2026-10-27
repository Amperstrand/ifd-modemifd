# FINDINGS — lpac CLI post-error SIGSEGV (ES10 card-error SW)

**Task:** root-cause the lpac post-error segfault (Amperstrand/lpac#1, upstream a43e46ae).
**Status:** root cause CONFIRMED, fix reconstructed (d68be3b equivalent), verified on 3 builds.
**Bug is live at upstream HEAD today:** `origin/main` of estkme-group/lpac **is** a43e46a
(0 commits after) — every current build of lpac carries this.
**Constraint compliance:** offline dig; zero GitHub writes (all `gh` calls were `view`/`search`
reads); no hardware/router/SSH; every lpac run wrapped in `timeout(1)`; no `pkill -f`.

---

## 1. Root cause (one sentence)

The `src/applet/*` CLI layer declares **11 `_cleanup_`-attributed locals without
initializers across 8 files**; when an ES10/ES10b call fails and the applet returns
early, GCC's cleanup attribute runs teardown functions (`es10c_ex_euiccinfo2_free`,
`es10b_rat_list_free_all`, …) on **uninitialized stack garbage**, and the first
`free()` inside them (`free(euiccinfo2->profileVersion)`, `euicc/es10c_ex.c:269`)
dereferences a wild pointer.

This is exactly hypothesis 1 from the issue queue (the private Gate-2 commit
`d68be3b` "zero-init `_cleanup_` locals — fixes post-error teardown segfault").
Hypotheses 2 (double-free/UAF of euicc_ctx), 3 (cJSON partial teardown) and
4 (driver fini on inconsistent userdata) are all **ruled out** by the backtrace:
the faulting frame is the *cleanup attribute itself* at declaration site
(`src/applet/chip/info.c:19`), before any euicc_fini/cJSON teardown runs.

## 2. Reproduction (pure upstream, no private patches)

Clone at a43e46ae (`/tmp/opencode/segfault-dig/lpac`), x86. Because stock lpac
emits lowercase CSIM hex, run the mock **without** `--case-sensitive` (the mock's
default accepts lowercase; STORE DATA still falls through to `6D00` INS-not-supported).
This reproduces on untouched upstream — stronger for the upstream report than the
archived UPPER variant, which needed the private uppercase patch.

```sh
python3 ifd-modemifd/tests/hardware/mock_modem.py --listen 127.0.0.1:17029 --log mock.jsonl &
socat PTY,raw,echo=0,link=/tmp/at0 TCP:127.0.0.1:17029 &
timeout 30 env LPAC_APDU=at_csim LPAC_APDU_AT_DEVICE=/tmp/at0 ./lpac chip info
# -> {"type":"lpa","payload":{"code":-1,"message":"es10c_get_eid","data":""}}
# -> Segmentation fault (exit 139)
```

Exchange seen by the mock (byte-equivalent to archived `mock-UPPER.dbg`, modulo hex case):
MANAGE CHANNEL open `→ 019000`, ISD-R SELECT `→ 6F048402A0009000` (transport healthy),
STORE DATA (getEID) `→ 6D00`, channel close `→ 9000` — then the crash.

Three independent pre-fix evidence lines:

**a) Plain build (natural stack garbage): exit 139** after the correct error JSON —
reproduces the archived transcripts (`plain-pre.out` + shell `Segmentation fault`).

**b) gdb on the plain build — natural-shape backtrace** (`gdb-plain.log`; identical to
the prior session's lost `segfix/gdb3.log` capture, including the magic garbage value):

```
Program received signal SIGSEGV, Segmentation fault.
0x00007ffff7cade55 in __GI___libc_free (mem=0x3) at ./malloc/malloc.c:3375
#0  __GI___libc_free (mem=0x3)
#1  es10c_ex_euiccinfo2_free (euiccinfo2=0x7fffffffdc60) at euicc/es10c_ex.c:269
#2  applet_main (...) at src/applet/chip/info.c:19        <-- the _cleanup_ attribute
#3  applet_entry (...) at src/applet.c:34
#4  applet_main (...) at src/applet/chip.c:24
#5  applet_entry (...) at src/applet.c:34
#6  lpac_main (...) at src/lpac.c:221
#7  main (...) at src/main.c:3
rdi 0x3    ; es10c_ex.c:269 = free(euiccinfo2->profileVersion)
```

`info.c:19` is the declaration `_cleanup_(es10c_ex_euiccinfo2_free) struct
es10c_ex_euiccinfo2 euiccinfo2;` — the crash fires when the early `return -1`
(after `es10c_get_eid` fails) triggers the cleanup attribute on a never-written
struct. `profileVersion` held residual stack value `0x3`.

**c) ASan + `-ftrivial-auto-var-init=pattern` build — deterministic backtrace**
(`pattern-pre.err`; stock ASan alone does NOT reproduce — ASan's different stack
layout happened to leave NULL in the slot, clean exit: that layout-luck is itself
proof the variable is uninitialized). Pattern-init poisons every uninit local to
0xAA…, making the wild `free()` fire every run, inside ASan:

```
==ERROR: AddressSanitizer: SEGV (free of non-heap pointer, inside ASan allocator)
    #3 free
    #4 es10c_ex_euiccinfo2_free /tmp/.../euicc/es10c_ex.c:269
    #5 applet_main /tmp/.../src/applet/chip/info.c:19
    #6 applet_entry src/applet.c:34  → chip.c:24 → lpac.c:221 → main
```

Why the error path is reachable at all: `es10c_get_eid` → `es10x_transmit` →
STORE DATA answered with card-error SW (`6D00`) → `libapduinterface_logic_channel`
teardown closes the channel fine (the `9000` close in the mock log) — only the
*applet-level* cleanup is broken. Any ES10/ES10b call that fails after
`euicc_init` succeeds walks the same path (`chip info`, `profile list`,
`notification list/process/remove/dump/replay`).

## 3. Full defect inventory (11 declarations, 8 files, all at a43e46ae)

| File:line | Declaration | Cleanup handler runs on garbage |
|---|---|---|
| `src/applet/chip/info.c:17` | `struct es10a_euicc_configured_addresses addresses;` | `free(defaultDpAddress)`, `free(rootDsAddress)` |
| `src/applet/chip/info.c:18` | `struct es10b_rat *ratList;` | `es10b_rat_list_free_all` walks wild list |
| `src/applet/chip/info.c:19` | `struct es10c_ex_euiccinfo2 euiccinfo2;` | 14× `free(field)` + 2 pointer-array loops |
| `src/applet/profile/list.c:14` | `struct es10c_profile_info_list *profiles;` | list walk + 4× `free` per node |
| `src/applet/notification/list.c:14` | `*notifications, *rptr;` (2 vars) | list walk ×2 |
| `src/applet/notification/process.c:18` | `struct es10b_pending_notification notification;` | 2× `free` |
| `src/applet/notification/process.c:86` | `*notifications, *rptr;` (2 vars) | list walk ×2 |
| `src/applet/notification/remove.c:60` | `*notifications, *rptr;` (2 vars) | list walk ×2 |
| `src/applet/notification/dump.c:16` | `struct es10b_pending_notification notification;` | 2× `free` |
| `src/applet/notification/dump.c:63` | `*notifications, *rptr;` (2 vars) | list walk ×2 |
| `src/applet/notification/replay.c:95` | `struct es10b_pending_notification notification;` | 2× `free` |

All five cleanup handlers are verified NULL/zero-safe (`while(ptr)` loops,
`free(NULL)` no-ops, guarded inner loops), so zero-initializing is both
sufficient and behavior-preserving on success paths.

## 4. Minimal fix (reconstruction of d68be3b)

`d68be3b` was not found in any local tree (the Gate-2 build sandbox is gone;
`/tmp/opencode/segfix/fixed-run.out` is its only surviving artifact). Reconstructed
from the defect inventory — mechanical zero-init, no success-path change.
Full diff in `fix.patch`; shape:

```diff
-    _cleanup_(es10a_euicc_configured_addresses_free) struct es10a_euicc_configured_addresses addresses;
-    _cleanup_es10b_rat_list_ struct es10b_rat *ratList;
-    _cleanup_(es10c_ex_euiccinfo2_free) struct es10c_ex_euiccinfo2 euiccinfo2;
+    _cleanup_(es10a_euicc_configured_addresses_free) struct es10a_euicc_configured_addresses addresses = {0};
+    _cleanup_es10b_rat_list_ struct es10b_rat *ratList = NULL;
+    _cleanup_(es10c_ex_euiccinfo2_free) struct es10c_ex_euiccinfo2 euiccinfo2 = {0};
```
…plus `= NULL` / `= {0}` on the other 8 declarations (see fix.patch).

## 5. Verification (post-fix, same reproducer)

| Build | Pre-fix | Post-fix |
|---|---|---|
| plain (`-g -O0` Debug) | exit **139** (SIGSEGV after error JSON) | exit **255**, `{"type":"lpa","payload":{"code":-1,"message":"es10c_get_eid","data":""}}`, no crash |
| ASan | exit 255 (layout luck — clean) | exit 255, no ASan report |
| ASan + `-ftrivial-auto-var-init=pattern` | **deterministic ASan SEGV** (evidence c) | exit **255**, no report — poisoned locals safely free(NULL)-walked |

- Post-fix stdout is byte-identical to the prior session's surviving
  `segfix/fixed-run.out` (d68be3b build) — corroborates the reconstruction.
- Adjacent regression: project test suite on the fixed tree — `ctest` **3/3 passed**
  (lpac-utils-tests, lpac-tests, libeuicc-tests).
- Artifacts: `plain-{pre,post}.{out,err}`, `pattern-{pre,post}.{out,err}`,
  `asan-post.out`, `gdb-plain.log`, `*-mock.jsonl`, `fix.patch`, `run-repro.sh`,
  `gdb-repro.sh` — all in `/tmp/opencode/segfault-dig/`.

## 6. Archaeology (read-only) — upstream issue landscape

- **estkme-group/lpac#419 (OPEN, 2026-02-27)** — "Segfault running `lpac chip`",
  qmi_qrtr backend, `euicc_init` error JSON printed then SIGSEGV, *probabilistic*
  ("almost every try, not 100%"), and it even logs `Cleaning up leaked APDU
  channel 1662901524` — a garbage channel number, itself an uninitialized-variable
  tell. ~~Same bug family as ours~~ **RESOLVED 2026-10-26 (see §9): related
  meta-family but a DISTINCT bug — driver-layer, NOT fixed by the applet
  zero-init.**
- **#406 (CLOSED)** — coredump on `profile discovery -i IMEI`: **not** our family;
  root-caused separately as a buffer overflow in `euicc_hexutil_gsmbcd2bin` with
  15-digit IMEIs, fixed by #415 (`80c8268`, "fix BOF … reached when setting IMEI").
- **#174 (CLOSED)** — old segfault in `lpac driver http` on QMI; predates the
  current driver architecture; no action.
- Precedent for uninit-var fixes upstream: `15f9d60` "derutil: fix uninitialized var"
  — the maintainers accept this fix class; a mechanical zero-init PR fits house style.
- **Upstream teardown-path history:** `euicc/euicc.c:euicc_fini` and the
  `*_free`/`*_free_all` handlers were last touched in the #311-era refactors; the
  applet `_cleanup_` declarations date to the applet framework introduction and
  were never initialized in any commit since (verified: no commit between then and
  a43e46ae touches these declarations).
- **Side question — `driver/apdu/at_etsi_csim.c:97` `aid_hex` computed, never
  used:** it was **born dead** in the #311 full rewrite (`1ec09a2`, 2025-10-12):
  the commit adds exactly the two lines (malloc + `bin2hex`) and zero uses. The
  rewrite's predecessor (`driver/apdu/at_unix.c`, pre-#311) opened channels via
  `AT+CCHO="<hex-aid>"` — the hex AID was load-bearing there. #311 switched
  `at_etsi_csim.c` to raw MANAGE CHANNEL + SELECT-by-AID over CSIM (binary AID
  copied into the APDU at :123) and left the hex conversion behind.
  **Verdict: harmless refactor remnant, not an unfinished path** — a 2-line
  deletion candidate to fold into any future driver cleanup PR. Not load-bearing
  for this bug; do not report it alone.

## 7. Upstream-report recommendation (GATED — nothing was posted)

Warranted: bug confirmed at upstream HEAD with a hardware-free reproducer, a
root cause, and a one-line-per-declaration fix. The issue draft in
Amperstrand/lpac#1 should be improved as follows before any owner-GO'd posting
(deltas only — keep the gated draft's framing):

1. Drop the `LPAC_APDU_AT_HEX=UPPER` requirement — run the mock **without**
   `--case-sensitive` so the reproducer works on stock upstream (no fork patches,
   no env var that upstream doesn't have).
2. State the root cause up front (maintainers value diagnosis): applet-layer
   `_cleanup_` locals uninitialized; cleanup attribute frees garbage on any ES10
   error exit; attach the gdb backtrace (`free(mem=0x3)` ← `es10c_ex.c:269` ←
   `info.c:19` cleanup attribute) and note the 11-declaration inventory across
   `chip/info.c`, `profile/list.c`, `notification/{list,process,remove,dump,replay}.c`.
3. Note generality: reachable on **any** APDU backend whenever an ES10 exchange
   ends in card-error SW after `euicc_init` succeeds — and that it explains the
   probabilistic shape of open issue #419 (suggest maintainers check whether #419
   is the same fix; do not claim it outright).
4. Propose the mechanical fix (`= {0}` / `= NULL`) and offer the ASan+
   `-ftrivial-auto-var-init=pattern` technique as the deterministic reproducer
   (stock ASan can miss it due to stack-layout luck — worth stating so maintainers
   don't dismiss a non-reproducing ASan build).
5. Do **not** fold in the `aid_hex` dead code (noise; separate trivial cleanup).

## 8. Provenance / hygiene

- Dig tree: `/tmp/opencode/segfault-dig/lpac` (clone of estkme-group/lpac @ a43e46ae,
  fix applied in working tree, uncommitted — see `fix.patch`).
- d68be3b search: no local tree under `/tmp/opencode` contains it (lpac-stock /
  lpac-upper are at a43e46a with only the uppercase+mbim-timeout diffs); the
  Gate-2 branch sandbox no longer exists. Reconstruction cross-checked against
  `docs/INDEPENDENT-REVIEW-9ESIM-20261025.md` ("11 across 8 applet files",
  gdb `free(mem=0x3)` in `es10c_ex_euiccinfo2_free`) and byte-matched against
  `segfix/fixed-run.out`.
- All mock/socat processes reaped by PID; workspace `git status` shows no
  artifacts from this dig (early-cwd strays moved to the dig dir).

## 9. #419 teardown read (2026-10-26, this lane) — DISTINCT root cause

Follow-up: the teardown code-read §6 deferred ("not confirmed identical").
Verdict: **#419 is NOT the applet bug and is NOT fixed by the zero-init PR
(#467)** — different layer, different variable, different fix. Chain verified
by code-read at `a43e46ae` against #419's public transcript (line-for-line
match):

1. `qmi_qrtr.c:82` — `qmi_priv = malloc(sizeof(struct qmi_data))`, **unzeroed**;
   `lastChannelId` = heap garbage (reporter's `1662901524` = `0x63000054`).
2. `qmi_common.c:90` — `lastChannelId` assigned **only on successful open**;
   the QMI-error-94 path returns at `:81-84` before it → garbage survives.
3. `euicc.c` `euicc_init` failure path **disconnects** (open fails →
   `disconnect(ctx)`): `qmi_common.c:137-141` NULLs `uimClient`/`context`.
4. `lpac.c:225` `euicc_driver_fini` → `qmi_qrtr.c:110` → `qmi_cleanup`
   (`qmi_common.c:144`): garbage `> 0` passes the `<= 0` guard → prints the
   "Cleaning up leaked APDU channel" line → `logic_channel_close` calls libqmi
   with the **released/NULL client+context** → SIGSEGV.
   Probabilistic shape = malloc garbage ≤ 0 on some runs → early return.

Minimal fix: `qmi_priv->lastChannelId = -1;` after the malloc (mirrors
`gbinder_hidl.c:28` static `-1` precedent) or `calloc`. `mbim.c:317` has the
same unzeroed-alloc + `last_channel_id` pattern (`:287` print) — same
treatment warranted; gbinder is safe.

Owner-gated cross-ref draft written: `issue-419-crossref-comment.md`
(mirrored in `docs/upstream-reports/`). Filing is owner-GO only; nothing
posted. This also resolves the esim-research duplicate-filing question: any
"invalid write to NULL after ES10 JSON" crash is the **#467** family
(post-init, applet layer); a crash after the **`euicc_init`** JSON with a
garbage leaked-channel line is **this** bug (driver layer) — the printed
error JSON names the layer.

**Addendum 2026-10-26 (post-draft):** chain independently verified by the
esim-research lane in their tree (same qmi lines; mbim same pattern — their
alloc pins ~L318 on bench-tree drift, ours is 317 @ a43e46ae). Both one-line
`-1` inits folded fork-local on `amperstrand/bench` @ `e1964d2` — mbim
compile-verified via mipsel SDK cross-build; **qmi_qrtr compile-UNverified**
(no libqmi-full toolchain anywhere on the bench — basic-collection feed SDK
compiles UIM APIs out); closure path is upstream CI at PR time (upstream
builds the qmi_qrtr driver). Upstreaming agreement (esim-research): the init
fix folds into/behind the gated #419 cross-ref draft, never a separate
filing. Tracker capture: Amperstrand/lpac issue (this family) + the at_csim
leak fix (esim-research's) were the two untracked follow-ups at session
close; ops#29 is the ops-level umbrella.
