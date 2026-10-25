#!/usr/bin/env python3
"""rehearse_c0_battery.py — reproducible PTY rehearsal for
tests/hardware/at_csim_c0_battery.sh against tests/hardware/mock_c0_responder.py.

Regenerates the instrument-verification evidence any time, no hardware:
  scenario c0-works     -> exit 0, C0 ALLOCATED-CHANNEL-OK,   LOWERCASE-REJECTED
  scenario c0-fails     -> exit 1, C0 ALLOCATED-CHANNEL-FAIL(6D00), LOWERCASE-REJECTED
  scenario case-tolerant-> exit 1, LOWERCASE-ACCEPTED(9000)

The PTY slave is set raw (tty.setraw): a real modem port runs raw
(stty raw -echo), and cooked-mode ECHO creates a responder feedback loop
(the bug the first rehearsal run caught — keep it raw).

Usage: rehearse_c0_battery.py [--keep DIR]
Exit 0 iff all three scenarios produce their expected verdicts.
"""
import os
import pty
import subprocess
import sys
import time
import tty

HERE = os.path.dirname(os.path.abspath(__file__))
BATTERY = os.path.join(HERE, "at_csim_c0_battery.sh")
RESPONDER = os.path.join(HERE, "mock_c0_responder.py")

EXPECT = {
    "c0-works": (0, ["C0: ALLOCATED-CHANNEL-OK", "hex-case: LOWERCASE-REJECTED"]),
    "c0-fails": (1, ["C0: ALLOCATED-CHANNEL-FAIL(6D00)", "hex-case: LOWERCASE-REJECTED"]),
    "case-tolerant": (1, ["hex-case: LOWERCASE-ACCEPTED(9000)"]),
}


def run_scenario(scenario, keep_dir):
    master, slave = pty.openpty()
    tty.setraw(slave)
    os.set_blocking(master, False)
    responder = subprocess.Popen(
        [sys.executable, "-u", RESPONDER, scenario, str(master)],
        pass_fds=(master,),
    )
    time.sleep(0.3)
    try:
        proc = subprocess.run(
            ["sh", BATTERY, os.ttyname(slave), "2"],
            capture_output=True, text=True, timeout=200,
        )
        rc, out = proc.returncode, proc.stdout + proc.stderr
    except subprocess.TimeoutExpired as e:
        rc, out = -99, "[TIMEOUT]"
    responder.terminate()
    responder.wait()
    os.close(master)
    os.close(slave)
    if keep_dir:
        open(os.path.join(keep_dir, f"{scenario}.log"), "w").write(out)
    want_rc, want_lines = EXPECT[scenario]
    ok = rc == want_rc and all(w in out for w in want_lines)
    print(f"{scenario}: exit={rc} expected={want_rc} -> {'PASS' if ok else 'FAIL'}")
    if not ok:
        for w in want_lines:
            if w not in out:
                print(f"  missing: {w!r}")
    return ok


def main():
    keep = None
    if "--keep" in sys.argv:
        keep = sys.argv[sys.argv.index("--keep") + 1]
        os.makedirs(keep, exist_ok=True)
    results = [run_scenario(s, keep) for s in EXPECT]
    print("REHEARSAL:", "PASS" if all(results) else "MISMATCH")
    sys.exit(0 if all(results) else 1)


if __name__ == "__main__":
    main()
