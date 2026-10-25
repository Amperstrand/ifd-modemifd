#!/usr/bin/env python3
"""mock_c0_responder — PTY-backed fake Quectel-class modem for rehearsing
tests/hardware/at_csim_c0_battery.sh without hardware.

Scenario selectable via argv[1]:
  c0-works : GET RESPONSE (INS C0) on the allocated channel returns data+9000
             -> battery verdict must be ALLOCATED-CHANNEL-OK, exit 0
  c0-fails : C0 returns 6D00 (session-state class)
             -> verdict must be ALLOCATED-CHANNEL-FAIL(...6D00...), exit 1

Wire facts it simulates (findings lineage, RG502Q-EA/AACR class):
  * MANAGE CHANNEL open succeeds twice (ch 02, ch 03), then 6A81 exhaustion
  * MANAGE CHANNEL close: unconditional AT-level ERROR (known leak class)
  * ISD-R SELECT by AID (P2=0C) on an allocated CLA: 61 21
  * inline-FCI SELECT (P2=00, Le=00): 6A82
  * basic channel: SELECT MF 9000; SELECT ICCID by FID (P2=0C) 9000 (no 61xx)
  * command echo precedes every response (the battery parses echo-correlated)

Usage: mock_c0_responder.py <scenario> <pty_master_fd>
The battery runs against the slave side: TTY=<slave> sh at_csim_c0_battery.sh <slave> 2
"""
import os
import sys
import time

ISDR = "A0000005591010FFFFFFFF8900000100"
C0_DATA = "A00A0000591010FFFFFFFF890000010000"  # plausible FCI-ish payload body


def respond(w, line, scenario, state):
    # echo first (echo-correlated parsing)
    w.write(f"\r\n{line}\r\n")
    w.flush()

    if scenario == "case-tolerant":
        # tolerant modem: accepts lowercase PDUs (compare pySim 8e15c18's
        # rejecting EM7565, which is the c0-works/c0-fails default here)
        pass  # matching below is case-insensitive when we uppercase the PDU
    if not line.startswith("AT"):
        return
    if line == "AT":
        w.write("\r\nOK\r\n")
    elif line == "AT+CPIN?":
        w.write("\r\n+CPIN: READY\r\n\r\nOK\r\n")
    elif line.startswith('AT+CSIM=14,"00A40000023F00"'):
        w.write('\r\n+CSIM: 4,"9000"\r\n\r\nOK\r\n')
    elif line.upper().startswith('AT+CSIM=14,"00A4000C022FE2"') if scenario == "case-tolerant" else line.startswith('AT+CSIM=14,"00A4000C022FE2"'):
        # P2=0C "no data returned": plain 9000, no 61xx
        w.write('\r\n+CSIM: 4,"9000"\r\n\r\nOK\r\n')
    elif line.startswith('AT+CSIM=10,"00B000000A"'):
        w.write('\r\n+CSIM: 24,"984500000000000000109000"\r\n\r\nOK\r\n'.replace('" + "', '"'))
    elif line.startswith('AT+CSIM=8,"0070800'):
        # MANAGE CHANNEL close: AT-level refusal (leak class) — plain ERROR
        w.write("\r\nERROR\r\n")
    elif line.startswith('AT+CSIM=10,"0070000001"'):
        n = state["opens"]
        state["opens"] += 1
        if n == 0:
            w.write('\r\n+CSIM: 6,"029000"\r\n\r\nOK\r\n')
        elif n == 1:
            w.write('\r\n+CSIM: 6,"039000"\r\n\r\nOK\r\n')
        else:
            w.write('\r\n+CSIM: 6,"006A81"\r\n\r\nOK\r\n')
    elif "A4040C10" + ISDR in line:
        # ISD-R SELECT on whatever CLA the battery computed: 61 21
        w.write('\r\n+CSIM: 4,"6121"\r\n\r\nOK\r\n')
    elif "A4040010" + ISDR + '00"' in line:
        # inline-FCI form: 6A82
        w.write('\r\n+CSIM: 4,"6A82"\r\n\r\nOK\r\n')
    elif "C0000021" in line:
        # THE decisive cell: GET RESPONSE INS C0, Le=0x21
        if scenario == "c0-works":
            payload = C0_DATA + "9000"
            w.write(f'\r\n+CSIM: {len(payload)},"{payload}"\r\n\r\nOK\r\n')
        else:
            w.write('\r\n+CSIM: 4,"6D00"\r\n\r\nOK\r\n')
    else:
        w.write("\r\nERROR\r\n")
    w.flush()


def main():
    scenario = sys.argv[1] if len(sys.argv) > 1 else "c0-works"
    master_fd = int(sys.argv[2])
    os.set_blocking(master_fd, False)
    w = os.fdopen(master_fd, "w")
    state = {"opens": 0}
    buf = b""
    deadline = time.time() + 120
    while time.time() < deadline:
        try:
            chunk = os.read(master_fd, 4096)
            if chunk:
                buf += chunk
                while b"\r" in buf:
                    raw, _, buf = buf.partition(b"\r")
                    line = raw.decode(errors="replace").strip()
                    if line:
                        respond(w, line, scenario, state)
        except BlockingIOError:
            time.sleep(0.05)
    sys.exit(0)


if __name__ == "__main__":
    main()
