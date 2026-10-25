#!/usr/bin/env python3
"""mock_modem.py — scriptable mock Quectel-class AT modem with a seated fake SIM.

Dry-run backend for the modem-hil sacrificial-SIM suites (validate-sacrificial.sh,
validate-esimgate.sh). Reuses the mock-modem pattern proven in tests/mock_at.rs:
one TCP connection per driver session, ATE1 echo discipline, strict
`+CSIM: <len>,"HEX"` framing, and — the RG502Q-EA fingerprint from the repo
README "Modem findings" — a BASIC-channel filter that silently drops

  * SELECT by DF-name/AID  (A4, P1=04)   on channel 0
  * READ BINARY            (B0)          on channel 0

after echoing the AT line, leaving the client in echo-then-silence (the
driver classifies this Refused -> SW 6F00). The same APDUs on logical
channels pass instantly. `--filter none` models a firmware without the
filter (drift check for the fingerprint cases).

Fake SIM (per connection, no persistence):
  * SELECT by FID (P1=00): 3F00 (MF) and 2FE2 (EF.ICCID) answer 9000;
    anything else 6A82. P2=00 gets a minimal FCI, P2=0C gets no data.
  * READ BINARY on a logical channel with 2FE2 selected returns the fixed
    10-byte mock ICCID 89450000000000000001 (deterministic fixture
    fingerprint); without a selection it answers 6982.
  * MANAGE CHANNEL open/close manages channels 1..3.
  * SELECT by DF-name on a logical channel: the GSMA SGP.22 ISD-R AID
    (A0000005 591010 FFFFFFFF 89000001 00) answers 9000 (mock models a
    healthy eUICC); any other AID 6A82.

Other AT traffic: AT / ATE0 / ATE1 -> OK, AT+CSIM=? -> bare OK (per README:
test commands answer bare OK, no range line), AT+CPIN? -> +CPIN: READY,
AT+CGMM -> mock model string, unknown AT+<cmd> -> ERROR.

Every exchange is logged as one JSON object per line to --log (or stdout).

Usage:
  mock_modem.py [--listen 127.0.0.1:17002] [--log FILE] [--filter basic|none]
                [--chaos-urc]

Zero real-AT side effects: listens on loopback only, never dials out.
"""

from __future__ import annotations

import argparse
import json
import re
import socket
import sys
import threading
import time

ISD_R_AID = bytes.fromhex("A0000005591010FFFFFFFF8900000100")
MOCK_ICCID = bytes.fromhex("89450000000000000001")
MOCK_MODEL = "RG502Q-EA-MOCK"
MAX_CHANNEL = 3  # logical channels 1..3

CSIM_RE = re.compile(r'^AT\+CSIM=(\d+),"([0-9A-Fa-f]*)"$')


class ConnState:
    """Per-connection fake-SIM state (channels, selected EFs)."""

    def __init__(self) -> None:
        self.free_channels = list(range(1, MAX_CHANNEL + 1))
        # channel -> selected EF file bytes (None = nothing selected)
        self.selected: dict[int, bytes | None] = {0: None}

    def handle_apdu(self, apdu: bytes, basic_filter: bool) -> bytes | None:
        """Return the R-APDU, or None to model a modem-level silent drop."""
        if len(apdu) < 4:
            return bytes.fromhex("6700")
        cla, ins, p1, p2 = apdu[0], apdu[1], apdu[2], apdu[3]
        channel = cla & 0x07
        lc = apdu[4] if len(apdu) > 4 else 0
        data = apdu[5 : 5 + lc]

        if basic_filter and channel == 0:
            if ins == 0xA4 and p1 == 0x04:
                return None  # dropped by the modem, never reaches the SIM
            if ins == 0xB0:
                return None  # dropped by the modem, never reaches the SIM

        if ins == 0xA4 and p1 == 0x00:  # SELECT by FID
            fid = data.hex().upper()
            if fid == "3F00":
                self.selected[channel] = None
                return self._select_resp("3F00", p2)
            if fid == "2FE2":
                self.selected[channel] = MOCK_ICCID
                return self._select_resp("2FE2", p2)
            return bytes.fromhex("6A82")

        if ins == 0xA4 and p1 == 0x04:  # SELECT by DF-name (logical only here)
            if data == ISD_R_AID:
                self.selected[channel] = None
                return self._select_resp("2FE2", p2, isd_r=True)
            return bytes.fromhex("6A82")

        if ins == 0x70:  # MANAGE CHANNEL (ISO 7816-4: open P1=00 P2=00, close P1=80 P2=ch)
            if p1 == 0x00 and p2 == 0x00:  # open
                if not self.free_channels:
                    return bytes.fromhex("6A81")
                ch = self.free_channels.pop(0)
                self.selected[ch] = None
                return bytes([ch]) + b"\x90\x00"
            if p1 == 0x80:  # close; P2 names the channel to close
                target = p2
                if target not in range(1, MAX_CHANNEL + 1) or target not in self.selected:
                    return bytes.fromhex("6A88")
                if target in self.free_channels:
                    return bytes.fromhex("6A88")  # not open
                del self.selected[target]
                self.free_channels.append(target)
                self.free_channels.sort()
                return b"\x90\x00"
            return bytes.fromhex("6A86")

        if ins == 0xB0:  # READ BINARY (only reachable on logical channels w/ filter)
            sel = self.selected.get(channel)
            if sel is None:
                return bytes.fromhex("6982")
            off = (p1 << 8) | p2
            le = apdu[4] if len(apdu) > 4 else len(sel)
            chunk = sel[off : off + (le if le else 256)]
            return chunk + b"\x90\x00"

        return bytes.fromhex("6D00")  # INS not supported

    @staticmethod
    def _select_resp(fid_hex: str, p2: int, isd_r: bool = False) -> bytes:
        if p2 & 0x0C:  # no response data requested
            return b"\x90\x00"
        # minimal FCI: 6F 04 84 02 <2-byte name> (ISD-R: its AID head)
        name = ISD_R_AID[:2].hex().upper() if isd_r else fid_hex.zfill(4)
        fci = bytes.fromhex("6F048402") + bytes.fromhex(name)
        return fci + b"\x90\x00"


def process_line(state: ConnState, line: str, cfg: dict) -> list[str] | None:
    """Map one AT line to response lines; None = echo-only (silent drop)."""
    if line == "AT" or line in ("ATE0", "ATE1"):
        return []
    if line == "AT+CSIM=?":
        return []  # bare OK, no range line (README modem finding)
    if line == "AT+CPIN?":
        return ["+CPIN: READY"]
    if line == "AT+CGMM":
        return [MOCK_MODEL]
    m = CSIM_RE.match(line)
    if not m:
        return ["ERROR"] if line.startswith("AT") else None
    declared, hexs = int(m.group(1)), m.group(2)
    if declared != len(hexs) or len(hexs) % 2:
        return ["ERROR"]
    if cfg.get("case_sensitive") and any(c in "abcdef" for c in hexs):
        # Qualcomm-class lowercase rejection (pySim gerrit 8e15c18 /
        # lpac #428 fingerprint): lowercase CSIM payload -> bare ERROR.
        return ["ERROR"]
    rapdu = state.handle_apdu(bytes.fromhex(hexs), cfg["filter"] == "basic")
    if rapdu is None:
        return None  # basic-channel drop: echo, then silence
    out = ["+CSIM: {},\"{}\"".format(len(rapdu) * 2, rapdu.hex().upper())]
    if cfg["chaos_urc"]:
        out = ["+CPIN: READY"] + out  # tolerated URC between echo and answer
    return out


def handle_conn(sock: socket.socket, addr, cfg: dict, log_lock: threading.Lock) -> None:
    state = ConnState()
    buf = b""

    def log(ev: dict) -> None:
        if cfg["logf"] is not None:
            with log_lock:
                cfg["logf"].write(json.dumps({"t": time.time(), "peer": addr[0], **ev}) + "\n")
                cfg["logf"].flush()

    try:
        while True:
            chunk = sock.recv(4096)
            if not chunk:
                break
            buf += chunk
            while b"\r" in buf:
                raw, buf = buf.split(b"\r", 1)
                line = raw.lstrip(b"\n").decode("ascii", "replace").strip()
                if not line:
                    continue
                log({"dir": "tx", "line": line})
                # ATE1 echo discipline (tests/mock_at.rs pattern)
                sock.sendall(b"\r\n" + line.encode() + b"\r\n")
                resp = process_line(state, line, cfg)
                if resp is None:
                    log({"dir": "rx", "line": "<DROP: echo only, no answer>"})
                    continue  # silent drop: nothing after the echo
                for l in resp:
                    sock.sendall(b"\r\n" + l.encode() + b"\r\n")
                    log({"dir": "rx", "line": l})
                if resp and resp[-1] == "ERROR":
                    continue  # ERROR is a final result code: no trailing OK
                sock.sendall(b"\r\n\r\nOK\r\n")
                log({"dir": "rx", "line": "OK"})
    except OSError:
        pass
    finally:
        try:
            sock.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        sock.close()


def serve(listen: str, cfg: dict) -> None:
    host, _, port = listen.rpartition(":")
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind((host or "127.0.0.1", int(port)))
    srv.listen(8)
    print("mock_modem listening on {} (filter={}, chaos_urc={})".format(
        listen, cfg["filter"], cfg["chaos_urc"]), flush=True)
    log_lock = threading.Lock()
    try:
        while True:
            sock, addr = srv.accept()
            threading.Thread(
                target=handle_conn, args=(sock, addr, cfg, log_lock), daemon=True
            ).start()
    except KeyboardInterrupt:
        pass
    finally:
        srv.close()


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--listen", default="127.0.0.1:17002",
                    help="host:port to listen on (default 127.0.0.1:17002)")
    ap.add_argument("--log", default=None, help="JSONL exchange log file (default stdout)")
    ap.add_argument("--filter", choices=["basic", "none"], default="basic",
                    help="basic = RG502Q basic-channel drop filter (default), "
                         "none = filter disabled (drift check)")
    ap.add_argument("--chaos-urc", action="store_true",
                    help="interleave a +CPIN: READY URC before each +CSIM answer")
    ap.add_argument("--case-sensitive", action="store_true",
                    help="reject lowercase CSIM payloads with bare ERROR "
                         "(Qualcomm-class parser fingerprint)")
    args = ap.parse_args()

    logf = open(args.log, "a", encoding="utf-8") if args.log else sys.stdout
    cfg = {"filter": args.filter, "chaos_urc": args.chaos_urc,
           "case_sensitive": args.case_sensitive, "logf": logf}
    serve(args.listen, cfg)
    return 0


if __name__ == "__main__":
    sys.exit(main())
