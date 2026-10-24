# VPCD architecture for the modem SIM reader (work-package spec)

Decision (2026-10-24, owner-directed): the standard vsmartcard **vpcd**
architecture becomes the primary path for exposing the NR7101 modem SIM
as a PC/SC reader. Goals in priority order:

1. **Reliable** — the provisioning one-shot must never lose a transport
   mid-BPP (the quantified failure: one drop = one burned activation
   code; Oct 23 evidence: 4 futile TTL-Expired retries after a single
   mid-load card drop).
2. **Secure** — least exposure, documented trust model.
3. **Reasonably performant** — pacing-floor throughput is sufficient;
   maximum performance is explicitly NOT a goal (the modem's own CSIM
   pacing floor dominates anyway).

## Architecture

    HOST (all local PC/SC, standard parts):        ROUTER (local to the modem):
      lpac → pcscd → libifdvpcd-amperstrand.so       modem-vpicc agent (Rust, musl)
                  ↖ vpcd protocol over LAN TCP  ←──   → AT+CSIM → SIM
                    (agent connects OUT; vpcd         stty raw setup + File tty I/O,
                     listens on a bound interface)    echo-correlation, 250 ms pacing,
                                                     strict CSIM parse, drop filters

Why: pcscd stays fully local (owner requirement); the modem AT discipline
runs router-side where the tty is local (the entire remote-tty failure
class of 2026-10-24 — relay lifecycle, byte-theft, stty/baud/VMIN quirks,
env stripping — is structural, not incidental); every component on the
host side is either a standard (vpcd) or already-proven Amperstrand
infrastructure (libifdvpcd-amperstrand.so, built and benched in the
ccid-firmware-rs campaign).

## Wire protocol (reference: org.amperstrand.VpcdBridge, bench-proven)

- Frame = 2-byte big-endian length + payload.
- 1-byte control payloads:
  - `0x04` (RESET), `0x02` (ATR request) → reply one ATR frame;
  - `0x00` (OFF) → **NO reply** (an empty ack de-syncs the driver's next
    ATR probe — bench-verified); deselect semantics.
- Length > 1 = C-APDU → reply one R-APDU frame (or status-word-only).
- The agent is the TCP **client** and reconnects every ~500 ms on error;
  a vpcd slot is single-client (never probe the port while owned).
- Operational: after a pcscd restart the vpcd slot answers the ATR probe
  immediately but C-APDU transmits can stay empty for ~2 min — tests
  must not fail-fast a fresh instance (TRANSMIT_DEAD_WINDOW.md in the
  sibling repo).

## Security requirements

- Agent opens NO listening sockets (connects out only).
- vpcd listener binds a chosen interface (bench LAN), not `*`; a host
  firewall rule admits only the router's address on that port. The
  bench-LAN plaintext trust domain is documented; anything beyond the
  bench requires a tunnel (single, standard place to add it).
- Agent participates in the router-side `modem-lane` lock and holds the
  per-tty `flock` for its lifetime (the 2026-10-24 byte-theft lesson,
  encoded in tooling).
- Logging matches the driver doctrine: opt-in, 0600, no-symlink, APDU
  content behind its own opt-in (v3.4.1 semantics).
- No new packages on the router beyond what is disclosed in the tracker
  (currently: GNU stty 9.7, modem-lane helper; the agent binary itself
  lives on flash, ~1–2 MB, deployed via scp with recorded sha256).

## Migration / sequencing

1. Frame codec + unit tests in this crate (std-only; same conventions).
2. `modem-vpicc` agent binary in this crate (`[[bin]]`), reusing
   `at.rs`/`reader.rs` logic; `--target tcp://h:p` mode for mock tests,
   `--target tty:/dev/ttyUSBx` for the router (stty subprocess + File
   I/O keeps the crate std-only).
3. Host: build libifdvpcd-amperstrand.so (sibling script), reader entry
   (`DEVICENAME /dev/null:0x8C7B`, `CHANNELID <port>`), lane-runner vpcd
   mode reusing the namespace/env-injection/lock machinery.
4. Cross-compile `mipsel-unknown-linux-musl`, deploy under procd,
   mock-vpcd smoke on-loopback, then live on the Telenor card.
5. The deferred stress block (sacrificial suite, soak, concurrency,
   kill -9, lane bounce, lpac behavior) runs unchanged against the vpcd
   reader — the suite investment carries over verbatim.
6. Docs: vpcd primary; TCP legacy (proven) stays; ssh:// becomes the
   secondary/secondary-bench path (v3.5.2-fixed; see its 2026-10-24
   defect ledger before relying on it).

## Acceptance criteria (provisioning readiness)

- [ ] 48 h of mock-vpcd loopback soak: zero protocol desyncs, zero
      leaked processes on either side, reconnect after every induced
      failure recovers within one reconnect interval.
- [ ] Live Telenor round: sacrificial suite 9/9 through the vpcd reader;
      SAC_REPEATS=50 soak green; kill -9 of any single component (agent,
      lane pcscd, client) recovers without operator action.
- [ ] lpac chip info against the Telenor card: clean failure class, lane
      healthy afterward; against the 9eSIM (post switch-back, sanctioned
      window): EID read stable ×3.
- [ ] Provisioning rehearsal: full BPP-sized load through the reader
      with zero transport drops (mock SM-DP+ or replay), only then the
      real one-shot.
- [ ] Security checklist items above verified and recorded.
