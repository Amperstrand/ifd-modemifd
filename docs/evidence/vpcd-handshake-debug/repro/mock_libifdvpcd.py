#!/usr/bin/env python3
"""mock_libifdvpcd.py — the amperstrand-patched libifdvpcd listener as a machine.

Offline reproduction of the 2026-11-21 NR7101 vpcd handshake failure
(docs/evidence/vpcd-handshake-debug/README.md). This models the HOST side:
the pcscd IFD driver (upstream vpcd.c @ 93286a9 + the #326 amperstrand
patch) whose listener the router-side modem-vpicc agent connects OUT to.

Faithful behaviors (all verified against the fetched upstream source and
tests/hardware/host/libifdvpcd-amperstrand.patch in ccid-firmware-rs):

  * accept is POLL-DRIVEN, not backlog-driven: pcscd's presence poll calls
    vicc_connect() -> waitforclient(poll, 0 ms) + accept() every
    PCSCLITE_STATUS_POLL_RATE (~400 ms; --poll-ms here). A vpicc TCP
    connect completes into the kernel backlog immediately even though the
    application has not accepted yet (listen(sock, 0) still holds one).
  * immediately after accept, the ATR control (vpcd.h VPCD_CTRL_ATR = 4)
    is sent and ONE answer is expected within VPCD_RECV_TIMEOUT_SEC = 2 s
    (--window-ms): the patch sets SO_RCVTIMEO on the accepted socket
    because upstream recvall() blocks forever (the pcscd-EH wedge).
  * window expired  -> recvall() = -1 -> patched vicc_transmit() ejects
    on r < 0: close(client_sock). The LISTENER persists; the next poll
    accepts the reconnecting vpicc. This close is the FIN the host shows
    in FIN-WAIT-2 while the agent still holds its end.
  * zero-length answer -> "ICC absent", no eject (patch fix 1);
    EOF/RST -> peer_closed, client dropped.

Event log: one JSON object per line to --log (or stdout). Final summary
line: {"summary": {accepts, answered, ejects, peer_closed, latencies_ms}}.

Zero real pcscd involvement: loopback only.
"""

from __future__ import annotations

import argparse
import json
import socket
import sys
import time

CTRL_OFF = 0x00
CTRL_ON = 0x01
CTRL_RESET = 0x02
CTRL_ATR = 0x04


def recv_frame_deadline(sock: socket.socket, deadline: float) -> tuple[bytes | None, bool]:
    """Read one vpcd frame (2-byte BE length + payload).

    Returns (payload, timed_out). payload is None on EOF/RST.
    """
    buf = b""
    while len(buf) < 2:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            return None, True
        sock.settimeout(remaining)
        try:
            chunk = sock.recv(2 - len(buf))
        except socket.timeout:
            return None, True
        except OSError:
            return None, False
        if not chunk:
            return None, False
        buf += chunk
    n = int.from_bytes(buf, "big")
    payload = b""
    while len(payload) < n:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            return None, True
        sock.settimeout(remaining)
        try:
            chunk = sock.recv(n - len(payload))
        except socket.timeout:
            return None, True
        except OSError:
            return None, False
        if not chunk:
            return None, False
        payload += chunk
    return payload, False


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--listen", default="127.0.0.1:35963",
                    help="host:port to listen on (the vpcd slot)")
    ap.add_argument("--poll-ms", type=int, default=100,
                    help="presence-poll cadence (real pcscd: ~400)")
    ap.add_argument("--window-ms", type=int, default=2000,
                    help="VPCD_RECV_TIMEOUT_SEC answer window (real: 2000)")
    ap.add_argument("--duration-s", type=float, default=8.0,
                    help="run this long, then print the summary and exit")
    ap.add_argument("--log", default=None, help="JSONL event log (default stdout)")
    args = ap.parse_args()

    logf = open(args.log, "a", encoding="utf-8") if args.log else sys.stdout

    def log(ev: dict) -> None:
        logf.write(json.dumps({"t": round(time.time(), 3), **ev}) + "\n")
        logf.flush()

    host, _, port = args.listen.rpartition(":")
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind((host or "127.0.0.1", int(port)))
    srv.listen(0)  # upstream opensock(): backlog 0 by design
    srv.settimeout(args.poll_ms / 1000.0)
    print("mock_libifdvpcd listening on {} (poll={}ms window={}ms)".format(
        args.listen, args.poll_ms, args.window_ms), flush=True)

    accepts = 0
    answered = 0
    ejects = 0
    peer_closed = 0
    latencies: list[int] = []

    end_at = time.monotonic() + args.duration_s
    while time.monotonic() < end_at:
        conn: socket.socket | None = None
        # presence poll: accept a pending vpicc if we have no client
        while conn is None and time.monotonic() < end_at:
            try:
                conn, _ = srv.accept()
            except socket.timeout:
                continue
            accepts += 1
            log({"event": "accept", "conn": accepts})
        if conn is None:
            break
        conn.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        # probe loop on the accepted client (vicc_present -> vicc_getatr)
        while time.monotonic() < end_at:
            t0 = time.monotonic()
            try:
                conn.sendall(bytes([0, 1, CTRL_ATR]))
            except OSError:
                peer_closed += 1
                log({"event": "peer_closed", "conn": accepts})
                break
            payload, timed_out = recv_frame_deadline(conn, t0 + args.window_ms / 1000.0)
            if payload is not None and not timed_out:
                answered += 1
                latencies.append(round((time.monotonic() - t0) * 1000))
                log({"event": "answered", "conn": accepts,
                     "latency_ms": latencies[-1], "len": len(payload)})
            elif timed_out:
                ejects += 1  # patched vicc_transmit: r < 0 -> vicc_eject
                log({"event": "eject", "conn": accepts,
                     "reason": "answer window expired"})
                break
            else:
                peer_closed += 1
                log({"event": "peer_closed", "conn": accepts})
                break
            time.sleep(args.poll_ms / 1000.0)
        try:
            conn.close()  # vicc_eject / drop — the listener persists
        except OSError:
            pass
        time.sleep(args.poll_ms / 1000.0)

    summary = {"summary": {
        "accepts": accepts, "answered": answered, "ejects": ejects,
        "peer_closed": peer_closed, "latencies_ms": latencies}}
    logf.write(json.dumps(summary) + "\n")
    logf.flush()
    if args.log:
        logf.close()
    srv.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
