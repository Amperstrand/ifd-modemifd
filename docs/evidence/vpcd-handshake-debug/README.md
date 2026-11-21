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

- `cargo test -p modem-vpicc --test vpcd` — 10/10 green (3 codec units,
  the original contract battery, the 2 driver-machine regression tests,
  and the 3 pty-based tty-window regressions from iteration 2).
- `cargo test -p ifd-modemifd` — everything green **except**
  `logging_enabled::apdu_hex_not_logged_by_default`,
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

## Iteration 2 — live test follow-up (2026-11-21, deployed sha 7e979f45)

The mipsel build of `43127dc` confirmed the fix direction (the ATR race
is gone) and exposed the next bug: after two full reset→window cycles
the agent logged only its config line and went silent — no handshake
verdict, no vpcd TCP (verified host-side), process alive for minutes.

**Root cause: `SerialLink::set_read_timeout` was a no-op.** The AT
engine's `at_full` arms a per-read timeout (150 ms) and checks its
exchange deadline *between* reads — but on a tty the only real deadline
mechanism is termios VMIN/VTIME, and the stub did nothing, so the
response read blocked in the kernel (open-time `stty min 1 time 0`).
When the modem's AT window died mid-handshake (this modem's windows
last only ~minutes after a `uqmi --set-device-operating-mode` reset),
the blocked read never returned to check the deadline → `run()` never
reached its retry loop. The mock battery couldn't catch this: every
mock modem is TCP, where `set_read_timeout` is a real socket timeout.

**Fix (`src/transport.rs`):** `SerialLink::set_read_timeout` now arms
VMIN=0 + VTIME=ceil(d/100 ms) via `tcgetattr`/`tcsetattr` (None
restores VMIN=1/VTIME=0). `SerialLink::read` maps the resulting
`Ok(0)` (timer expiry — a tty has no EOF; hangup is EIO) to
`WouldBlock` so the deadline loop treats it as "no data yet" instead
of "peer closed" (healthy serial exchanges sit silent 300–400 ms — the
card-service floor — while the engine polls at 150 ms). Every
`at_full` exchange is now bounded by its existing deadline on ALL
transports: handshake worst case ≈ 2 s + 2.5 s + one read overshoot.

**Plus rate-limited live-state logging (`src/agent.rs`)**: the first
failed handshake and then every 20th attempt (≈10 s at the 500 ms
cadence) is logged, and the recovery ("AT window opened after N failed
attempts") is announced — the fail-fast-while-AT-dead steady state is
now visible in the opt-in log without spam.

**Regression tests** (`agent/tests/vpcd.rs`, pty-based — the only
faithful local double for a serial port):

- `tty_read_deadline_is_enforced` — a serial read under an armed
  deadline returns `WouldBlock` when the timer fires (this is the
  exact deployed hang; verified to FAIL against the no-op stub).
- `tty_window_death_mid_handshake_is_bounded_and_recovers` — ATE1
  answered, AT+CSIM=? goes silent: `init_handshake` fails bounded
  (≤8 s guard over the ~2.7 s deadline sum) and the NEXT attempt once
  the window reopens succeeds.
- `agent_rides_tty_windows_with_real_binary` — the production steady
  state through the real binary on a real tty: window closed at boot →
  bounded handshake failures and ZERO vpcd connections (health gate);
  window opens → connect → first ATR probe answered in-window → one
  stable connection, zero ejects.

A/B proof: with `set_read_timeout` reverted to the no-op stub, all
three tty tests fail (`agent_rides…` reproduces the live symptom
verbatim: "window opened but the agent never served — accepts=0");
with the fix the suite is 10/10. `slow_modem.py` also gained
`--go-silent-after N` to model window death in the python repro kit.

Expected steady state on the router (owner's framing): handshake fails
fast while AT-dead, retry every 500 ms, connect the moment a window
opens, serve until it closes; the serve loop already tolerates
session-end (bounded `at_full` + reconnect).

## Iteration 3 — round-2 live test follow-up (2026-11-21, deployed sha a8e772da)

The VMIN/VTIME build STILL hung: three restarts (ttyUSB2 in-window,
ttyUSB2 dead-port, ttyUSB3 in-window — all identical) showed only the
config line, no failure heartbeat, no connect. Kernel evidence from
the owner: `State S`, **`Threads: 1`**, empty wchan, single fd to the
target tty held open.

Analysis notes:

- `Threads: 1` is EXPECTED for this agent — the only pump thread in
  the transport layer belongs to the ssh link (`modemifd-ssh-pump`);
  the tty path is deliberately single-threaded. The smoking gun is not
  the thread count but the never-returning blocking read.
- The VMIN/VTIME arm demonstrably works on a pty (round-2 tests) yet
  did not bound the read on the real option-driver USB tty. The
  plausible hardware-side mechanisms — `tcsetattr` failing silently
  (EIO on a wedged port; `at_full` ignored the Result), or the
  arm not taking effect on that fd — cannot be reproduced by a pty,
  which always honors termios. Conclusion: **the exchange deadline
  must not depend on termios state at all.**

Fix (this round):

1. **poll(2)-gated reads** — `ModemLink::wait_readable(d)` (new trait
   method) implemented as `poll(2)` on the raw fd for `SerialLink` and
   `TcpStream`, and as a channel `recv_timeout` for `SshLink`.
   `at_full` now loops `wait_readable(150 ms slice) → nonblocking
   read`; it never calls `set_read_timeout` and never issues a
   blocking read. Boundedness is by construction on every fd kind,
   including a USB tty that refuses or ignores termios.
2. **Live open-diagnostics** — `transport::last_open_report()`: every
   open attempt records stty exit status, device-open result, and
   whether the termios deadline arm was accepted (`ok` vs
   `REFUSED: <errno>`); `open_serial` also verifies the arm at open
   time. The agent's failure heartbeat now carries the report
   ("no target answered (attempt N); last open: tty /dev/ttyUSB2:
   stty ok; open ok; termios arm REFUSED: …"), and the success line
   names the winning target. Live runs can no longer be silent about
   where they stopped.
3. **Multi-target `--target`** (the mid-session port flap the owner
   documented: ttyUSB2 dead 60 s later while ttyUSB3 was alive):
   comma-separated list, one cached Reader per target, tried in order
   every cycle — the agent follows the window across ports instead of
   pinning the dead one. e.g.
   `--target tty:/dev/ttyUSB2,tty:/dev/ttyUSB3`.
   Per-cycle cost while fully AT-dead: one ATE1 deadline (~2 s) per
   target.

Regression tests added (suite now 13):

- `wait_readable_bounded_with_blocking_termios_armed` — the round-3
  invariant: with termios deliberately sabotaged back to blocking
  VMIN=1/VTIME=0, `wait_readable` still returns bounded. This is the
  exact violation class that hung the hardware and that no pty could
  reproduce through the read path.
- `tty_window_death_mid_read_is_bounded` — the daemon echoes the
  command, emits one stray byte of the response, then dies: the
  exchange fails bounded (deadline), no kernel block.
- `multi_target_falls_through_dead_port_to_live_one` — dead first
  target (connection refused) + live pty second: the agent serves
  through the second with one connection and zero ejects.

Validation: agent suite 13/13; full workspace 48 passed with 4
pre-existing failures — all in the logging family
(`logging_enabled`, `logging_repair` ×2, `logging_symlink`), verified
identical on pristine `c3e8096` (bench-environmental, untouched by
this branch); the transport-facing suites (`mock_at` 28, ssh,
pacing, vpcd 13) are green. Loopback driver-machine repro on the new
release binary: accepts=1, answered=26, ejects=0 (unchanged vpcd
behavior).

## Iteration 4 — the actual root cause of EVERY live stall (2026-11-21, round-4 live log)

The round-3 diagnostics cracked it on the next deployment: the live
log read

```
tty /dev/ttyUSBx: stty spawn failed: No such file or directory (os error 2); open ok; termios arm ok
```

**This OpenWrt router has NO `stty` binary at all** — not a busybox
applet, not installable from the broken apk feed; the arch doc's
"GNU stty 9.7" was only ever deployed on the production router.
Every stall across rounds 1–3 traces to this: the tty-open path
configured the port by spawning `stty -F … 115200 raw -echo min 1
time 0`; the spawn failed with ENOENT, the failure was ignored, and
the tty stayed in **canonical mode** —

- under ICANON, VMIN/VTIME are ignored by the kernel → the blocking
  read hung (round-2's Threads:1 stall) even with the deadline
  "armed" (`tcsetattr` accepts the values; they are inert);
- kernel ECHO echoed the modem's inbound bytes back at it and
  line-buffered/translated both directions → the AT handshake could
  never succeed **regardless of AT windows** (why "ttyUSB3 in-window"
  behaved identically);
- round 3's poll(2) gating fixed the HANG (bounded retries, visible
  heartbeat) but not the futility — exactly as its log then revealed.

**Fix:** the stty spawn is GONE. `SerialLink::configure_raw()`
configures the tty natively via libc — `tcgetattr` → `cfmakeraw` →
`CREAD|CLOCAL` → `VMIN=1/VTIME=0` → `cfsetispeed/cfsetospeed(B115200)`
→ `tcsetattr(TCSANOW)` — the exact semantics of the old invocation,
minus the external process. A configure failure now fails the open
(fast, honest retry, carried in the open report) instead of serving
a cooked-mode link. The crate already links libc; the agent's tty
path now spawns zero child processes on any OpenWrt target. (The
ssh:// remote relay still uses router-side stty by design — that is
the pcscd-side path, documented separately.)

**Regression test** (`tty_configured_natively_without_stty`):
a PATH containing ONLY a booby-trapped `stty` (records its own
execution, no other binaries reachable) — the open must succeed, NO
stty execution may occur (marker absent), the open report must show
`native termios ok`, and a fresh probe `tcgetattr` on the pty must
show the raw state actually applied (ICANON off, ECHO off, ICRNL
off, OPOST off, CS8, VMIN=1/VTIME=0, B115200). A/B-verified:
reinserting an `stty` spawn into `open_serial` makes this test fail
with "the tty path spawned stty"; the native build passes 14/14.

Suite: agent 14/14; workspace 49 passed / 4 pre-existing
logging-family failures (identical on pristine `c3e8096`, untouched).

## Iteration 5 — SCardConnect follow-up (2026-11-21, round 5, deployed sha 4469435d)

Live state after round 4: agent connects, stable ESTABLISHED session,
answers every presence probe (ATR), ON/OFF handled — but PC/SC
`SCardConnect` still fails "no smart card" (one attempt reached
"Sharing violation" = a live card context existed from enumeration).
Owner hypothesis: the driver expects a reply to ON.

**Verdict on ON — refuted by driver source and both proven clients:**

- upstream `vicc_poweron()` is SEND-ONLY (vpcd.c @ the pinned
  `93286a9`): it never reads a reply; the amperstrand patch does not
  touch it. The only reads in the whole driver live in
  `vicc_transmit` — i.e. after `vicc_getatr` (the ATR control the
  driver itself sends) and after APDUs.
- `IFDHPowerICC(IFD_POWER_UP)` = send ON (no read) + immediately
  `IFDHGetCapabilities(TAG_IFD_ATR)` = send ATR control + read one
  frame. So the wire shape of a client connect is the burst
  `[ON][ATR]`, answered by exactly ONE frame (the ATR).
- both bench-proven vpicc clients stay silent on ON (ESP32 vpcd.rs:
  `VPCD_CTRL_ON => None // fire-and-forget`; VpcdBridge.java:
  "OFF/ON: no reply").
- Answering ON would be actively harmful: the ON-reply would be
  consumed by the following getatr's read, and every subsequent
  answer lands one request late — a PERMANENT one-deep skew.

**Real finding this round: the agent answered RESET — same skew
class.** `vicc_reset()` is send-only too, and `IFDHPowerICC(IFD_RESET)`
always follows with its own getatr. The agent (and, it turns out,
both proven clients) answered RESET with an ATR; with this driver
that leaves a stale frame in the socket queue shifting every later
read one request late — invisible while all answers are ATRs (why
the proven benches never noticed), fatal the moment a C-APDU's
R-APDU is silently replaced by the stale ATR frame. Fixed: RESET is
now no-reply (aligned to the driver's actual read pattern); the ATR
answer the driver wants after a reset comes from the driver's own
follow-up ATR request. Log lines split (ATR / RESET / APDU now
distinct — round-6 live logs will discriminate).

**Regression:** `pcscd_activation_sequence_stays_frame_aligned` —
replays the strace-verified pcscd burst shapes (`[OFF][ON][ATR]`,
`[RESET][ATR]` as single writes; see ccid-firmware-rs
TRANSMIT_DEAD_WINDOW.md) and asserts: exactly ONE frame per burst
(the ATR), silence after, and the NEXT APDU's answer is the modem's
R-APDU, not a stale ATR. A/B-verified: making RESET answer again
fails this test.

**ATR well-formedness checked and cleared** (a malformed ATR would
have explained presence-yes/connect-no): FAKE_ATR
(3B 9F 96 80 1F 47 … 73) decodes as a structurally valid T=0 ATR
(TA1=96, TA2=1F, K=15 historical, no TCK needed).

**Remaining suspicion for round 6 (pcscd/driver-side, documented in
the sibling repo):** TRANSMIT_DEAD_WINDOW.md pins a stall class
inside pcscd + libifdvpcd where the card-present path works while
the transmit/connect path does not route — observed there after
pcscd restarts (~2 min self-heal), with their conftest needing
retries + real-connect (never GetStatusChange) against real pcscd.
The likely aggravator on our bench: pcscd added the reader while the
agent was not yet connected → the add-time POWER_UP failed hard
(IFD_COMMUNICATION_ERROR) → pcscd's reader context may not re-power
without a card-removal/insertion transition. Operational
discriminators for the next live test: (a) restart pcscd only AFTER
the agent is connected and stable (add-time POWER_UP then succeeds
against a live VICC); (b) retry SCardConnect for up to ~2 min;
(c) use real SCardConnect, not GetStatusChange, when checking. If
(a) fixes it, the durable cure is their candidate driver-side change
("on VICC reconnect, force a full IFD re-init") — owner-gated.

Suite: agent 15/15 (new activation-sequence regression); workspace
unchanged (49 pass + the 4 pre-existing logging failures).
