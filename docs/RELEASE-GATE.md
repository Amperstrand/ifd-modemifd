# Release gate — ifd-modemifd

How a version of this driver is declared releasable. Every cell in the
targets table below points at **evidence that exists today**: a test
file in this repository, or a hardware run recorded in the README
evidence table / commit history. No aspirational rows — where evidence
does not exist, the verdict says so (SKIP or BLOCKED with reason and
owner), it is never assumed.

## Verdict semantics

| Verdict | Meaning | How it flips |
|---|---|---|
| **PASS** | Criterion was executed and is green; the named evidence (test binary, recorded hardware run) exists and reproduces. | Anyone re-running the evidence can confirm. |
| **FAIL** | Criterion was executed and is red. Release is blocked until a fixing commit lands and the evidence goes green. | A fixing commit + green re-run. |
| **SKIP** | Deliberately not executed for this release. Must record reason and owner; re-evaluated at every release cut — a SKIP is a decision, not a backlog promise. | Owner re-decides at the next cut. |
| **BLOCKED** | Cannot be executed until an external dependency clears (hardware-lane window, owner-gated action on a shared daemon). Not a code defect. | The dependency owner clears it; then it becomes PASS or FAIL. |

**Release rule.** A version may be cut only with **zero FAIL** among its
criteria. SKIP and BLOCKED are permitted only with reason and owner
recorded in the tables below.

## Evidence registry (what exists, as of 2026-10-24)

Software — reproducible on any host, `cargo build --release && cargo test --release`:

| Evidence | Command | Result |
|---|---|---|
| `tests/mock_at.rs` — transport semantics: no-resend invariants, AtOutcome classification, echo correlation, strict CSIM framing | `cargo test --release --test mock_at` | 15/15 PASS |
| `tests/logging.rs` + `tests/logging_enabled.rs` + `tests/logging_repair.rs` + `tests/logging_symlink.rs` — log off by default, APDU hex redaction, 0600 creation, legacy-0644 tightening, symlink refusal | `cargo test --release --test logging --test logging_enabled --test logging_repair --test logging_symlink` | 4/4 PASS |
| Release build, std-only crate (no runtime deps; `pcsc` is a dev-dependency for examples only) | `cargo build --release` | clean |

Hardware — recorded runs, **not** re-executed on demand. Owner of any
new hardware round: the hardware lane (bench doctrine; see
`tests/hardware/README.md`). Recorded runs:

| Evidence | Date | Result |
|---|---|---|
| pySim-shell full-card filesystem soak (operator USIM, ~60 EFs) | 2026-10-23 (v4.0, commit 3a475eb) | PASS, exit 0 |
| Sustained small-APDU load, SELECT MF ×300 | 2026-10-23 | 300/300, p50 400 ms |
| BPP-install-shaped load, 100-byte STORE DATA ×150 (logical channel) | 2026-10-23 | 150/150, p50 400 / p99 401 ms |
| Logical-channel lifecycle (open → AID select → close) | 2026-10-23 | clean, FCI answered |
| Real SGP.22 profile download + enable + notification (9eSIM ECP3 in RG502Q-EA) | 2026-10-23 | success, one shot, ~60 s, zero transport errors |

Historical (commit-recorded, superseded by the runs above): v3.2 fixed
the LP64 by-value `SCARD_IO_HEADER` ABI bug behind pcscd zero-blob
responses (conwrt-bench #22; every v3.3+ hardware run exercises
`IFDHTransmitToICC` and is the standing regression evidence — no
dedicated unit test exists for the ABI itself).

## Release targets by version

| Version (commit) | Release target | Criteria | Verdict | Evidence |
|---|---|---|---|---|
| v3.2 (f0e71a) | LP64 ABI correctness + session hardening | pcscd transmit arguments land in the correct registers (by-value `SCARD_IO_HEADER`, 16 bytes / two registers); no zero-blob responses | PASS | Standing: every later hardware run (v3.3 soak onward) passes through `IFDHTransmitToICC`; ABI documented in `src/lib.rs`. No dedicated unit test — recorded as a known evidence gap. |
| v3.3 (03848b3) | pySim-shell fully usable against the modem reader | full-card filesystem soak survives end-to-end | PASS | pySim-shell soak PASS exit 0 (hardware, recorded 2026-10-23; README evidence table) |
| v4.0 (3a475eb, = origin/master) | Hardware validation gate + production-proven | phased `validate.sh` gate exists and its recorded run is green; sustained + BPP-shaped loads pass; real SGP.22 provisioning completes one-shot | PASS | README evidence table (5 hardware rows, 2026-10-23); `tests/hardware/validate.sh`, `at_channel_probe.sh`, `examples/loadtest.rs` in tree |
| v3.4 (fa42259) | No-resend transport semantics + strict CSIM framing | software criteria: mock-modem battery green (no-resend invariants, classification, framing) | PASS | `tests/mock_at.rs` 15/15 |
| v3.4 | (same version) hardware criteria: post-v3.4 behavior re-validated on the RG502Q bench | **BLOCKED** | Hardware lane owns the modem; no post-v3.4 hardware round exists yet. Not a code defect — software evidence above is green. Owner: hardware lane. |
| v3.4.1 (2d50e52) | APDU log security | logging off by default; 0600 creation; symlink refusal; hex only under `IFD_MODEMIFD_LOG_APDU` | PASS | `tests/logging.rs`, `tests/logging_enabled.rs` 2/2. (As-shipped evidence gap: no test exercised symlink refusal — caught by the post-merge Codex audit, closed in v3.4.2 below.) |
| v3.4.2 (post-merge Codex audit of PR #2) | Log-open hardening | symlink refusal enforced atomically at open (`O_NOFOLLOW`), closing the check-to-open race; existing looser-permission logs (pre-v3.4.1 `0644`) tightened to `0600` on open via the fd — **fail closed** when the tightening fails (EPERM on a foreign-owned file) | PASS | `tests/logging_symlink.rs` (planted-symlink refusal), `tests/logging_repair.rs` (0644→0600, RED→GREEN), `tests/logging_failclosed.rs` (nobody-vs-0666 EPERM repro via passwordless sudo; skips with a note where sudo is unavailable) |

## Open items (SKIP / BLOCKED registry — recorded gaps, not promises)

| Item | Verdict | Reason | Owner |
|---|---|---|---|
| Post-v3.4 hardware round on RG502Q (pacing + no-resend under a live modem; incl. response-lag thrash test) | BLOCKED | Requires the hardware lane's bench window; an active tollgate session is using the router's modem-data WAN, blocking modem-disturbing tests | Hardware lane |
| Deployed bench `.so` rotation (pre-v3.4.1 binary still writes world-readable `/tmp` logs under live pcscd) | BLOCKED | Shared daemon; rotation is owner-gated, not autonomous | Bench owner |
| Serial transport (`/dev/ttyUSBx` direct) | SKIP | Not implemented; documented limitation, not a criterion of any released version | — |
| Multi-reader LUN registry beyond one reader | SKIP | Single-modem bench; registry present but untested beyond one reader (documented limitation) | — |
| IFDHandler conformance sweep (IFDHControl/GET_FEATURE_REQUEST, GetCapabilities coverage, CGLA fallback) | SKIP | In progress in a parallel lane; not a criterion of v3.2–v3.4.1 — will gate the next version cut, not past ones | Conformance lane |

## Using this gate

1. Regenerate software evidence: `cargo build --release && cargo test --release` — expect 21/21 (15 mock_at + 6 logging across 5 binaries; the fail-closed probe skips with a printed note where passwordless sudo is unavailable).
2. Hardware criteria: only the hardware lane executes them; a new run
   supersedes the recorded one only when its report lands (stamped,
   persistent — never green-by-default).
3. To cut a version: every criterion of that version PASS, zero FAIL;
   any SKIP/BLOCKED carries reason + owner from the registry above.
