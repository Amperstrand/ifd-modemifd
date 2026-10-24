# Hardware validation suites (split gate)

The old monolithic `validate.sh` gate (v4.0) is retired: it stopped the
SYSTEM pcscd for its direct-AT probe and hardcoded a router SSH target.
The gate is now split into two focused suites plus a lane runner, with a
default dry-run mode that needs no hardware at all.

| File | Role |
|---|---|
| `validate.sh` | chain entry: runs sacrificial, then esimgate only if sacrificial was green (otherwise esimgate cases are reported BLOCKED — never silently green) |
| `validate-sacrificial.sh` | zero-risk class-A qualification of a disposable SIM (see below) |
| `validate-esimgate.sh` | pre-provisioning READ-ONLY eSIM gate (see below) |
| `lane-runner.sh` | private-pcscd lane wrapper (hardware sessions) + mock dry-run + `--plan` |
| `mock_modem.py` | mock Quectel-class AT modem for dry-run (loopback only) |
| `suite.py` | case runner: direct-AT and pcsc backends, JSONL output, card-safety guard |
| `report.py` | stamped JSON+MD reports with enforced truthful verdicts |
| `at_channel_probe.sh` | router-side direct-AT probe (hardware sessions only, unchanged) |
| `examples/loadtest.rs` | PC/SC load tester (hardware sessions; ad-hoc use) |

## Quickstart — dry-run (no hardware, no pcscd)

```sh
./tests/hardware/validate.sh                # whole chain against the mock modem
./tests/hardware/lane-runner.sh --dry-run   # same, via the lane runner
```

A mock modem is started automatically on `127.0.0.1:17002` if nothing is
listening. Reports land in `tests/hardware/reports/<stamp>-<suite>/`
(`report.json` + `report.md` + `cases.jsonl`). A dry-run report is always
annotated as such and can never green-light hardware operation or
provisioning.

## validate-sacrificial.sh — disposable-SIM qualification (class-A only)

Every case is non-mutating and non-authenticating; nothing here consumes
PIN attempts, GP error budgets, or SM-DP+ windows:

| Cases | What they prove |
|---|---|
| SAC-001 | basic-channel SELECT by FID (the one basic-channel op the modem forwards) |
| SAC-002/003 | modem drop fingerprints: A4/P1=04 and B0 on the basic channel are silently dropped (echo-then-silence → driver 6F00). The DROP is the expected PASS, so these probes never reach the card. A card answer (6A82/6982/6A86) is still a PASS but flagged as filter drift |
| SAC-004..008 | the real read path per README modem findings: MANAGE CHANNEL open → SELECT MF and EF.ICCID (2FE2) by FID → READ BINARY (captures the ICCID as the fixture fingerprint) → close — all on a LOGICAL channel (`SAC_REPEATS` re-runs the cycle) |
| SAC-009 | post-cycle basic-channel sanity select |

## validate-esimgate.sh — pre-provisioning read-only gate

| Cases | What they prove |
|---|---|
| ESG-001..003 | ISD-R readiness: logical channel open → SELECT ISD-R by public AID (non-authenticating, doctrine-sanctioned) → close |
| ESG-004 | `lpac chip info` ×N (ES10b GetEID / euiccInfo): EID captured and cross-checked for stability. SKIPs when lpac is absent or the backend is the mock |

The provisioning gate opens ONLY when mode=hw AND every case PASSed. In
dry-run, or with any FAIL/SKIP/BLOCKED, the report says NO — do not
provision.

## Hardware sessions (private pcscd lane, reader by FRIENDLYNAME)

```sh
# inspect the resolved plan first:
./tests/hardware/lane-runner.sh --plan

# then, with the lane's hardware owned and a DISPOSABLE card seated:
MODEMIFD_ADDR=<bridge host:port> MODEMIFD_CARD=sacrificial \
    ./tests/hardware/lane-runner.sh hw
```

The runner follows the ccid-firmware-rs AGENTS.md lane recipe:
`systemd-socket-activate` + private `reader.conf.d-lanes/<lane>` dir
(containing ONLY the modemifd entry) + `PCSCLITE_CSOCK_NAME` for every
client. The system pcscd is never stopped or touched. The reader is
selected BY FRIENDLYNAME at two layers and never implicitly "first".

Safety interlocks (all enforced in code, not docs):

- no default `MODEMIFD_ADDR` — nothing ever dials a router implicitly;
- non-loopback targets additionally require
  `HW_CONFIRM=YES_I_OWN_THE_HARDWARE`;
- hw runs require `MODEMIFD_CARD` declaring the seated card, and any value
  naming the **9eSIM is refused outright** — the production 9eSIM must
  never be a suite target (session-2 directive). The sacrificial suite
  additionally requires `MODEMIFD_CARD=sacrificial`.

## Doctrine

1. **Never hammer secure-channel-class commands.** All cases here are
   non-authenticating (SELECTs, EF reads, public-AID select, GetEID).
   Authentication happens exactly once — in a real download.
2. **Read-only by construction.** No case in either suite mutates card
   state; the only "expected failure" cases (drop fingerprints) are dropped
   by the modem before reaching the card.
3. **Private lanes only.** Never stop the system pcscd for probing (other
   users' readers live there); the lane recipe isolates this reader
   completely.
4. **One shot per activation code.** If the esimgate is green and the
   download still fails, STOP. Diagnose before retry.
5. **Re-detect the AT port after any modem power cycle** (ttyUSB0-3
   shuffle). The lane bridge script auto-detects.
6. **Truthful reports.** Unexecuted cases are SKIP (with reason) or BLOCKED
   (prerequisite/transport), never green; a report with zero executed
   cases qualifies nothing (enforced in `report.py`).

## Report verdict ladder

Case verdicts: `PASS` (executed, evidence recorded) · `FAIL` (executed,
wrong answer) · `SKIP` (deliberately not run, reason recorded) · `BLOCKED`
(prerequisite or transport unavailable). Suite verdict: FAIL if any FAIL,
else BLOCKED if any BLOCKED, else PASS only when every case ran and passed,
else SKIP. Exit codes follow: 0 pass · 1 fail · 2 skip/blocked · 3 infra.
