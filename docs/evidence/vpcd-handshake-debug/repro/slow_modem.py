#!/usr/bin/env python3
"""slow_modem.py — AT-dialect modem mock with per-command pre-answer delays.

The NR7101 shape for the vpcd handshake repro: every exchange succeeds,
but SLOWLY. The two `init_handshake` commands (`ATE1`, `AT+CSIM=?`) each
answer after a configurable delay — under each exchange deadline in the
agent's AT engine (2 s / 2.5 s) but together past the driver's 2 s answer
window. This is exactly the "slow-but-healthy tty" that lost the ATR race
on the bench.

Dialect mirrors tests/hardware/mock_modem.py (echo discipline, strict
+CSIM framing); APDUs answer the canned 9000.

Usage:
  slow_modem.py [--listen 127.0.0.1:17002] [--ate1-delay-ms 1500]
                [--csim-test-delay-ms 1500]

Zero real-AT side effects: loopback only.
"""

from __future__ import annotations

import argparse
import re
import socket
import sys
import threading
import time

CSIM_RE = re.compile(r'^AT\+CSIM=(\d+),"([0-9A-Fa-f]*)"$')


def reply_lines(line: str) -> list[str] | None:
    if line in ("AT", "ATE0", "ATE1"):
        return []
    if line == "AT+CSIM=?":
        return []  # bare OK, no range line (README modem finding)
    if line == "AT+CPIN?":
        return ["+CPIN: READY"]
    m = CSIM_RE.match(line)
    if m:
        declared, hexs = int(m.group(1)), m.group(2)
        if declared != len(hexs) or len(hexs) % 2:
            return ["ERROR"]
        return ['+CSIM: 4,"9000"']
    if line.startswith("AT"):
        return ["ERROR"]
    return None


def handle(sock: socket.socket, ate1_ms: int, csim_test_ms: int) -> None:
    buf = b""
    try:
        while True:
            chunk = sock.recv(4096)
            if not chunk:
                return
            buf += chunk
            while b"\r" in buf:
                raw, buf = buf.split(b"\r", 1)
                line = raw.lstrip(b"\n").decode("ascii", "replace").strip()
                if not line:
                    continue
                delay_ms = 0
                if line == "ATE1":
                    delay_ms = ate1_ms
                elif line == "AT+CSIM=?":
                    delay_ms = csim_test_ms
                if delay_ms:
                    time.sleep(delay_ms / 1000.0)
                # ATE1 echo discipline
                sock.sendall(b"\r\n" + line.encode() + b"\r\n")
                resp = reply_lines(line)
                if resp is None:
                    continue
                for l in resp:
                    sock.sendall(b"\r\n" + l.encode() + b"\r\n")
                if resp and resp[-1] == "ERROR":
                    continue
                sock.sendall(b"\r\n\r\nOK\r\n")
    except OSError:
        return


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--listen", default="127.0.0.1:17002")
    ap.add_argument("--ate1-delay-ms", type=int, default=1500)
    ap.add_argument("--csim-test-delay-ms", type=int, default=1500)
    args = ap.parse_args()

    host, _, port = args.listen.rpartition(":")
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind((host or "127.0.0.1", int(port)))
    srv.listen(8)
    print("slow_modem listening on {} (ATE1 +{}ms, AT+CSIM=? +{}ms)".format(
        args.listen, args.ate1_delay_ms, args.csim_test_delay_ms), flush=True)
    try:
        while True:
            sock, _ = srv.accept()
            threading.Thread(
                target=handle,
                args=(sock, args.ate1_delay_ms, args.csim_test_delay_ms),
                daemon=True,
            ).start()
    except KeyboardInterrupt:
        pass
    finally:
        srv.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
