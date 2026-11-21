# vpcd handshake debug — NR7101 agent rejected by libifdvpcd-amperstrand (2026-11-21)

**Status: ROOT-CAUSED, fixed agent-side, reproduced offline old-vs-new.**
Branch `debug/vpcd-handshake` (local, not pushed). No router/modem was
touched; all analysis is source-level plus loopback mocks.

## Observed failure (bench, 2026-11-21)

The modem-vpicc agent (master `d6c214d`) on the NR7101 connects out to
the pcscd lane's `libifdvpcd-amperstrand.so` listener (TCP :35963):

- the TCP connect is **accepted, then closed by the driver side almost
  immediately** (host shows the accepted socket in FIN-WAIT-2),
- the agent **logs no error** and silently reconnects every 500 ms,
- pcscd lists the reader `NR7101_9eSIM 00 00` / `00 01`, but
  `SCardConnect` says **"no smart card"**.

## Root cause: a race the agent always loses — modem handshake between vpcd connect and serve loop

The driver expects **no preamble, no version byte, no first frame from
the vpicc** — the vpcd protocol is pure request/response (`[u16 BE
length][payload]`; 1-byte controls OFF=0 / ON=1 / RESET=2 / ATR=4 per
upstream `vpcd.h`). The mismatch is **timing**, not framing:

1. The agent's `run()` did, per cycle: `TcpStream::connect(vpcd)` →
   **`Reader::init_handshake()`** (blocking modem AT session: transport
   open + `ATE1` ≤ 2 s + `AT+CSIM=?` ≤ 2.5 s) → only then entered
   `serve_connection`'s read loop. During the handshake it never read
   the vpcd socket.
2. The driver's accept is **poll-driven**: pcscd calls `IFDHICCPresence`
   every `PCSCLITE_STATUS_POLL_RATE` ≈ 400 ms → `vicc_present` →
   `vicc_connect` = `waitforclient(poll, 0 ms)` + `accept()`. The
   agent's TCP connect completes into the kernel backlog instantly
   (`listen(sock, 0)` still holds one), so **accept + ATR probe happen
   within ≤ 400 ms of connect** — while the agent is still handshaking
   with the modem.
3. The amperstrand patch sets `SO_RCVTIMEO = VPCD_RECV_TIMEOUT_SEC = 2 s`
   on the accepted socket (upstream `recvall` blocks forever — the
   pcscd-EH wedge the patch fixed). An ATR answer later than 2 s →
   `recvall` = −1 → patched `vicc_transmit` ejects on `r < 0` →
   `close(client_sock)`. **That close is the FIN; the socket sits in
   FIN-WAIT-2 because the agent still holds its end** (it is mid-
   handshake and not reading).
4. The agent finishes the handshake (~3 s on the real tty: stty setup +
   echo-correlated exchanges + pacing), enters `serve_connection`,
   answers the *buffered* ATR control into the half-closed socket (→
   RST), errors (`session ended` — written only to the opt-in `--log`
   file, never stderr → "logs no error"), sleeps 500 ms, reconnects.
   Loop forever.
5. pcscd: every presence poll ejects before the agent can answer, so
   `IFDHICCPresence` = `IFD_ICC_NOT_PRESENT` forever → reader listed,
   `SCardConnect` = "no smart card".

A fast (< ~2 s) modem handshake can win the race; the NR7101 tty
handshake (~3 s) loses it every cycle. Both bench-proven vpicc clients
in ccid-firmware-rs do all slow setup **before** connecting and enter
their read loop the instant connect returns:

- `crates/ccid-service/src/vpcd.rs` (ESP32 firmware client, handler
  constructed outside the reconnect loop — its #349 lesson),
- `tests/hardware/host/vjavacard/src/main/java/org/amperstrand/VpcdBridge.java`.

### Driver source references (all verified line-by-line)

- upstream `vpcd.c` @ `93286a9` (the pin in
  `tests/hardware/host/build_libifdvpcd_amperstrand.sh`):
  `opensock` (`listen(sock, 0)`), `waitforclient` (poll+accept),
  `vicc_connect` (accept-on-demand), `vicc_present` = connect+getatr,
  `vicc_getatr` = `vicc_transmit(VPCD_CTRL_ATR)`, `vicc_transmit`
  (send + blocking recvall; eject on failure), `vicc_eject`
  (close client, listener persists).
- `libifdvpcd-amperstrand.patch` (ccid-firmware-rs): `SO_RCVTIMEO`
  2 s in `vicc_connect`; eject only on `r < 0` (zero-length answer =
  valid "ICC absent").
- upstream `ifd-vpcd.c`: `IFDHICCPresence` → `vicc_present`.
- `docs/transport-research-154.md` (ccid-firmware-rs) §2.1: the same
  lifecycle facts, bench-verified there.

## Secondary finding: swapped control-constant names

The agent's `vpcd.rs` named `CTRL_ATR = 0x02` / `CTRL_RESET = 0x04`;
upstream `vpcd.h` says `VPCD_CTRL_RESET = 2`, `VPCD_CTRL_ATR = 4`.
Behaviorally harmless (both are answered with the ATR frame; OFF = 0 is
the only asymmetric control and was correct; ON = 1 landed in "ignored",
which equals upstream fire-and-forget). Fixed to the upstream values,
`CTRL_ON` now matched explicitly (no reply — `vicc_poweron` expects
none). `tests/hardware/VPCD-ARCHITECTURE.md` carried the same swapped
labels; corrected.

## The fix (agent-side, `src/agent.rs`)

`run()` reordered — **modem first, vpcd second**:

1. `Reader` is constructed once and survives reconnects (firmware
   client #349 doctrine; also stops re-spawning `stty` per reconnect on
   the router).
2. `init_handshake()` runs **before** `TcpStream::connect` each cycle;
   if the modem is unhealthy the agent **stays unconnected** (reader
   honestly absent) instead of churning vpcd connections.
3. `serve_connection` is entered the instant connect returns — the
   first ATR probe is answered in milliseconds (measured 0–2 ms).

## Offline reproduction (loopback only)

`repro/` contains the kit:

| file | role |
|---|---|
| `mock_libifdvpcd.py` | the patched driver as a machine: poll-cadence accept, ATR control on accept, 2 s answer window, eject on expiry, listener persists (models `vicc_present`/`vicc_getatr`/`vicc_transmit`/`vicc_eject`) |
| `slow_modem.py` | AT-dialect modem mock; `ATE1` and `AT+CSIM=?` each answer after 1.5 s — each inside its at_full deadline (2 / 2.5 s), together past the driver's 2 s window: the NR7101 "slow-but-healthy tty" shape |
| `run_repro.sh OLD_BIN NEW_BIN [DUR]` | runs both agents against the same mocks, writes `transcripts/` |

(The task brief mentioned `tests/hardware/mock_vpicc.py`; no such file
exists in either repo — the available mocks were
`tests/hardware/mock_modem.py` and the agent's `--target tcp://` mode.
The driver-side mock above fills that gap and lives here with the
evidence.)

### Results (2026-11-21, host loopback, 10 s runs)

OLD = master `d6c214d` release build; NEW = this branch:

```
OLD: {"accepts": 3, "answered": 0, "ejects": 3, "peer_closed": 0, "latencies_ms": []}
NEW: {"accepts": 1, "answered": 70, "ejects": 0, "peer_closed": 0,
      "latencies_ms": [0,0,1,0,...,2,1,1,0,...]}   # max 2 ms
```

- `transcripts/old-driver.log` — the smoking-gun timeline: `accept →
  (2.001 s silence) → eject` per cycle, three cycles, zero answers.
  The 2.001 s gap *is* `VPCD_RECV_TIMEOUT_SEC`.
- `transcripts/new-driver.log` — one accept, every probe answered.
- `transcripts/old-finwait2-observation.txt` — `ss -tn state
  fin-wait-2` sampled every 250 ms during a 12 s OLD run: 12 hits across
  the 3 agent source ports — the same FIN-WAIT-2 the bench showed,
  reproduced locally.

## Regression tests (agent/tests/vpcd.rs)

- `slow_modem_handshake_cannot_lose_the_atr_race` — a ~3 s modem
  handshake (delays inside each exchange deadline, total past the 2 s
  window) against `DriverMock` (the rust twin of the python driver
  machine): asserts every probe answered inside the 2 s window, **zero
  ejects, exactly one connection**. Fails on master's ordering,
  passes with the fix.
- `agent_survives_the_patched_driver_machine` — instant modem through
  the same machine (control for the mock itself).
- `MockModem::spawn_delays` + `DriverMock` are reusable harness pieces.

## Validation

- `cargo test -p modem-vpicc --test vpcd` — 7/7 green (3 codec units,
  the original contract battery, the 2 new regression tests).
- `cargo test --workspace` — everything green **except**
  `ifd-modemifd --test logging_enabled::apdu_hex_not_logged_by_default`,
  which fails identically on **pristine master** (verified via stash) —
  pre-existing, unrelated to this change (it exercises
  `Reader::transmit` logging, not the agent or vpcd path). Not touched
  here; flagged for its owner.

## Operational notes

- With the fix, a vpcd-listener-side observer sees the agent connect
  only after the modem is confirmed alive; reader churn disappears;
  pcscd's presence poll succeeds on the first probe after connect.
- The fix deliberately does NOT change the APDU path: transport errors
  still map to SW 6F00 (`vpcd` has no transport-error frame); the ATR
  remains the configured static value (a modem cannot report a real
  AR over AT — documented limitation).
- Deploying to the router still requires the mipsel build + procd
  restart — out of scope for this offline task (no device access).
