#!/usr/bin/env python3
"""suite.py — case runner for the modem-hil sacrificial-SIM validation suites.

Two suites, both strictly READ-ONLY (class-A: SELECT by FID, logical-channel
EF reads, channel lifecycle, public-AID select, lpac chip info):

  sacrificial  — zero-risk transport qualification of a disposable SIM
  esimgate     — pre-provisioning read-only gate (ISD-R readiness, EID,
                 chipInfo) for a eUICC that is NOT the production 9eSIM

Backends:
  direct — raw AT+CSIM over TCP (mock_modem.py or a socat bridge), mirroring
           the driver's v3.4 transport semantics: echo-correlated strict
           +CSIM parse; echo-then-silence => SW 6F00 (modem drop / refusal);
           no-echo => transport error (case BLOCKED, never a fake verdict).
  pcsc   — ctypes libpcsclite (honors PCSCLITE_CSOCK_NAME). The reader MUST
           be named with --reader and must match exactly one reader; the
           first reader is NEVER implicitly selected.

CARD SAFETY (session-2 directive: the 9eSIM must never be a target):
  any execution that can reach a real card (pcsc backend, or direct against
  a non-loopback endpoint) requires --card-decl; values containing "9esim"
  are refused outright, and the sacrificial suite additionally requires
  --card-decl sacrificial. Dry-run against the loopback mock needs nothing.

Output: JSONL case records for report.py; exit 0 all-PASS, 1 any FAIL,
2 BLOCKED/SKIP, 3 usage/infra error.
"""

from __future__ import annotations

import argparse
import ctypes
import json
import os
import re
import socket
import subprocess
import sys
import threading
import time
from pathlib import Path

ISD_R_AID = "A0000005591010FFFFFFFF8900000100"
LOOPBACK = {"127.0.0.1", "::1", "localhost"}
CSIM_RE = re.compile(r'^\+CSIM:\s*(\d+),"([0-9A-Fa-f]*)"\s*$')
PACE_S = 0.25
CSIM_MAX_RESPONSE_HEX = 516

VERDICT_FAIL = "FAIL"
VERDICT_PASS = "PASS"
VERDICT_SKIP = "SKIP"
VERDICT_BLOCKED = "BLOCKED"


class SuiteError(Exception):
    pass


class ApduResult:
    def __init__(self, classification: str, rapdu: bytes | None, detail: str) -> None:
        self.classification = classification  # ok | refused | error
        self.rapdu = rapdu
        self.detail = detail

    @property
    def sw(self) -> str:
        if self.rapdu is None or len(self.rapdu) < 2:
            return ""
        return self.rapdu[-2:].hex().upper()

    @property
    def data(self) -> bytes:
        return self.rapdu[:-2] if self.rapdu is not None and len(self.rapdu) >= 2 else b""


def parse_csim(line: str) -> bytes | None:
    m = CSIM_RE.match(line.strip())
    if not m:
        return None
    declared, hexs = int(m.group(1)), m.group(2)
    if declared != len(hexs) or len(hexs) % 2 or len(hexs) > CSIM_MAX_RESPONSE_HEX:
        return None
    return bytes.fromhex(hexs)


class DirectBackend:
    """AT+CSIM over TCP with the driver's transport classification."""

    def __init__(self, endpoint: str, timeout_ms: int) -> None:
        self.endpoint = endpoint
        self.timeout_ms = timeout_ms
        self.sock: socket.socket | None = None
        self.buf = b""
        self._last = 0.0

    def connect(self) -> None:
        host, _, port = self.endpoint.rpartition(":")
        self.sock = socket.create_connection((host or "127.0.0.1", int(port)), timeout=5.0)
        self.sock.settimeout(0.2)
        self.buf = b""

    def close(self) -> None:
        if self.sock:
            try:
                self.sock.close()
            except OSError:
                pass
            self.sock = None

    def _pace(self) -> None:
        wait = PACE_S - (time.monotonic() - self._last)
        if wait > 0:
            time.sleep(wait)

    def _readline(self, deadline: float) -> str | None:
        while b"\n" not in self.buf:
            if time.monotonic() > deadline:
                return None
            try:
                chunk = self.sock.recv(4096)
            except socket.timeout:
                continue
            except OSError:
                return None
            if not chunk:
                return None
            self.buf += chunk
        line, self.buf = self.buf.split(b"\n", 1)
        return line.strip(b"\r\n \t").decode("ascii", "replace")

    def transmit(self, apdu: bytes) -> ApduResult:
        self._pace()
        assert self.sock is not None
        hexs = apdu.hex().upper()
        cmd = f'AT+CSIM={len(hexs)},"{hexs}"'
        deadline = time.monotonic() + self.timeout_ms / 1000.0
        echo_deadline = time.monotonic() + min(2.0, self.timeout_ms / 1000.0)
        self.sock.sendall(cmd.encode() + b"\r")
        echo_seen = False
        rapdu: bytes | None = None
        while time.monotonic() < deadline:
            line = self._readline(deadline)
            if line is None:
                break
            if not echo_seen:
                if line == cmd:
                    echo_seen = True
                elif time.monotonic() > echo_deadline:
                    return ApduResult("error", None, "no echo before deadline")
                continue
            m = line.startswith("+CSIM:")
            if m:
                parsed = parse_csim(line)
                if parsed is None:
                    return ApduResult("refused", bytes.fromhex("6F00"),
                                      "malformed +CSIM framing (driver: 6F00)")
                rapdu = parsed
            elif line == "OK":
                self._last = time.monotonic()
                if rapdu is None:
                    return ApduResult("refused", bytes.fromhex("6F00"),
                                      "final OK without +CSIM (driver: 6F00)")
                return ApduResult("ok", rapdu, "exchange verified (echo + CSIM + OK)")
            elif line == "ERROR":
                self._last = time.monotonic()
                return ApduResult("refused", bytes.fromhex("6F00"),
                                  "modem ERROR (card-level refusal, driver: 6F00)")
        self._last = time.monotonic()
        if not echo_seen:
            return ApduResult("error", None, "no echo (ambiguous delivery)")
        return ApduResult("refused", bytes.fromhex("6F00"),
                          "echo then silence (modem drop, driver: 6F00)")


SCARD_SCOPE_USER = 0
SCARD_SHARE_SHARED = 0x00000002
SCARD_PROTOCOL_ANY = 0x00000003
SCARD_LEAVE_CARD = 0
ERROR_NAMES = {
    0x80100009: "SCARD_E_UNKNOWN_READER",
    0x8010000C: "SCARD_E_NO_SMARTCARD",
    0x80100016: "SCARD_E_NOT_TRANSACTED",
    0x8010001D: "SCARD_E_NO_SERVICE",
    0x8010002D: "SCARD_E_NO_READERS_AVAILABLE",
    0x80100066: "SCARD_W_RESET_CARD",
    0x80100068: "SCARD_W_INSERTED_CARD",
    0x80100069: "SCARD_W_REMOVED_CARD",
}


def pcsc_error_name(rc: int) -> str:
    return ERROR_NAMES.get(rc, f"pcsc-rc-0x{rc & 0xFFFFFFFF:08X}")


class IoRequest(ctypes.Structure):
    _fields_ = [("protocol", ctypes.c_uint32), ("pci_length", ctypes.c_uint32)]


def load_pcsc() -> ctypes.CDLL:
    for name in ("libpcsclite.so.1", "libpcsclite.so"):
        try:
            lib = ctypes.CDLL(name)
            lib.SCardEstablishContext.restype = ctypes.c_long
            lib.SCardEstablishContext.argtypes = [
                ctypes.c_ulong, ctypes.c_void_p, ctypes.c_void_p, ctypes.POINTER(ctypes.c_ulong)]
            lib.SCardListReaders.restype = ctypes.c_long
            lib.SCardListReaders.argtypes = [
                ctypes.c_ulong, ctypes.c_char_p, ctypes.c_char_p, ctypes.POINTER(ctypes.c_ulong)]
            lib.SCardConnect.restype = ctypes.c_long
            lib.SCardConnect.argtypes = [
                ctypes.c_ulong, ctypes.c_char_p, ctypes.c_ulong, ctypes.c_ulong,
                ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.c_ulong)]
            lib.SCardTransmit.restype = ctypes.c_long
            lib.SCardTransmit.argtypes = [
                ctypes.c_ulong, ctypes.POINTER(IoRequest), ctypes.c_char_p, ctypes.c_ulong,
                ctypes.POINTER(IoRequest), ctypes.c_char_p, ctypes.POINTER(ctypes.c_ulong)]
            lib.SCardDisconnect.restype = ctypes.c_long
            lib.SCardDisconnect.argtypes = [ctypes.c_ulong, ctypes.c_ulong]
            lib.SCardReleaseContext.restype = ctypes.c_long
            lib.SCardReleaseContext.argtypes = [ctypes.c_ulong]
            return lib
        except OSError:
            continue
    raise SuiteError("libpcsclite not found (install pcsclite)")


def pcsc_list_readers() -> list[str]:
    lib = load_pcsc()
    h = ctypes.c_ulong()
    rc = lib.SCardEstablishContext(SCARD_SCOPE_USER, None, None, ctypes.byref(h))
    if rc:
        raise SuiteError(f"SCardEstablishContext: {pcsc_error_name(rc)}")
    try:
        n = ctypes.c_ulong(0)
        rc = lib.SCardListReaders(h, None, None, ctypes.byref(n))
        if rc == 0x8010002D:
            return []
        if rc:
            raise SuiteError(f"SCardListReaders: {pcsc_error_name(rc)}")
        buf = ctypes.create_string_buffer(n.value)
        rc = lib.SCardListReaders(h, None, buf, ctypes.byref(n))
        if rc:
            raise SuiteError(f"SCardListReaders(2): {pcsc_error_name(rc)}")
        return [r.decode() for r in buf.raw[: n.value].split(b"\x00") if r]
    finally:
        lib.SCardReleaseContext(h)


def match_reader(available: list[str], wanted: str) -> str:
    exact = [r for r in available if r == wanted]
    if exact:
        return exact[0]
    partial = [r for r in available if wanted.lower() in r.lower()]
    if len(partial) == 1:
        return partial[0]
    listing = "\n".join(f"  - {r}" for r in available) or "  (none)"
    if len(partial) > 1:
        raise SuiteError(f"--reader {wanted!r} matches multiple readers:\n{listing}")
    raise SuiteError(f"--reader {wanted!r} not found; available readers:\n{listing}")


class PcscBackend:
    def __init__(self, reader: str, card_wait_s: float = 20.0) -> None:
        self.reader_name = match_reader(pcsc_list_readers(), reader)
        self.lib = load_pcsc()
        self.h = ctypes.c_ulong()
        self.card = ctypes.c_ulong()
        self.proto = ctypes.c_ulong()
        self.card_wait_s = card_wait_s

    def connect(self) -> None:
        rc = self.lib.SCardEstablishContext(SCARD_SCOPE_USER, None, None, ctypes.byref(self.h))
        if rc:
            raise SuiteError(f"SCardEstablishContext: {pcsc_error_name(rc)}")
        deadline = time.monotonic() + self.card_wait_s
        while True:
            rc = self.lib.SCardConnect(
                self.h, self.reader_name.encode(), SCARD_SHARE_SHARED, SCARD_PROTOCOL_ANY,
                ctypes.byref(self.card), ctypes.byref(self.proto))
            if rc == 0:
                return
            if rc != 0x8010000C or time.monotonic() > deadline:
                raise SuiteError(f"SCardConnect({self.reader_name}): {pcsc_error_name(rc)}")
            time.sleep(0.5)

    def close(self) -> None:
        if self.card.value:
            self.lib.SCardDisconnect(self.card, SCARD_LEAVE_CARD)
            self.card = ctypes.c_ulong()
        if self.h.value:
            self.lib.SCardReleaseContext(self.h)
            self.h = ctypes.c_ulong()

    def transmit(self, apdu: bytes) -> ApduResult:
        send_pci = IoRequest(self.proto.value, ctypes.sizeof(IoRequest))
        recv = ctypes.create_string_buffer(65536)
        n = ctypes.c_ulong(len(recv))
        rc = self.lib.SCardTransmit(
            self.card, ctypes.byref(send_pci), bytes(apdu), len(apdu),
            None, recv, ctypes.byref(n))
        if rc:
            return ApduResult("error", None, f"SCardTransmit: {pcsc_error_name(rc)}")
        return ApduResult("ok", recv.raw[: n.value], "pcsc exchange")


class Ctx:
    def __init__(self) -> None:
        self.channel: int | None = None
        self.iccid_hex: str | None = None
        self.eids: list[str] = []


class Case:
    def __init__(self, cid, name, risk, build=None, check=None,
                 needs_channel=False, skip_reason="", lpac=False) -> None:
        self.cid = cid
        self.name = name
        self.risk = risk
        self.build = build
        self.check = check
        self.needs_channel = needs_channel
        self.skip_reason = skip_reason
        self.lpac = lpac


def select_fid(fid: str) -> bytes:
    return bytes([0x00, 0xA4, 0x00, 0x0C, 0x02]) + bytes.fromhex(fid)


def select_fid_ch(ctx: Ctx, fid: str) -> bytes:
    ch = ctx.channel or 0
    return bytes([ch, 0xA4, 0x00, 0x0C, 0x02]) + bytes.fromhex(fid)


def sw_ok(result: ApduResult, want: str = "9000", accept: tuple[str, ...] = (),
          drift_note: str = ""):
    if result.classification == "error":
        return VERDICT_BLOCKED, f"transport: {result.detail}"
    sw = result.sw
    if sw == want:
        return VERDICT_PASS, ""
    if sw in accept:
        return VERDICT_PASS, f"drift: {drift_note} (SW {sw})"
    return VERDICT_FAIL, f"expected SW {want}, got {result.sw or '(none)'}"


def sacrificial_cases() -> list[Case]:
    return [
        Case("SAC-001", "SELECT MF by FID on basic channel", "class-A: non-mutating select",
             build=lambda ctx: select_fid("3F00"),
             check=lambda ctx, r: sw_ok(r, "9000")),
        Case("SAC-002", "basic-channel drop fingerprint: SELECT by DF-name (A4/P1=04)",
             "class-A: modem drops it before the card; harmless select if not",
             build=lambda ctx: bytes([0x00, 0xA4, 0x04, 0x0C, 0x02, 0x31, 0x32]),
             check=lambda ctx, r: sw_ok(r, "6F00", accept=("6A82",),
                                        drift_note="basic-channel DF-name select answered (filter absent)")),
        Case("SAC-003", "basic-channel drop fingerprint: READ BINARY (B0)",
             "class-A: modem drops it before the card; harmless read if not",
             build=lambda ctx: bytes([0x00, 0xB0, 0x00, 0x00, 0x0A]),
             check=lambda ctx, r: sw_ok(r, "6F00", accept=("6982", "6A86", "6A82"),
                                        drift_note="basic-channel READ BINARY answered (filter absent)")),
        Case("SAC-004", "open logical channel (MANAGE CHANNEL)",
             "class-A: channel lifecycle, no card state change",
             build=lambda ctx: bytes([0x00, 0x70, 0x00, 0x00, 0x00]),
             check=lambda ctx, r: _check_open(ctx, r)),
        Case("SAC-005", "SELECT MF by FID on logical channel",
             "class-A: non-mutating select (logical channel per modem findings)",
             build=lambda ctx: select_fid_ch(ctx, "3F00"),
             check=lambda ctx, r: sw_ok(r, "9000"), needs_channel=True),
        Case("SAC-006", "SELECT EF.ICCID (2FE2) by FID on logical channel",
             "class-A: fixture-identity select",
             build=lambda ctx: select_fid_ch(ctx, "2FE2"),
             check=lambda ctx, r: sw_ok(r, "9000"), needs_channel=True),
        Case("SAC-007", "READ BINARY EF.ICCID on logical channel (fixture fingerprint)",
             "class-A: read-only EF read via logical channel",
             build=lambda ctx: bytes([ctx.channel or 0, 0xB0, 0x00, 0x00, 0x0A]),
             check=lambda ctx, r: _check_iccid(ctx, r), needs_channel=True),
        Case("SAC-008", "close logical channel",
             "class-A: channel lifecycle",
             build=lambda ctx: bytes([0x00, 0x70, 0x80, ctx.channel or 0, 0x00]),
             check=lambda ctx, r: _check_close(ctx, r), needs_channel=True),
        Case("SAC-009", "post-cycle sanity: SELECT MF by FID on basic channel",
             "class-A: non-mutating select",
             build=lambda ctx: select_fid("3F00"),
             check=lambda ctx, r: sw_ok(r, "9000")),
    ]


def _check_open(ctx: Ctx, r: ApduResult):
    verdict, detail = sw_ok(r, "9000")
    if verdict == VERDICT_PASS:
        if not r.data or not 1 <= r.data[0] <= 3:
            return VERDICT_FAIL, f"channel number out of range: {r.data.hex() or '(none)'}"
        ctx.channel = r.data[0]
    return verdict, detail


def _check_close(ctx: Ctx, r: ApduResult):
    verdict, detail = sw_ok(r, "9000")
    if verdict == VERDICT_PASS:
        ctx.channel = None
    return verdict, detail


def _check_iccid(ctx: Ctx, r: ApduResult):
    verdict, detail = sw_ok(r, "9000")
    if verdict == VERDICT_PASS:
        if not 1 <= len(r.data) <= 20:
            return VERDICT_FAIL, f"implausible ICCID length {len(r.data)}"
        ctx.iccid_hex = r.data.hex().upper()
    return verdict, detail


def esimgate_cases() -> list[Case]:
    return [
        Case("ESG-001", "open logical channel", "class-A: channel lifecycle",
             build=lambda ctx: bytes([0x00, 0x70, 0x00, 0x00, 0x00]),
             check=lambda ctx, r: _check_open(ctx, r)),
        Case("ESG-002", "SELECT ISD-R by AID on logical channel (readiness)",
             "class-A: public-AID select, non-authenticating (doctrine-sanctioned)",
             build=lambda ctx: bytes([ctx.channel or 0, 0xA4, 0x04, 0x0C, 0x10])
                 + bytes.fromhex(ISD_R_AID),
             check=lambda ctx, r: _check_isd_r(r), needs_channel=True),
        Case("ESG-003", "close logical channel", "class-A: channel lifecycle",
             build=lambda ctx: bytes([0x00, 0x70, 0x80, ctx.channel or 0, 0x00]),
             check=lambda ctx, r: _check_close(ctx, r), needs_channel=True),
        Case("ESG-004", "lpac chip info (EID + chipInfo, read-only)",
             "class-A: ES10b GetEID/euiccInfo only; no profile operations",
             lpac=True),
    ]


def _check_isd_r(r: ApduResult):
    if r.classification == "error":
        return VERDICT_BLOCKED, f"transport: {r.detail}"
    sw = r.sw
    if sw == "9000":
        return VERDICT_PASS, ""
    if sw == "6A82":
        return VERDICT_FAIL, "ISD-R AID not found — no eUICC / wrong card seated"
    return VERDICT_FAIL, f"ISD-R select SW {sw}"


def find_eid(obj) -> str | None:
    if isinstance(obj, dict):
        for k, v in obj.items():
            if k.lower() == "eid" and isinstance(v, str) and re.fullmatch(r"[0-9A-Fa-f]{32}", v):
                return v.lower()
            found = find_eid(v)
            if found:
                return found
    elif isinstance(obj, list):
        for item in obj:
            found = find_eid(item)
            if found:
                return found
    return None


def run_lpac(ctx: Ctx, attempts: int) -> list[dict]:
    import shutil

    records: list[dict] = []
    lpac_bin = shutil.which("lpac")
    if lpac_bin is None:
        return [{
            "id": "ESG-004", "attempt": 1, "name": "lpac chip info (EID + chipInfo, read-only)",
            "verdict": VERDICT_SKIP, "expected": "lpac chip info code 0 + 32-hex EID",
            "observed": "", "detail": "lpac not on PATH", "duration_ms": 0,
            "evidence": {},
        }]
    for i in range(1, attempts + 1):
        t0 = time.monotonic()
        try:
            proc = subprocess.run(
                [lpac_bin, "chip", "info"], capture_output=True, text=True, timeout=90,
                env={**os.environ, "LPAC_APDU": "pcsc"})
        except subprocess.TimeoutExpired:
            records.append(_lpac_rec(i, VERDICT_FAIL, "lpac timed out (90s)", "", 0, t0))
            continue
        out = proc.stdout.strip()
        err = proc.stderr.strip()
        rec = _lpac_rec(i, None, "", out, proc.returncode, t0, err=err)
        if proc.returncode != 0:
            infra = re.search(r"reader|service|SCard|socket", err, re.I) is not None
            rec["verdict"] = VERDICT_BLOCKED if infra else VERDICT_FAIL
            rec["detail"] = f"lpac rc={proc.returncode}: {err[:200]}"
        else:
            try:
                eid = find_eid(json.loads(out))
            except json.JSONDecodeError:
                eid = None
            if eid is None:
                rec["verdict"] = VERDICT_FAIL
                rec["detail"] = "lpac OK but no 32-hex EID found in output"
            else:
                ctx.eids.append(eid)
                rec["verdict"] = VERDICT_PASS
                rec["observed"] = f"EID {eid}"
                rec["detail"] = ""
        records.append(rec)
    if ctx.eids and len(set(ctx.eids)) > 1:
        for rec in records:
            if rec["id"] == "ESG-004" and rec["verdict"] == VERDICT_PASS:
                rec["verdict"] = VERDICT_FAIL
                rec["detail"] = f"EID changed between reads: {ctx.eids}"
    return records


def _lpac_rec(attempt, verdict, detail, out, rc, t0, err=""):
    return {
        "id": "ESG-004", "attempt": attempt,
        "name": "lpac chip info (EID + chipInfo, read-only)",
        "verdict": verdict if verdict else "",
        "expected": "lpac chip info code 0 + 32-hex EID",
        "observed": out[:120] if verdict == VERDICT_PASS else "",
        "detail": detail, "duration_ms": int((time.monotonic() - t0) * 1000),
        "evidence": {"rc": str(rc), "stderr": err[:300], "stdout": out[:600]},
    }


def card_guard(suite: str, backend: str, endpoint: str | None, card_decl: str | None) -> None:
    touches_real = backend == "pcsc" or (
        backend == "direct" and endpoint is not None
        and endpoint.rpartition(":")[0].lower() not in LOOPBACK
    )
    if not touches_real:
        return
    if not card_decl:
        raise SuiteError(
            "refusing: this run can reach a real card — set --card-decl "
            "(disposable card identifier; NEVER the 9eSIM)")
    if "9esim" in card_decl.lower():
        raise SuiteError("refusing: --card-decl names the production 9eSIM — "
                         "that card must never be a suite target (session-2 directive)")
    if suite == "sacrificial" and card_decl != "sacrificial":
        raise SuiteError("refusing: sacrificial suite requires --card-decl sacrificial "
                         "(disposable SIM only)")


def run_suite(suite: str, args) -> int:
    cases = sacrificial_cases() if suite == "sacrificial" else esimgate_cases()
    records: list[dict] = []
    out = open(args.results, "w", encoding="utf-8") if args.results else sys.stdout

    def emit(rec: dict) -> None:
        records.append(rec)
        out.write(json.dumps(rec) + "\n")
        out.flush()

    try:
        if args.block_all:
            for c in cases:
                emit({"id": c.cid, "attempt": 1, "name": c.name, "verdict": VERDICT_BLOCKED,
                      "expected": _expected_for(c.cid), "observed": "",
                      "detail": f"blocked: {args.block_all}", "duration_ms": 0, "evidence": {}})
            return _exit_code(records)

        card_guard(suite, args.backend, args.endpoint, args.card_decl)
        ctx = Ctx()
        backend = None
        try:
            if args.backend == "direct":
                backend = DirectBackend(args.endpoint, args.timeout_ms)
            else:
                backend = PcscBackend(args.reader, args.card_wait_s)
                print(f"reader: {backend.reader_name}", file=sys.stderr)
            try:
                backend.connect()
            except OSError as e:
                raise SuiteError(f"backend connect failed: {e}") from e
            for c in cases:
                if c.lpac:
                    if args.backend != "pcsc":
                        emit({"id": c.cid, "attempt": 1, "name": c.name, "verdict": VERDICT_SKIP,
                              "expected": _expected_for(c.cid), "observed": "",
                              "detail": "requires pcsc backend (dry-run: mock has no lpac target)",
                              "duration_ms": 0, "evidence": {}})
                        continue
                    for rec in run_lpac(ctx, args.lpac_repeats):
                        emit(rec)
                    continue
                attempts = args.repeats if suite == "sacrificial" and c.cid in REPEATABLE else 1
                for attempt in range(1, attempts + 1):
                    _run_case(emit, c, ctx, backend, attempt)
        finally:
            if backend:
                backend.close()
    finally:
        if out is not sys.stdout:
            out.close()
    return _exit_code(records)


def _exit_code(records: list[dict]) -> int:
    verdicts = {r["verdict"] for r in records}
    if VERDICT_FAIL in verdicts:
        return 1
    if VERDICT_BLOCKED in verdicts or VERDICT_SKIP in verdicts:
        return 2
    return 0 if records else 3


REPEATABLE = {"SAC-004", "SAC-005", "SAC-006", "SAC-007", "SAC-008"}


def _run_case(emit, c: Case, ctx: Ctx, backend, attempt: int) -> None:
    if c.needs_channel and ctx.channel is None:
        emit({"id": c.cid, "attempt": attempt, "name": c.name, "verdict": VERDICT_BLOCKED,
              "expected": _expected_for(c.cid), "observed": "",
              "detail": "prerequisite failed: no logical channel open", "duration_ms": 0,
              "evidence": {}})
        return
    apdu = c.build(ctx) if c.build else None
    if apdu is None:
        emit({"id": c.cid, "attempt": attempt, "name": c.name, "verdict": VERDICT_SKIP,
              "expected": _expected_for(c.cid), "observed": "", "detail": c.skip_reason,
              "duration_ms": 0, "evidence": {}})
        return
    t0 = time.monotonic()
    try:
        result = backend.transmit(apdu)
    except OSError as e:
        result = ApduResult("error", None, f"transport died: {e}")
    dur = int((time.monotonic() - t0) * 1000)
    verdict, detail = c.check(ctx, result)
    observed = f"SW {result.sw}" if result.rapdu is not None else f"transport ({result.classification})"
    emit({"id": c.cid, "attempt": attempt, "name": c.name, "verdict": verdict,
          "expected": _expected_for(c.cid), "observed": observed,
          "detail": (detail + " [" + c.risk + "]") if detail else ("[" + c.risk + "]"),
          "duration_ms": dur,
          "evidence": {"apdu": apdu.hex().upper(),
                       "rapdu": result.rapdu.hex().upper() if result.rapdu else "",
                       "classification": result.classification}})


def _expected_for(cid: str) -> str:
    return {
        "SAC-001": "SW 9000", "SAC-002": "SW 6F00 (drop) or 6A82 (drift)",
        "SAC-003": "SW 6F00 (drop) or 6982/6A86 (drift)",
        "SAC-004": "SW 9000 + channel 1..3", "SAC-005": "SW 9000",
        "SAC-006": "SW 9000", "SAC-007": "SW 9000 + 1..20 data bytes",
        "SAC-008": "SW 9000", "SAC-009": "SW 9000",
        "ESG-001": "SW 9000 + channel 1..3", "ESG-002": "SW 9000",
        "ESG-003": "SW 9000",
        "ESG-004": "lpac chip info code 0 + 32-hex EID",
    }.get(cid, "")


def probe_endpoint(endpoint: str) -> int:
    try:
        host, _, port = endpoint.rpartition(":")
        s = socket.create_connection((host or "127.0.0.1", int(port)), timeout=2.0)
        s.settimeout(2.0)
        s.sendall(b"AT\r")
        deadline = time.monotonic() + 2.0
        buf = b""
        while time.monotonic() < deadline:
            try:
                buf += s.recv(4096)
            except socket.timeout:
                break
            if b"OK" in buf:
                print("alive")
                s.close()
                return 0
        s.close()
        print("no AT response", file=sys.stderr)
        return 1
    except OSError as e:
        print(f"unreachable: {e}", file=sys.stderr)
        return 1


def self_test() -> int:
    checks: list[tuple[str, bool, str]] = []

    def check(name: str, ok: bool, info: str = "") -> None:
        checks.append((name, bool(ok), info))

    check("parse_csim strict ok", parse_csim('+CSIM: 4,"9000"') == b"\x90\x00")
    check("parse_csim len mismatch", parse_csim('+CSIM: 8,"9000"') is None)
    check("parse_csim non-hex", parse_csim('+CSIM: 4,"ZZZZ"') is None)
    check("parse_csim odd", parse_csim('+CSIM: 3,"900"') is None)
    check("parse_csim r-apdu cap", parse_csim('+CSIM: 518,"{}"'.format("AB" * 259)) is None)

    check("match exact", match_reader(["NR7101_SIM_dut", "other"], "NR7101_SIM_dut") == "NR7101_SIM_dut")
    check("match unique substring", match_reader(["x NR7101_SIM_dut"], "NR7101") == "x NR7101_SIM_dut")
    for label, args in {
        "ambiguous": ["NR7101_SIM_dut", "NR7101_SIM_dut2"],
        "missing": ["unrelated"],
    }.items():
        try:
            match_reader(args, "NR7101_SIM_dut" if label == "missing" else "SIM")
            check(f"match {label} rejected", False)
        except SuiteError:
            check(f"match {label} rejected", True)
    check("match empty list rejected", _raises(match_reader, [], "x"))

    check("guard: 9esim refused", _raises(card_guard, "sacrificial", "pcsc", None, "9esim"))
    check("guard: undeclared refused", _raises(card_guard, "esimgate", "pcsc", None, None))
    check("guard: sacrificial requires sacrificial",
          _raises(card_guard, "sacrificial", "pcsc", None, "esim-test"))
    check("guard: loopback direct needs nothing",
          not _raises(card_guard, "esimgate", "direct", "127.0.0.1:17002", None))
    check("guard: non-loopback direct declared ok",
          not _raises(card_guard, "esimgate", "direct", "192.168.1.10:7002", "esim-test"))

    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import mock_modem

    for flavor in ({"filter": "basic"}, {"filter": "none"}, {"filter": "basic", "chaos_urc": True}):
        label = ",".join(f"{k}={v}" for k, v in flavor.items())
        srv = _spawn_mock(mock_modem, flavor)
        try:
            verdicts = _run_all_against(f"127.0.0.1:{srv.server_address[1]}", "sacrificial")
            ok = all(v == "PASS" for v in verdicts.values())
            check(f"sacrificial suite vs mock ({label})", ok, str(verdicts))
        finally:
            srv.shutdown()
            srv.server_close()

        srv = _spawn_mock(mock_modem, flavor)
        try:
            verdicts = _run_all_against(f"127.0.0.1:{srv.server_address[1]}", "esimgate")
            ok = (verdicts["ESG-001"] == "PASS" and verdicts["ESG-002"] == "PASS"
                  and verdicts["ESG-003"] == "PASS" and verdicts["ESG-004"] == "SKIP")
            check(f"esimgate suite vs mock ({label})", ok, str(verdicts))
        finally:
            srv.shutdown()
            srv.server_close()

    failed = [c for c in checks if not c[1]]
    for name, ok, info in checks:
        print(f"  {'ok' if ok else 'FAIL'}: {name}" + (f" ({info})" if info and not ok else ""))
    print(f"suite.py self-test: {len(checks) - len(failed)}/{len(checks)} PASS")
    return 1 if failed else 0


def _raises(fn, *args) -> bool:
    try:
        fn(*args)
        return False
    except SuiteError:
        return True


def _spawn_mock(mock_modem, flavor):
    import socketserver

    cfg = {"filter": flavor.get("filter", "basic"),
           "chaos_urc": flavor.get("chaos_urc", False), "logf": None}

    class Handler(socketserver.BaseRequestHandler):
        def handle(self) -> None:
            mock_modem.handle_conn(self.request, self.client_address, cfg, threading.Lock())

    class Server(socketserver.ThreadingTCPServer):
        allow_reuse_address = True
        daemon_threads = True

    srv = Server(("127.0.0.1", 0), Handler)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv


def _run_all_against(endpoint: str, suite: str) -> dict:
    import tempfile

    verdicts: dict[str, str] = {}
    with tempfile.NamedTemporaryFile("r+", suffix=".jsonl", delete=False) as f:
        path = f.name
    real_argv = sys.argv
    sys.argv = ["suite.py", suite, "--backend", "direct", "--endpoint", endpoint,
                "--results", path, "--timeout-ms", "1200", "--repeats", "1"]
    try:
        main()
    finally:
        sys.argv = real_argv
    with open(path, encoding="utf-8") as f:
        for line in f:
            if line.strip():
                rec = json.loads(line)
                verdicts.setdefault(rec["id"], rec["verdict"])
                verdicts[rec["id"] + f"#{rec['attempt']}"] = rec["verdict"]
    Path(path).unlink(missing_ok=True)
    return verdicts


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("suite", nargs="?", choices=["sacrificial", "esimgate"])
    ap.add_argument("--backend", choices=["direct", "pcsc"], default="direct")
    ap.add_argument("--endpoint", default="127.0.0.1:17002",
                    help="host:port for the direct backend")
    ap.add_argument("--reader", default=None,
                    help="PC/SC reader FRIENDLYNAME (required for pcsc; never first-reader)")
    ap.add_argument("--card-decl", default=None,
                    help="declared target card when a real card is reachable "
                         "(must NEVER be the 9eSIM)")
    ap.add_argument("--repeats", type=int, default=1)
    ap.add_argument("--lpac-repeats", type=int, default=3)
    ap.add_argument("--timeout-ms", type=int, default=3000)
    ap.add_argument("--card-wait-s", type=float, default=20.0,
                    help="pcsc: wait for card presence on a freshly added reader")
    ap.add_argument("--results", default=None, help="JSONL output file (default stdout)")
    ap.add_argument("--block-all", default=None,
                    help="emit every case BLOCKED with this reason; no execution")
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--list-readers", action="store_true")
    ap.add_argument("--probe-endpoint", default=None, metavar="HOST:PORT")
    args = ap.parse_args()

    if args.self_test:
        return self_test()
    if args.probe_endpoint:
        return probe_endpoint(args.probe_endpoint)
    if args.list_readers:
        try:
            for name in pcsc_list_readers():
                print(name)
            return 0
        except SuiteError as e:
            print(f"suite.py: {e}", file=sys.stderr)
            return 3
    if not args.suite:
        ap.error("suite required (sacrificial|esimgate)")
    try:
        return run_suite(args.suite, args)
    except SuiteError as e:
        print(f"suite.py: {e}", file=sys.stderr)
        return 3


if __name__ == "__main__":
    sys.exit(main())
