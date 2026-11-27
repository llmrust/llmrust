#!/usr/bin/env python3
"""Run the workforce skill's required read-only preflight checks and archive them."""
from __future__ import annotations

import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import subprocess

REPO = Path.cwd()
PROBE = REPO / ".workforce/evidence/B-018-ROUTER/probe"
SKILL = REPO / ".agents/skills/agent-workforce-loop"


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat()


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> int:
    PROBE.mkdir(parents=True, exist_ok=True)
    checks = [
        ("validate-skill", ["node", str(SKILL / "scripts/validate-skill.mjs")]),
        ("selftest", ["node", str(SKILL / "scripts/selftest.mjs")]),
        ("loop-status", ["node", str(SKILL / "scripts/loop.mjs"), "status", "--repo", str(REPO)]),
    ]
    records = []
    for name, argv in checks:
        stdout_path = PROBE / f"skill-{name}.stdout.log"
        stderr_path = PROBE / f"skill-{name}.stderr.log"
        exit_path = PROBE / f"skill-{name}.exit.txt"
        for path in (stdout_path, stderr_path, exit_path):
            if path.exists():
                raise RuntimeError(f"refusing to overwrite {path}")
        started = utc_now()
        with stdout_path.open("wb") as out, stderr_path.open("wb") as err:
            cp = subprocess.run(argv, cwd=REPO, stdout=out, stderr=err, check=False)
        ended = utc_now()
        exit_path.write_bytes(str(cp.returncode).encode("ascii"))
        record = {
            "id": name,
            "argv": argv,
            "started_at_utc": started,
            "ended_at_utc": ended,
            "exit_code": cp.returncode,
            "stdout": stdout_path.relative_to(REPO).as_posix(),
            "stderr": stderr_path.relative_to(REPO).as_posix(),
            "exit_code_file": exit_path.relative_to(REPO).as_posix(),
            "stdout_sha256": sha256(stdout_path),
            "stderr_sha256": sha256(stderr_path),
            "exit_code_file_sha256": sha256(exit_path),
        }
        records.append(record)
        print(f"{name}: exit={cp.returncode}", flush=True)
    summary = {
        "schema": "agent-workforce-gate-skill-preflight/v1",
        "checked_at_utc": utc_now(),
        "checks": records,
        "all_exit_zero": all(r["exit_code"] == 0 for r in records),
        "note": "Auxiliary skill checks; not part of the 13 required gate commands.",
    }
    (PROBE / "skill-preflight.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(summary, ensure_ascii=False), flush=True)
    return 0 if summary["all_exit_zero"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
