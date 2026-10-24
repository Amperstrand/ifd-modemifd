#!/usr/bin/env python3
"""report.py — stamped JSON+MD report generator for the modem-hil suites.

Input: one JSON object per line (JSONL), each a case record:
  {"id": str, "attempt": int, "name": str, "verdict": "PASS|FAIL|SKIP|BLOCKED",
   "expected": str, "observed": str, "detail": str, "duration_ms": number,
   "evidence": {str: str}}

Output: <out-dir>/<UTC-stamp>-<suite>/report.json + report.md + cases.jsonl.

Truthfulness contract (enforced, not just documented):
  * case verdicts must be one of PASS/FAIL/SKIP/BLOCKED — anything else aborts;
  * PASS requires non-empty evidence (observed SW or recorded output);
  * suite verdict is PASS only when every case PASSed and at least one ran;
    any FAIL -> FAIL, else any BLOCKED -> BLOCKED, otherwise SKIP;
  * dry-run reports can never be a hardware qualification: the mode is
    stamped into the verdict line and the eSIM-provisioning gate stays
    closed unless mode=hw AND verdict=PASS.
"""

from __future__ import annotations

import json
import sys
import tempfile
import time
from datetime import datetime, timezone
from pathlib import Path

VERDICTS = ("PASS", "FAIL", "SKIP", "BLOCKED")
SCHEMA = "modemifd-suite-report/1"
GATE_SUITES = ("validate-esimgate",)


class ReportError(Exception):
    pass


def load_cases(path: str) -> list[dict]:
    cases: list[dict] = []
    with open(path, encoding="utf-8") as f:
        for lineno, line in enumerate(f, 1):
            line = line.strip()
            if not line:
                continue
            try:
                rec = json.loads(line)
            except json.JSONDecodeError as e:
                raise ReportError(f"{path}:{lineno}: bad JSON: {e}") from e
            verdict = rec.get("verdict")
            if verdict not in VERDICTS:
                raise ReportError(f"{path}:{lineno}: verdict {verdict!r} not in {VERDICTS}")
            for key in ("id", "name"):
                if not rec.get(key):
                    raise ReportError(f"{path}:{lineno}: missing {key}")
            if verdict == "PASS" and not (rec.get("observed") or rec.get("evidence")):
                raise ReportError(
                    f"{path}:{lineno}: PASS without evidence — refusing (never green for unexecuted)"
                )
            rec.setdefault("attempt", 1)
            rec.setdefault("expected", "")
            rec.setdefault("observed", "")
            rec.setdefault("detail", "")
            rec.setdefault("duration_ms", 0)
            rec.setdefault("evidence", {})
            cases.append(rec)
    return cases


def rollup(cases: list[dict]) -> dict:
    counts = {v: 0 for v in VERDICTS}
    for c in cases:
        counts[c["verdict"]] += 1
    executed = counts["PASS"] + counts["FAIL"]
    if counts["FAIL"]:
        verdict = "FAIL"
    elif counts["BLOCKED"]:
        verdict = "BLOCKED"
    elif executed == 0 or counts["SKIP"]:
        verdict = "SKIP"
    else:
        verdict = "PASS"
    return {
        "pass": counts["PASS"],
        "fail": counts["FAIL"],
        "skip": counts["SKIP"],
        "blocked": counts["BLOCKED"],
        "executed": executed,
        "total": len(cases),
        "verdict": verdict,
    }


def gate_for(suite: str, mode: str, verdict: str) -> dict | None:
    if suite not in GATE_SUITES:
        return None
    if mode != "hw":
        return {"provisioning_may_proceed": False, "reason": f"mode={mode}: mock/dry-run evidence only"}
    return {"provisioning_may_proceed": verdict == "PASS", "reason": f"mode=hw verdict={verdict}"}


def verdict_line(suite: str, mode: str, verdict: str) -> str:
    if mode == "dry-run":
        return f"{verdict} (dry-run — mock modem evidence only, NOT a hardware qualification)"
    return verdict


def render_markdown(rep: dict) -> str:
    lines: list[str] = []
    mode = rep["mode"]
    lines.append(f"# {rep['suite']} — {verdict_line(rep['suite'], mode, rep['verdict'])}")
    lines.append("")
    lines.append(f"- stamp (UTC): {rep['stamp_utc']}")
    lines.append(f"- mode: **{mode}**")
    if mode == "dry-run":
        lines.append("  - ⚠ DRY-RUN: every executed case ran against `mock_modem.py` on loopback.")
        lines.append("  - ⚠ This report can never green-light hardware operation or provisioning.")
    lines.append(f"- host: {rep['host']}")
    git = rep.get("git") or {}
    lines.append(f"- git: {git.get('branch', '?')} @ {git.get('commit', '?')}"
                 + (" (dirty)" if git.get("dirty") else ""))
    for k, v in sorted((rep.get("meta") or {}).items()):
        lines.append(f"- {k}: {v}")
    lines.append("")
    s = rep["summary"]
    lines.append("## Summary")
    lines.append("")
    lines.append(f"PASS {s['pass']} · FAIL {s['fail']} · SKIP {s['skip']} · BLOCKED {s['blocked']}"
                 f" — executed {s['executed']}/{s['total']}")
    if s["executed"] == 0:
        lines.append("")
        lines.append("**NOT EXECUTED — zero cases ran; this report qualifies nothing.**")
    lines.append("")
    lines.append("## Cases")
    lines.append("")
    lines.append("| id | attempt | verdict | detail | ms |")
    lines.append("|---|---|---|---|---|")
    for c in rep["cases"]:
        detail = str(c["detail"]).replace("|", "\\|")
        lines.append(f"| {c['id']} | {c['attempt']} | {c['verdict']} | {detail} | {c['duration_ms']} |")
    lines.append("")
    lines.append("## Evidence")
    lines.append("")
    for c in rep["cases"]:
        lines.append(f"### {c['id']} (attempt {c['attempt']}) — {c['name']}")
        lines.append(f"- expected: {c['expected']}")
        lines.append(f"- observed: {c['observed']}")
        for k, v in c["evidence"].items():
            lines.append(f"- {k}: `{v}`")
        lines.append("")
    gate = rep.get("gate")
    if gate is not None:
        lines.append("## Provisioning gate")
        lines.append("")
        may = "**NO** — do NOT provision" if not gate["provisioning_may_proceed"] else "**YES**"
        lines.append(f"- provisioning may proceed: {may}")
        lines.append(f"- reason: {gate['reason']}")
        lines.append("")
    lines.append(f"---\n_generated by report.py · schema {SCHEMA} · {rep['stamp_utc']}_")
    return "\n".join(lines) + "\n"


def build_report(suite: str, mode: str, cases: list[dict], meta: dict, git: dict) -> dict:
    summary = rollup(cases)
    return {
        "schema": SCHEMA,
        "suite": suite,
        "mode": mode,
        "stamp_utc": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "host": _hostname(),
        "git": git,
        "meta": meta,
        "cases": cases,
        "summary": summary,
        "verdict": summary["verdict"],
        "gate": gate_for(suite, mode, summary["verdict"]),
    }


def _hostname() -> str:
    import socket

    return socket.gethostname()


def _git_info(git_dir: str | None) -> dict:
    import subprocess

    def git_out(*args: str) -> str:
        cmd = ["git", *args]
        if git_dir:
            cmd = ["git", "-C", git_dir, *args]
        try:
            return subprocess.run(cmd, capture_output=True, text=True, check=True).stdout.strip()
        except (subprocess.CalledProcessError, FileNotFoundError):
            return ""

    commit = git_out("rev-parse", "HEAD")
    return {
        "commit": commit[:12] or "unknown",
        "branch": git_out("branch", "--show-current") or "unknown",
        "dirty": bool(git_out("status", "--porcelain")),
    }


def write_report(rep: dict, out_dir: str, results_path: str) -> Path:
    base = Path(out_dir)
    base.mkdir(parents=True, exist_ok=True)
    stamp = rep["stamp_utc"].replace("-", "").replace(":", "").replace("Z", "Z")
    target = base / f"{stamp}-{rep['suite']}"
    suffix = 2
    while target.exists():
        target = base / f"{stamp}-{rep['suite']}-{suffix}"
        suffix += 1
    target.mkdir(parents=True)
    (target / "report.json").write_text(json.dumps(rep, indent=2) + "\n", encoding="utf-8")
    (target / "report.md").write_text(render_markdown(rep), encoding="utf-8")
    (target / "cases.jsonl").write_text(Path(results_path).read_text(encoding="utf-8"), encoding="utf-8")
    return target


def parse_args(argv: list[str]) -> dict:
    args: dict = {"suite": None, "mode": None, "results": None, "out_dir": ".", "meta": {}, "git_dir": None}
    it = iter(argv)
    for a in it:
        if a == "--suite":
            args["suite"] = next(it, None)
        elif a == "--mode":
            args["mode"] = next(it, None)
        elif a == "--results":
            args["results"] = next(it, None)
        elif a == "--out-dir":
            args["out_dir"] = next(it, None)
        elif a == "--git-dir":
            args["git_dir"] = next(it, None)
        elif a == "--meta":
            kv = next(it, "")
            key, sep, val = kv.partition("=")
            if not sep:
                raise ReportError(f"--meta expects key=value, got {kv!r}")
            args["meta"][key] = val
        else:
            raise ReportError(f"unknown arg {a!r}")
    for req in ("suite", "mode", "results"):
        if not args[req]:
            raise ReportError(f"missing required --{req}")
    if args["mode"] not in ("hw", "dry-run"):
        raise ReportError(f"--mode must be hw or dry-run, got {args['mode']!r}")
    return args


def run_cli(argv: list[str]) -> int:
    try:
        args = parse_args(argv)
        cases = load_cases(args["results"])
        rep = build_report(
            args["suite"], args["mode"], cases, args["meta"], _git_info(args["git_dir"])
        )
        target = write_report(rep, args["out_dir"], args["results"])
        print(f"report: {target}/report.md")
        print(f"verdict: {verdict_line(rep['suite'], rep['mode'], rep['verdict'])}")
        return {"PASS": 0, "FAIL": 1, "BLOCKED": 2, "SKIP": 2}[rep["verdict"]]
    except ReportError as e:
        print(f"report.py: error: {e}", file=sys.stderr)
        return 3


def _case(vid: str, verdict: str, *, evidence: str = "9000", **kw) -> dict:
    rec = {"id": vid, "attempt": 1, "name": f"case {vid}", "verdict": verdict,
           "expected": "SW 9000", "observed": evidence if verdict in ("PASS", "FAIL") else "",
           "detail": kw.get("detail", ""), "duration_ms": 1, "evidence": {}}
    rec.update({k: v for k, v in kw.items() if k not in rec})
    if verdict == "PASS" and not rec["observed"]:
        rec["observed"] = evidence
    if verdict in ("SKIP", "BLOCKED"):
        rec["observed"] = ""
        rec["evidence"] = {}
    return rec


def self_test() -> int:
    import shutil

    tmp = Path(tempfile.mkdtemp(prefix="report-selftest-"))
    checks: list[tuple[str, bool, str]] = []

    def check(name: str, ok: bool, info: str = "") -> None:
        checks.append((name, ok, info))

    def write_jsonl(name: str, cases: list[dict]) -> str:
        p = tmp / name
        p.write_text("".join(json.dumps(c) + "\n" for c in cases), encoding="utf-8")
        return str(p)

    def gen(name: str, suite: str, mode: str, cases: list[dict]) -> dict:
        rep = build_report(suite, mode, cases, {}, {"commit": "selftest", "branch": "selftest", "dirty": False})
        target = write_report(rep, str(tmp / name), write_jsonl(f"{name}.jsonl", cases))
        md = (target / "report.md").read_text(encoding="utf-8")
        js = json.loads((target / "report.json").read_text(encoding="utf-8"))
        return {"rep": rep, "md": md, "json": js, "dir": str(target)}

    ok_cases = [_case("T-001", "PASS"), _case("T-002", "PASS", evidence="6F00")]
    r = gen("allpass", "validate-sacrificial", "hw", ok_cases)
    check("all-pass hw -> PASS", r["rep"]["verdict"] == "PASS")
    check("all-pass hw has no dry-run banner", "DRY-RUN" not in r["md"])
    check("sacrificial has no gate", r["rep"]["gate"] is None)

    r = gen("onefail", "validate-sacrificial", "hw", [_case("T-001", "PASS"), _case("T-002", "FAIL", evidence="6A87")])
    check("one fail -> FAIL", r["rep"]["verdict"] == "FAIL")

    r = gen("allskip", "validate-sacrificial", "hw", [_case("T-001", "SKIP", detail="lpac absent")])
    check("all skip -> SKIP", r["rep"]["verdict"] == "SKIP")
    check("all skip says NOT EXECUTED", "NOT EXECUTED" in r["md"])

    r = gen("partial", "validate-sacrificial", "hw", [_case("T-001", "PASS"), _case("T-002", "SKIP", detail="x")])
    check("pass+skip is never PASS", r["rep"]["verdict"] != "PASS", r["rep"]["verdict"])

    r = gen("blocked", "validate-sacrificial", "hw", [_case("T-001", "PASS"), _case("T-002", "BLOCKED", detail="transport")])
    check("blocked -> BLOCKED", r["rep"]["verdict"] == "BLOCKED")

    r = gen("gate-dry", "validate-esimgate", "dry-run", ok_cases)
    check("dry-run gate stays closed", r["rep"]["gate"]["provisioning_may_proceed"] is False)
    check("dry-run verdict is annotated", "dry-run" in r["md"] and "NOT a hardware qualification" in r["md"])

    r = gen("gate-hw-green", "validate-esimgate", "hw", ok_cases)
    check("hw all-pass opens gate", r["rep"]["gate"]["provisioning_may_proceed"] is True)
    r = gen("gate-hw-red", "validate-esimgate", "hw", [_case("T-001", "FAIL", evidence="6A82")])
    check("hw fail closes gate", r["rep"]["gate"]["provisioning_may_proceed"] is False)

    try:
        load_cases(write_jsonl("badverdict.jsonl", [_case("T-001", "MAYBE")]))
        check("invalid verdict rejected", False)
    except ReportError:
        check("invalid verdict rejected", True)
    try:
        load_cases(write_jsonl("noevidence.jsonl", [
            {"id": "T-001", "name": "x", "verdict": "PASS", "observed": "", "evidence": {}}
        ]))
        check("PASS without evidence rejected", False)
    except ReportError:
        check("PASS without evidence rejected", True)

    failed = [c for c in checks if not c[1]]
    for name, ok, info in checks:
        print(f"  {'ok' if ok else 'FAIL'}: {name}" + (f" ({info})" if info and not ok else ""))
    shutil.rmtree(tmp)
    print(f"report.py self-test: {len(checks) - len(failed)}/{len(checks)} PASS")
    return 1 if failed else 0


def main(argv: list[str]) -> int:
    if argv[:1] == ["--self-test"]:
        return self_test()
    return run_cli(argv)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
