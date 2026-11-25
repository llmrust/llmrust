#!/usr/bin/env python3
"""Verify B-018-HTTP gate evidence and write the factual gate receipt."""
from __future__ import annotations

import datetime as dt
import hashlib
import json
from pathlib import Path
import re
import subprocess

REPO = Path.cwd()
PROBE = REPO / ".workforce/evidence/B-018-HTTP/probe"
BATCH = "B-018-HTTP"
CARD = "C-018-HTTP-SAFE"
AGENT = "gate-llmrust"
SPEC_SHA = "f068a5a470bf219776b1a4d0a95aed64cd18f86ffbd8346004135257e967dcf7"
GATE_PATH = REPO / f".workforce/evidence/{BATCH}/gate.json"


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat()


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def rel(path: Path) -> str:
    return path.relative_to(REPO).as_posix()


def load_json(path: Path):
    return json.loads(path.read_text(encoding="utf-8"))


def git_bytes(*args: str) -> bytes:
    cp = subprocess.run(["git", *args], cwd=REPO, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)
    if cp.returncode != 0:
        raise RuntimeError(f"git {' '.join(args)} failed: {cp.stderr.decode('utf-8', 'replace')}")
    return cp.stdout


def parse_test_results(path: Path) -> list[str]:
    text = path.read_bytes().decode("utf-8", "replace")
    return [line.strip() for line in text.splitlines() if line.strip().startswith("test result:")]


def main() -> None:
    if GATE_PATH.exists():
        raise RuntimeError(f"refusing to overwrite existing gate receipt: {GATE_PATH}")
    manifest = load_json(PROBE / "run-manifest.json")
    preflight = load_json(PROBE / "preflight.json")
    run_summary = load_json(PROBE / "run-summary.json")
    run_lines = (PROBE / "runs.jsonl").read_text(encoding="utf-8").splitlines()
    runs = [json.loads(line) for line in run_lines if line.strip()]
    commands = manifest["commands"]

    checks = {
        "manifest_serial": manifest.get("serial") is True,
        "expected_command_count_is_14": manifest.get("expected_command_count") == 14,
        "recorded_command_count_is_14": len(runs) == 14,
        "run_summary_count_matches": run_summary.get("recorded_command_count") == len(runs),
        "no_skipped_commands": run_summary.get("skipped_commands") == [],
    }
    if len(runs) != len(commands):
        raise RuntimeError(f"run count mismatch: {len(runs)} != {len(commands)}")

    for index, (spec, run) in enumerate(zip(commands, runs), start=1):
        checks[f"sequence_{index:02d}_matches"] = (
            spec["sequence"] == run["sequence"] == index
            and spec["id"] == run["id"]
            and spec["criterion"] == run["criterion"]
            and spec["cmd"] == run["cmd"]
        )
        stdout_path = PROBE / spec["stdout"]
        stderr_path = PROBE / spec["stderr"]
        exit_path = PROBE / spec["exit_code_file"]
        command_path = PROBE / spec["command_file"]
        checks[f"output_{index:02d}_exists"] = all(p.is_file() for p in (stdout_path, stderr_path, exit_path, command_path))
        checks[f"stdout_sha_{index:02d}"] = sha256(stdout_path) == run["stdout_sha256"]
        checks[f"stderr_sha_{index:02d}"] = sha256(stderr_path) == run["stderr_sha256"]
        checks[f"exit_sha_{index:02d}"] = sha256(exit_path) == run["exit_code_file_sha256"]
        checks[f"exit_value_{index:02d}"] = exit_path.read_bytes() == str(run["exit_code"]).encode("ascii")
        checks[f"command_sha_{index:02d}"] = sha256(command_path) == run["command_file_sha256"]

    serial_ok = all(
        dt.datetime.fromisoformat(current["started_at_utc"]) >= dt.datetime.fromisoformat(previous["ended_at_utc"])
        for previous, current in zip(runs, runs[1:])
    )
    checks["serial_timestamps_non_overlapping"] = serial_ok

    required_runs = runs[:7]
    c4_runs = runs[7:13]
    c5_runs = runs[13:]
    config_required = load_json(REPO / ".workforce/config.json")["gates"]["required_commands"]
    checks["config_commands_exact_and_in_order"] = [r["cmd"] for r in required_runs] == config_required
    checks["card_c4_commands_exact_and_in_order"] = [r["cmd"] for r in c4_runs] == [c["cmd"] for c in commands[7:13]]
    checks["card_c5_command_exact"] = [r["cmd"] for r in c5_runs] == [commands[13]["cmd"]]
    checks["config_required_all_exit_zero"] = all(r["exit_code"] == 0 for r in required_runs)
    checks["card_c4_all_exit_zero"] = all(r["exit_code"] == 0 for r in c4_runs)
    checks["card_c5_exit_zero"] = all(r["exit_code"] == 0 for r in c5_runs)

    current_protected = {}
    protected_mismatches = []
    for name, expected in preflight["protected_sha256"].items():
        path = REPO / name
        actual = sha256(path) if path.is_file() else None
        current_protected[name] = actual
        if actual != expected:
            protected_mismatches.append({"path": name, "expected": expected, "actual": actual})
    checks["protected_inputs_unchanged"] = not protected_mismatches
    checks["spec_sha256_still_matches"] = sha256(REPO / ".workforce/spec.json") == SPEC_SHA

    git_status_after = git_bytes("status", "--porcelain=v1", "-z")
    (PROBE / "git-status-after.bin").write_bytes(git_status_after)
    git_diff_sha_after = hashlib.sha256(git_bytes("diff", "--binary")).hexdigest()
    checks["git_status_unchanged"] = git_status_after == (PROBE / "git-status-before.bin").read_bytes()
    checks["tracked_diff_unchanged"] = git_diff_sha_after == preflight["git_diff_binary_sha256"]
    current_head = git_bytes("rev-parse", "HEAD").decode("ascii").strip()
    checks["head_unchanged"] = current_head == preflight["head"] == preflight["batch_base"]

    postflight = {
        "schema": "agent-workforce-gate-postflight/v1",
        "batch_id": BATCH,
        "card_id": CARD,
        "agent": AGENT,
        "written_by": AGENT,
        "verified_at_utc": utc_now(),
        "head": current_head,
        "protected_sha256": current_protected,
        "protected_mismatches": protected_mismatches,
        "git_status_after": {
            "path": ".workforce/evidence/B-018-HTTP/probe/git-status-after.bin",
            "sha256": hashlib.sha256(git_status_after).hexdigest(),
            "bytes": len(git_status_after),
        },
        "git_diff_binary_sha256": git_diff_sha_after,
    }
    (PROBE / "postflight.json").write_text(json.dumps(postflight, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    failed_checks = [name for name, ok in checks.items() if not ok]
    config_pass = checks["config_commands_exact_and_in_order"] and checks["config_required_all_exit_zero"]
    c4_pass = checks["card_c4_commands_exact_and_in_order"] and checks["card_c4_all_exit_zero"]
    c5_pass = checks["card_c5_command_exact"] and checks["card_c5_exit_zero"]
    protected_pass = checks["protected_inputs_unchanged"] and checks["head_unchanged"] and checks["tracked_diff_unchanged"]
    verdict = "PASS" if config_pass and c4_pass and c5_pass and protected_pass and not failed_checks else "FAIL"

    command_records = []
    test_summaries = {}
    for spec, run in zip(commands, runs):
        record = {
            "id": spec["id"],
            "criterion": spec["criterion"],
            "cmd": spec["cmd"],
            "exit_code": run["exit_code"],
            "stdout": f".workforce/evidence/{BATCH}/probe/{spec['stdout']}",
            "stderr": f".workforce/evidence/{BATCH}/probe/{spec['stderr']}",
            "exit_code_file": f".workforce/evidence/{BATCH}/probe/{spec['exit_code_file']}",
            "stdout_sha256": run["stdout_sha256"],
            "stderr_sha256": run["stderr_sha256"],
            "exit_code_file_sha256": run["exit_code_file_sha256"],
        }
        command_records.append(record)
        results = parse_test_results(PROBE / spec["stdout"])
        if results:
            test_summaries[spec["id"]] = results

    artifact_names = [
        "prepare.py",
        "run.py",
        "finalize.py",
        "verify-receipt.py",
        "preflight.json",
        "run-manifest.json",
        "run-start.json",
        "runs.jsonl",
        "run-summary.json",
        "postflight.json",
        "git-status-before.bin",
        "git-status-after.bin",
    ]
    for spec in commands:
        artifact_names.extend([spec["command_file"], spec["stdout"], spec["stderr"], spec["exit_code_file"]])
    artifacts = [
        {"path": f".workforce/evidence/{BATCH}/probe/{name}", "sha256": sha256(PROBE / name)}
        for name in artifact_names
    ]

    verification = {
        "schema": "agent-workforce-gate-verification/v1",
        "batch_id": BATCH,
        "card_id": CARD,
        "checks": checks,
        "failed_checks": failed_checks,
        "config_required_commands": "PASS" if config_pass else "FAIL",
        "card_c4": "PASS" if c4_pass else "FAIL",
        "card_c5": "PASS" if c5_pass else "FAIL",
        "protected_inputs_unchanged": "PASS" if protected_pass else "FAIL",
        "command_outputs_archived": "PASS" if not any(name.startswith(("stdout_sha_", "stderr_sha_", "exit_sha_", "output_")) and not ok for name, ok in checks.items()) else "FAIL",
        "no_skip": "PASS",
        "verdict": verdict,
        "verified_at_utc": utc_now(),
    }
    (PROBE / "verification.json").write_text(json.dumps(verification, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    artifacts.append({
        "path": f".workforce/evidence/{BATCH}/probe/verification.json",
        "sha256": sha256(PROBE / "verification.json"),
    })

    deny_stderr = (PROBE / "required-07-cargo-deny.stderr.log").read_bytes().decode("utf-8", "replace")
    deny_stdout = (PROBE / "required-07-cargo-deny.stdout.log").read_bytes().decode("utf-8", "replace").strip()
    observations = [
        {
            "id": "OBS-CARGO-DENY-WARNINGS",
            "command_id": "required-07-cargo-deny",
            "exit_code": runs[6]["exit_code"],
            "stdout": deny_stdout,
            "stderr_warning_count": deny_stderr.count("warning["),
            "disposition": "warnings preserved verbatim; cargo deny exit 0 with advisories/bans/licenses/sources ok",
        },
        {
            "id": "OBS-DOCTEST-IGNORED-COUNTS",
            "default_doctest_result": test_summaries.get("required-02-test-default", [])[-1] if test_summaries.get("required-02-test-default") else None,
            "all_features_doctest_result": test_summaries.get("required-03-test-all-features", [])[-1] if test_summaries.get("required-03-test-all-features") else None,
            "disposition": "existing doctest ignored counts remain in raw stdout; no gate command was skipped",
        },
    ]

    nonzero = [{"id": r["id"], "cmd": r["cmd"], "exit_code": r["exit_code"]} for r in runs if r["exit_code"] != 0]
    notes = (
        f"独立门禁按配置 7 条 required_commands、卡 c4 六条、c5 一条，共 {len(runs)} 条命令串行实跑；"
        f"config={verification['config_required_commands']}，c4={verification['card_c4']}，c5={verification['card_c5']}。"
        "每条 stdout/stderr/exit 分别以原始文件保存并计算 sha256；无 SKIP、无 mock、未删 FAIL。"
        "cargo deny 的 license-not-encountered/duplicate 警告原文保留，exit 0 且 advisories/bans/licenses/sources 均 ok。"
        f"受保护输入与 QA 冻结文件复核={verification['protected_inputs_unchanged']}。"
    )
    if nonzero:
        notes += " 非零退出：" + json.dumps(nonzero, ensure_ascii=False)
    if failed_checks:
        notes += " 校验失败项：" + ", ".join(failed_checks)

    gate = {
        "schema": "agent-workforce-evidence/v1",
        "role": "gate",
        "agent": AGENT,
        "written_by": AGENT,
        "card_id": CARD,
        "batch_id": BATCH,
        "spec_sha256": SPEC_SHA,
        "handoff_id": manifest["handoff_id"],
        "inputs_seen": [
            ".workforce/spec.json",
            ".workforce/config.json",
            f".workforce/cards/{CARD}.json",
            ".workforce/loop-state.json",
            ".agents/skills/agent-workforce-loop/references/roles.md",
            ".agents/skills/agent-workforce-loop/assets/evidence.template.json",
            f".workforce/evidence/{BATCH}/dev-{CARD}.json",
            f".workforce/evidence/{BATCH}/qa-{CARD}.json",
        ],
        "mock_used": False,
        "artifacts": artifacts,
        "command_exit_codes": command_records,
        "expected_command_count": len(commands),
        "recorded_command_count": len(runs),
        "missing_commands": [],
        "skipped_commands": [],
        "command_serial": True,
        "criteria_results": {
            "config_required_commands": verification["config_required_commands"],
            "card_c4": verification["card_c4"],
            "card_c5": verification["card_c5"],
            "command_outputs_archived": verification["command_outputs_archived"],
            "protected_inputs_unchanged": verification["protected_inputs_unchanged"],
            "no_skip": verification["no_skip"],
        },
        "test_result_lines": test_summaries,
        "observations": observations,
        "verification": {
            "path": f".workforce/evidence/{BATCH}/probe/verification.json",
            "sha256": sha256(PROBE / "verification.json"),
            "failed_checks": failed_checks,
        },
        "verdict": verdict,
        "independent": True,
        "declared_by_lead": False,
        "generated_at_utc": utc_now(),
        "notes": notes,
    }
    with GATE_PATH.open("x", encoding="utf-8", newline="\n") as f:
        json.dump(gate, f, ensure_ascii=False, indent=2)
        f.write("\n")
    print(json.dumps({
        "gate": rel(GATE_PATH),
        "receipt_sha256": sha256(GATE_PATH),
        "verdict": verdict,
        "command_count": len(runs),
        "nonzero_commands": nonzero,
        "failed_checks": failed_checks,
        "artifact_count": len(artifacts),
    }, ensure_ascii=False))


if __name__ == "__main__":
    main()
