# ifd-modemifd

pcscd IFD handler exposing a **cellular modem SIM slot** as a PC/SC smartcard
reader. Any PC/SC application (pySim, OpenSC, cardpeek, lpac's stock pcsc
backend, ...) can then talk to the card seated in a router's modem — no
external reader hardware.

## Status: production-proven (Rust). The C reference implementation is retired and removed.

This driver has carried a **complete real-world SGP.22 eSIM provisioning**:
a 9eSIM (ECP3 eUICC) seated in a Quectel RG502Q-EA modem was profile-
downloaded and enabled end-to-end through this driver (lpac → pcscd →
ifd-modemifd → modem AT+CSIM), single one-shot attempt, ~60 s wall clock,
zero transport errors. The 1 GB roaming data profile then registered and
passes live traffic through the modem.

Evidence base:

| Test | Result |
|---|---|
| pySim-shell full-card filesystem soak (operator USIM, ~60 EFs) | PASS, exit 0 |
| Sustained small-APDU load (SELECT MF ×300) | 300/300, p50 400 ms |
| BPP-install-shaped load (100-byte STORE DATA ×150 on logical channel) | 150/150, p50 400 / p99 401 ms |
| Logical-channel lifecycle (open → AID select → close) | clean, FCI answered |
| Real SGP.22 profile download + enable + notification | success, one shot |

## Architecture

    PC/SC app → pcscd → libifd_modemifd.so (this crate)
      → TCP → socat bridge on the OpenWrt router → modem AT port
      → AT+CSIM (basic + logical channels) → seated SIM

Rust `cdylib`, std-only, no external crates. The IFDHandler 3.0 C ABI is
implemented in `src/lib.rs` (constants mirrored from ifdhandler.h in
`src/consts.rs`). Panic-safe: every export that performs driver work is
wrapped in `catch_unwind`; the two constant-success stubs
(`IFDHSetProtocolParameters`, `IFDHSetCapabilities`) contain no
fallible code.

## Modem findings baked into the driver (Quectel RG502Q-EA, Zyxel AACR firmware)

Measured with `tests/hardware/validate.sh` + the APDU shape probes — useful
to anyone driving this modem family as a smartcard reader:

- **Basic-channel filter**: the modem silently drops SELECT-by-AID/DF-name
  (A4 with P1=04) on the basic channel (channel 0); the same APDU on a
  logical channel passes instantly. Silence shows up as a driver timeout
  returning SW 6F00. (An earlier version of this finding also claimed a
  READ BINARY drop on the basic channel — contradicted by later
  bench evidence reading real EF data there; treat only the A4 half as
  established.)
- **CSIM hex case matters**: the modem's `AT+CSIM` parser answers a bare
  `ERROR` to lowercase hex payloads (Qualcomm-class behaviour; pySim
  gerrit 8e15c18, lpac #428/#466). This driver emits uppercase
  (`src/at.rs`), which is the accepted form — pinned by regression
  tests in `tests/mock_at.rs`.
- **MANAGE CHANNEL close needs the 5-byte form**: `xx 70 80 xx 00`
  (P3=00) returns `9000`; the 4-byte form is dropped as malformed
  (AT-level `ERROR`). See `tests/mock_at.rs` pins and the corrected
  findings in `docs/CSIM-SESSION-FINDINGS.md` (correction 6).
- **No command-size ceiling**: well-formed STORE DATA and SELECT commands
  of 100+ bytes transport fine; command SIZE is not the BPP-load failure
  mode. Malformed APDUs (bad Lc) ARE silently dropped.
- **AT+CGLA responses are swallowed** on this firmware (CCHO/CCHC work);
  AT test commands (`AT+X=?`) answer bare `OK` with no range line —
  capability probes must not require the `+X:` prefix form.
- **CSIM pacing is mandatory**: back-to-back AT+CSIM bursts trigger SIM-
  session thrash (URC flood, lagging/swallowed responses). The driver
  enforces a **400 ms inter-command floor** (`TRANSMIT_GAP_MS` in
  `src/reader.rs`) between paced transmits; presence probes (`AT+CPIN?`)
  are deliberately exempt so pcscd's presence poller stays cheap. Under
  this floor, the v4.0 hardware validation measured **p50 400 ms
  round-trips** (SELECT MF ×300 and 100-byte STORE DATA ×150, zero
  failures) — the pacing gap dominates the modem's own response time.
  Tunable per pcscd process: `IFD_MODMIFD_TX_GAP_MS=<ms>` (default 400;
  unparsable values fall back to the default; read once at the first
  paced transmit). `0` disables pacing — do not disable on thrash-prone
  firmware except for testing.
- **The AT port shuffles across resets** (ttyUSB0-3): never hardcode it.
  The reference socat bridge init script auto-detects at start.

For the complete corrected record of this modem's verified behaviour —
including the 2026-10-25 on-device SGP.22 provisioning proof over plain
AT+CSIM (complete download, enabled operational profile in place, no
composition switch) — see `docs/KNOWN-GOOD-ONDEVICE-PROVISIONING.md`,
`docs/CSIM-SESSION-FINDINGS.md`, and the evidence ledger in
`docs/INDEPENDENT-REVIEW-9ESIM-20261025.md`.

## Validation gate (split suites, dry-run by default)

`tests/hardware/validate.sh` chains two focused suites —
`validate-sacrificial.sh` (zero-risk class-A qualification of a disposable
SIM: SELECT by FID, EF reads via LOGICAL channels, modem drop fingerprints)
and `validate-esimgate.sh` (pre-provisioning read-only eSIM gate: ISD-R
readiness, EID, chipInfo). The default run is a **dry-run against a local
mock modem** — no hardware, no pcscd. Hardware sessions run through
`tests/hardware/lane-runner.sh` (private socket-activated pcscd lane,
reader selected BY FRIENDLYNAME, card-safety declarations enforced — the
production 9eSIM is refused as a target). Reports are stamped JSON+MD with
truthful PASS/FAIL/SKIP/BLOCKED verdicts; unexecuted cases are never green.
The gate encodes every failure class that has actually burned a one-shot
eSIM activation code, plus the card-handling doctrine (never hammer
secure-channel-class commands — one-shot authentication budgets on GP/eUICC
cards are small and non-renewable). See `tests/hardware/README.md`.
Per-version release verdicts (PASS/FAIL/SKIP/BLOCKED, mapped to existing
evidence only): `docs/RELEASE-GATE.md`.

## Transport semantics (v3.4): no resend after full delivery

An AT command whose line was fully written may already be executing on
the modem/SIM — a re-sent `AT+CSIM` could duplicate a state-changing
APDU (GP secure channel, eSIM provisioning, PIN) invisibly to the
application. Since v3.4 every exchange is classified into a typed
outcome (`AtOutcome` in `src/reader.rs`) and the retry policy follows
the classification:

| Outcome | Meaning | PC/SC client sees | Retried? |
|---|---|---|---|
| `Ok` | echo + `+CSIM:` answer + final `OK` all received | response APDU (unparsable content → SW 6F00) | never |
| `Error` | modem answered final `ERROR` (definitive refusal) | SW 6F00 (card-level) | never |
| `Refused` | our echo came back — the modem provably heard us — but no final result code (firmware-intercepted select, silent refusal) | SW 6F00 (card-level; v3.3 semantics preserved so AID-probing tools survive) | never |
| `NotSent` | the write failed before the terminating CR left the host — nothing executable reached the modem | `IFD_COMMUNICATION_ERROR` (transport-level) | **yes — exactly once; the only retried class** |
| `Ambiguous` | the line was fully written but no echo was ever seen (stalled bridge / death mid-relay — indistinguishable from "the modem already ran it") | `IFD_COMMUNICATION_ERROR` (transport-level) | never |

Card-level failures (`Error`, `Refused`, or a verified `Ok` exchange
whose content fails strict parsing) surface as SW `6F00`; exchanges with
unknown or provably-failed delivery surface as transport errors — never
a synthetic card status word.

Strict CSIM framing (same release): the declared length of a
`+CSIM: <len>,"HEX"` line must equal the hex-character count (TS 27.007
§8.45, the pySim convention), the R-APDU is capped at 258 bytes, and the
parse is **echo-correlated** — only `+CSIM:` lines after our command
echo answer the current exchange, so a stale response from a lagging
exchange can no longer be mistaken for the card's answer. Response
accumulation is bounded at 8 KiB: a broken or hostile bridge drops the
connection instead of buffering without limit.

Software evidence: `tests/mock_at.rs` (15 tests, scripted mock modem) —
the no-resend invariants (exactly one delivery under blackhole,
post-delivery cut, echo-silence, missing final `OK`, and flood),
echo correlation, strict framing, and parser unit cases. No post-v3.4
hardware round on the RG502Q has been run yet — see
`docs/RELEASE-GATE.md` for the recorded gap.

## Design decisions

- Prefix-matched, echo-correlated AT responses (`at_expect`): on a shared
  modem port, foreign URCs (e.g. `+CPIN: READY`) arrive interleaved;
  early-exit on any `OK` eats them (bug found in the retired C
  implementation and in any naive CSIM loop). Same pattern lpac uses.
  Since v3.4 only lines after our command echo answer the exchange
  (see Transport semantics).
- Pacing: see CSIM pacing above.
- Fake ATR: modems cannot report the real ATR over AT commands; a sysmo-class
  T=0 ATR is substituted (documented limitation).
- Logical channels: raw MANAGE CHANNEL passes through AT+CSIM on proven
  firmwares (no CCHO/CGLA interception needed).

## Debug logging (v3.4.1)

Debug logging is **off by default**, opt-in by environment variable. The
defaults come from a bench finding (2026-10-24): the pre-hardening
driver's debug gate was compiled on and wrote full APDU hex
world-readable (0644) to a fixed `/tmp` path — authentication and
provisioning payloads on disk — and a pre-planted symlink would have
redirected the append into arbitrary files.

| Variable | Effect |
|---|---|
| `IFD_MODMIFD_DEBUG` | master gate: any value enables the log; unset, no log file is ever opened. (Spelling is `MODMIFD` — no second `E` — matching the code.) |
| `IFD_MODEMIFD_LOG_APDU` | additionally logs APDU payload hex; without it, transmits are recorded as lengths only (`XMIT len=N`). (This one IS `MODEMIFD`.) |
| `IFD_MODMIFD_LOG_PATH` | log path override (default `/tmp/ifd-modemifd.log`), e.g. for bench isolation. |
| `IFD_MODMIFD_TX_GAP_MS` | inter-command pacing floor in ms (default 400; see CSIM pacing above). `0` disables — testing only. Read once at the first paced transmit. |

File behavior: created `0600`, append-only; an existing log file with
looser permissions (e.g. the pre-v3.4.1 world-readable `0644` log) is
tightened to `0600` on open; if that tightening fails (a writable file
owned by another user), logging fails closed — nothing is written; a
symlinked log path is refused outright
rather than followed — enforced atomically at open (`O_NOFOLLOW`), not
just by a pre-check. The environment decision is cached per process
(`OnceLock`), so the variables must be set on the pcscd daemon itself
(e.g. via its systemd unit), not on the client application. Software
evidence: `tests/logging.rs` (off by default), `tests/logging_enabled.rs`
(hex redaction + `0600` creation), `tests/logging_repair.rs` (legacy
`0644` tightened on open), `tests/logging_symlink.rs` (refusal through a
planted symlink) — deliberately separate test binaries because of that
same OnceLock cache.

## Known limitations

- Response-lag aliasing under extreme modem thrash: when the firmware enters
  the URC-flood state, responses can lag by one command. Since v3.4 the
  parse is echo-correlated — stale pre-echo `+CSIM:` lines are ignored and
  an answer arriving with no echo at all is distrusted — which defeats the
  observed aliasing shape. A modem emitting a stale answer *after* our
  echo could still alias in theory; that shape has not been observed, and
  a live thrash test is still pending (`docs/RELEASE-GATE.md`). Recovery
  from the URC-flood state itself is idle/CFUN cycle. (No aliasing
  observed under paced operation, including the full provisioning run.)
- Serial transport (`/dev/ttyUSBx` direct) not yet implemented (TCP only).
- Multi-reader LUN registry: create-by-name / remove / cross-reader
  isolation are covered by mock-modem tests (`tests/mock_at.rs`); not
  yet exercised against two physical modems.

## Build & install

    cargo build --release
    # target/release/libifd_modemifd.so → /usr/local/lib/

    # /etc/reader.conf.d/modemifd.conf
    FRIENDLYNAME NR7101_SIM_dut
    DEVICENAME <router-ip>:7002        # socat: TCP-LISTEN:7002,reuseaddr,fork FILE:/dev/ttyUSBx,raw,echo=0,b115200
    LIBPATH /usr/local/lib/libifd-modemifd.so
    CHANNELID 1

Non-root PC/SC clients may need a polkit rule allowing `access_pcsc` /
`access_card` for their group (SSH sessions are not polkit-active seats).

## License

LGPL-2.1-or-later
