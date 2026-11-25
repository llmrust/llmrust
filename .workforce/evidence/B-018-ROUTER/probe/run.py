#!/usr/bin/env python3
"""Run every B-018-ROUTER gate command serially and preserve raw streams."""
from __future__ import annotations

import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

REPO = Path.cwd()
PROBE = REPO / ".workforce/evidence/B-018-ROUTER/probe"
MANIFEST_PATH = PROBE / "run-manifest.json"
RUNS_PATH = PROBE / "runs.jsonl"


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat()


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def main() -> int:
    manifest = json.loads(MANIFEST_PATH.read_text(encoding="utf-8"))
    commands = manifest["commands"]
    if manifest.get("serial") is not True:
        raise RuntimeError("manifest is not marked serial")
    if len(commands) != manifest["expected_command_count"]:
        raise RuntimeError("manifest command count mismatch")
    if RUNS_PATH.exists():
        raise RuntimeError(f"refusing to overwrite existing run log: {RUNS_PATH}")

    comspec = os.environ.get("COMSPEC") or r"C:\Windows\System32\cmd.exe"
    start = {
        "schema": "agent-workforce-gate-run-start/v1",
        "pid": os.getpid(),
        "started_at_utc": utc_now(),
        "serial": True,
        "expected_command_count": len(commands),
        "comspec": comspec,
    }
    (PROBE / "run-start.json").write_text(json.dumps(start, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    runs = []
    with RUNS_PATH.open("w", encoding="utf-8", newline="\n") as run_log:
        for spec in commands:
            stdout_path = PROBE / spec["stdout"]
            stderr_path = PROBE / spec["stderr"]
            exit_path = PROBE / spec["exit_code_file"]
            command_path = PROBE / spec["command_file"]
            for path in (stdout_path, stderr_path, exit_path):
                if path.exists():
                    raise RuntimeError(f"refusing to overwrite existing output: {path}")
            expected_batch = b"@echo off\r\n" + spec["cmd"].encode("utf-8") + b"\r\n"
            if command_path.read_bytes() != expected_batch:
                raise RuntimeError(f"command file does not match manifest: {command_path}")

            started_at = utc_now()
            with stdout_path.open("wb") as stdout_file, stderr_path.open("wb") as stderr_file:
                cp = subprocess.run(
                    [comspec, "/d", "/s", "/c", str(command_path)],
                    cwd=REPO,
                    stdout=stdout_file,
                    stderr=stderr_file,
                    check=False,
                )
            ended_at = utc_now()
            exit_path.write_bytes(str(cp.returncode).encode("ascii"))
            run = {
                "sequence": spec["sequence"],
                "id": spec["id"],
                "criterion": spec["criterion"],
                "cmd": spec["cmd"],
                "started_at_utc": started_at,
                "ended_at_utc": ended_at,
                "exit_code": cp.returncode,
                "stdout": spec["stdout"],
                "stderr": spec["stderr"],
                "exit_code_file": spec["exit_code_file"],
                "command_file": spec["command_file"],
                "stdout_bytes": stdout_path.stat().st_size,
                "stderr_bytes": stderr_path.stat().st_size,
                "stdout_sha256": sha256(stdout_path),
                "stderr_sha256": sha256(stderr_path),
                "exit_code_file_sha256": sha256(exit_path),
                "command_file_sha256": sha256(command_path),
            }
            runs.append(run)
            run_log.write(json.dumps(run, ensure_ascii=False) + "\n")
            run_log.flush()
            print(f"[{spec['sequence']:02d}/{len(commands)}] {spec['id']} exit={cp.returncode} stdout={run['stdout_bytes']}B stderr={run['stderr_bytes']}B", flush=True)

    for previous, current in zip(runs, runs[1:]):
        if dt.datetime.fromisoformat(current["started_at_utc"]) < dt.datetime.fromisoformat(previous["ended_at_utc"]):
            raise RuntimeError(f"serial ordering violated between {previous['id']} and {current['id']}")

    summary = {
        "schema": "agent-workforce-gate-run-summary/v1",
        "started_at_utc": start["started_at_utc"],
        "finished_at_utc": utc_now(),
        "serial": True,
        "serial_order_verified": True,
        "expected_command_count": len(commands),
        "recorded_command_count": len(runs),
        "exit_codes": {run["id"]: run["exit_code"] for run in runs},
        "all_exit_zero": all(run["exit_code"] == 0 for run in runs),
        "skipped_commands": [],
    }
    (PROBE / "run-summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(summary, ensure_ascii=False), flush=True)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        print(f"gate runner infrastructure error: {exc}", file=sys.stderr)
        raise
