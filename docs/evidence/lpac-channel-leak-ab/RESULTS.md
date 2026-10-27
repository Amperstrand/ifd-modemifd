# Channel-leak A/B — stock a43e46ae vs close-on-abort c004218 (mock, 2026-10-27)

Dogfood validation of `fix/at-csim-close-on-init-failure` (upstream PR
estkme-group/lpac#468, tracked Amperstrand/lpac#5). Mock-only run, zero
hardware. Method: x86 worktrees of both commits (`LPAC_WITH_APDU_AT=ON`),
pty→TCP bridge into `mock_modem.py` (default fingerprint; the socat-held TCP
connection makes the mock's 3-channel pool effectively session-global —
matching the real modem's cross-session persistence), 3× failed-init via
`LPAC_CUSTOM_ISD_R_AID=AABBCCDD` (gotcha: the env prefix is
`LPAC_CUSTOM_`, not `LPAC_`), then 1× legit `chip info`.

## Result

| build | 3× failed init | 4th (legit) session |
|---|---|---|
| stock `a43e46ae` | clean `euicc_init` JSON, exit 255 — **channels 1–3 leaked** (open, no close) | first MANAGE CHANNEL open answers **`6A81`** — init-dead from pool exhaustion |
| fixed `c004218` | same clean failure — **each abort closes its channel inline** | **full ISD-R session** (SELECT → FCI `6F048402A0009000` → STORE DATA) — pool never poisoned |

The stock column reproduces the bench modem's cross-session exhaustion
fingerprint (fork-research defect model #3) end-to-end on the wire.

## Complementary-leak insight (both fixes needed)

A crash **before** teardown still leaks even on the fixed build: both trees
carry the #467 applet segfault, and a mid-session crash bypasses disconnect
entirely (observed: crash runs leak; the accidental no-env first pass showed
3 crashed sessions exhausting the pool on both builds — and the same applet
UB manifesting as glibc `free(): invalid size` abort on one build, wild
`SIGSEGV` on the other: stack-garbage roulette, one root cause).
**c004218 covers clean init failures; #467 stops the crashes that bypass
teardown; together they close both leak classes.**

Design note validated: the fix's `close()` ignores the transmit result —
tolerates this modem class's refused closes (fork-research defect #2).

Artifacts: `stock-mock.jsonl`, `fixed-mock.jsonl` (full TX/RX wire logs);
per-run stdout/stderr + builds were in `/tmp/opencode/leakab/` (volatile).
On-router leg: designed, not run (owner-gated) — discriminator there is the
same exhaustion shape on the real card-global channel pool.
