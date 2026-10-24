#!/usr/bin/env python3
"""Fake modem relay for SSH-transport tests.

Speaks just enough AT on stdin/stdout to exercise the SshLink path
(injected via IFD_MODMIFD_SSH_CMD="python3 tests/fake_relay.py"): the
driver spawns this process instead of ssh and talks to it over pipes.
The relay ignores its argv (the ssh-style options appended by
ssh_command_for) and models the bench modem's behavior:

- ATE1 / AT+CSIM=?   -> echo + OK (handshake)
- AT+CPIN?           -> echo + "+CPIN: READY" + OK (presence)
- AT+CSIM=...        -> echo + "+CSIM: 4,\"9000\"" + OK (normal APDU)
- AT+CSIM with the magic 4242... payload -> echo + ERROR (refusal)
- AT+SLEEP           -> echo, then silence (post-echo refusal)

Line-buffered input, flushed output — pipe semantics, no tty.
"""

import sys


def respond(out, line: str) -> None:
    if line.startswith("AT+SLEEP"):
        out.write(f"\r\n{line}\r\n")
        out.flush()
        return  # heard us, never answers
    out.write(f"\r\n{line}\r\n")
    if line == "ATE1" or line.startswith("AT+CSIM=?"):
        out.write("\r\nOK\r\n")
    elif line.startswith("AT+CPIN?"):
        out.write("\r\n+CPIN: READY\r\n\r\nOK\r\n")
    elif line.startswith("AT+CSIM="):
        if '"4242' in line or line.startswith("AT+CSIM=4,\"4242\""):
            out.write("\r\nERROR\r\n")
        else:
            out.write("\r\n+CSIM: 4,\"9000\"\r\n\r\nOK\r\n")
    else:
        out.write("\r\nOK\r\n")
    out.flush()


def main() -> int:
    buf = ""
    while True:
        chunk = sys.stdin.read(1)
        if not chunk:
            return 0
        if chunk == "\r":
            if buf:
                respond(sys.stdout, buf)
            buf = ""
        elif chunk != "\n":
            buf += chunk


if __name__ == "__main__":
    raise SystemExit(main())
