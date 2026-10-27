<!-- OWNER-GATED DRAFT — comment body for estkme-group/lpac issue #428.
     Do NOT post without owner approval. Filing is an owner decision.
     Attach when filing: r1-stock-atdebug.log, r1-upper-atdebug.log,
     mock-transcript.jsonl (rehearsal/mock captures; live results are quoted
     in prose below — owner may additionally attach the live wire capture). -->

Second Qualcomm-class data point for this: **a Quectel RG502Q-EA (Qualcomm-based)
modem** (inside a Zyxel NR7101 router), live-confirmed 2026-10-25 against stock
lpac `a43e46ae` with `LPAC_APDU=at_csim`:

- **Stock:** the connect probes (`AT`, `AT+CSIM=?`) pass, the MANAGE CHANNEL
  open passes (digits-only PDU — case-irrelevant), and the **first ISD-R
  SELECT — the first PDU containing hex letters — is refused with a bare
  `ERROR`**, so `apdu_interface_transmit` fails and the CLI dies in
  `euicc_init`. Exactly the signature reported here for the T99W696.
- **Same source tree + only the one-line `euicc/hexutil.c` change**
  (`"0123456789abcdef"` → `"0123456789ABCDEF"`): the same SELECT gets
  `+CSIM: 16,"6F048402A0009000"` (FCI + SW 9000) and the session proceeds to
  a **fully working LPA session over AT+CSIM** — `lpac chip info` completes,
  including the EID, exit 0 — on the **same modem and same card**, nothing
  else changed.

Wire shape (rehearsal replay against a mock modem fingerprinted to this
rejection class — captures attached; channel byte differs between the two
capture runs, which is irrelevant to the case point):

| exchange | stock build | uppercase build |
|---|---|---|
| `AT+CSIM=10,"0070000001"` (MANAGE CHANNEL) | `+CSIM: 6,"019000"` + `OK` | `+CSIM: 6,"029000"` + `OK` |
| `AT+CSIM=42,"01a4040010a0000005591010ffffffff8900000100"` (ISD-R SELECT) | **`ERROR`** → `euicc_init` failure | — |
| `AT+CSIM=42,"02A4040010A0000005591010FFFFFFFF8900000100"` (same, uppercase build) | — | `+CSIM: 16,"6F048402A0009000"` + `OK` |

Third independent witness, with an exact public transcript: osmocom **pySim
gerrit 8e15c18** ("transport/AT: Make sure PDU has upper case hex digits",
tested on a Sierra EM7565): `AT+CSIM=14,"00a40004023F00"` → `ERROR`; the same
PDU uppercased → `+CSIM: 4,"612F"` + `OK`. pySim shipped the uppercase fix
years ago.

So the pattern now spans three vendors — Sierra EM7565, Foxconn T99W696 (this
issue), Quectel RG502Q-EA — with byte-identical failure shapes: bare `ERROR`
for letter-bearing lowercase PDUs, success after uppercasing, digits-only
PDUs unaffected (which is why `euicc_init`-adjacent probes can look healthy
right up to the first SELECT-by-AID).

The fix is the one line already linked in this thread (SlarkXiao's fork),
still unmerged — `euicc/hexutil.c` lowercase digits table at `a43e46ae` and
at `main` as of 2026-10-25:

```diff
--- a/euicc/hexutil.c
+++ b/euicc/hexutil.c
@@ int euicc_hexutil_bin2hex(char *restrict output, const uint32_t output_le
     if (output == NULL || bin == NULL || output_len < ((2 * bin_len) + 1)) {
         return -1;
     }
-    static const char digits[] = "0123456789abcdef";
+    static const char digits[] = "0123456789ABCDEF";
```

Two notes in favor of fixing the default (rather than only adding the
`LPAC_APDU_AT_HEX=UPPER` toggle suggested upthread):

1. **RX is case-insensitive either way** — `euicc_hexutil_hex2bin` case-folds
   when parsing, so an uppercase TX table cannot break decoding of modem
   responses (which already come back uppercase on these modems).
2. TS 27.007 does not mandate a case; the field evidence says uppercase is
   the interoperable choice — pySim reached the same conclusion. Lowercase
   buys nothing except rejection on this modem class.

Happy to send this as a PR, the env toggle, or both, if useful. For anyone
hitting `euicc_init` failures on Qualcomm-based modules over CSIM: this
one-line change is worth trying before abandoning the AT path — at_csim then
works cleanly end-to-end on this modem+card.

**Attachments** (offline rehearsal wire captures against the case-sensitive
mock; live results quoted above):

- `r1-stock-atdebug.log` — stock `a43e46ae`, full TX/RX trace ending in the
  bare `ERROR` at the lowercase ISD-R SELECT
- `r1-upper-atdebug.log` — uppercase build, same trace shape, FCI answered
- `mock-transcript.jsonl` — replay transcript of the same fingerprint

Cross-ref: the rehearsal capture also trips an unrelated CLI crash right
after a card-error SW in the following ES10 exchange — being filed as a
separate issue.
