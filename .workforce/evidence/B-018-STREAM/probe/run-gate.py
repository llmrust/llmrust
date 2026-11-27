#!/usr/bin/env python3
"""B-018-STREAM gate: preflight, 14 serial commands, evidence, factual receipt."""
from __future__ import annotations

import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

REPO = Path.cwd()
PROBE = REPO / ".workforce/evidence/B-018-STREAM/probe"
BATCH = "B-018-STREAM"
CARD = "C-018-STREAM-CORE"
AGENT = "gate-llmrust"
SPEC_SHA = "b49068ecf7fc157d3edd5cca3cea5da0eb331978414e30e1ecccae71407f0c94"
HANDOFF = "ece5291caa237b095b4515f9"
QA_RECEIPT_SHA = "72f3ea4a851411df85ccf875fd7cb32e19dfc66e5a61e02057ba2a9b32c01a57"
GATE_PATH = REPO / f".workforce/evidence/{BATCH}/gate.json"
MANIFEST_PATH = PROBE / "run-manifest.json"
PREFLIGHT_PATH = PROBE / "preflight.json"
RUNS_PATH = PROBE / "runs.jsonl"
SUMMARY_PATH = PROBE / "run-summary.json"
POSTFLIGHT_PATH = PROBE / "postflight.json"
VERIFICATION_PATH = PROBE / "verification.json"


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
    cp = subprocess.run(
        ["git", *args], cwd=REPO, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False
    )
    if cp.returncode != 0:
        raise RuntimeError(
            f"git {' '.join(args)} failed ({cp.returncode}): "
            + cp.stderr.decode("utf-8", "replace")
        )
    return cp.stdout


def filtered_status(raw: bytes) -> bytes:
    """Remove only this gate's own untracked evidence from a -z status snapshot."""
    kept = []
    for entry in raw.split(b"\0"):
        if not entry:
            continue
        path = entry[3:] if len(entry) >= 3 else entry
        path_text = path.decode("utf-8", "surrogateescape")
        if path_text.startswith(f".workforce/evidence/{BATCH}/probe/"):
            continue
        if path_text == f".workforce/evidence/{BATCH}/gate.json":
            continue
        kept.append(entry)
    return b"\0".join(kept) + (b"\0" if kept else b"")


def parse_test_results(path: Path) -> list[str]:
    text = path.read_bytes().decode("utf-8", "replace")
    return [line.strip() for line in text.splitlines() if line.strip().startswith("test result:")]


def prepare() -> None:
    if GATE_PATH.exists():
        raise RuntimeError(f"refusing to overwrite existing gate receipt: {GATE_PATH}")
    for path in (MANIFEST_PATH, PREFLIGHT_PATH, RUNS_PATH, SUMMARY_PATH):
        if path.exists():
            raise RuntimeError(f"refusing to overwrite existing evidence: {rel(path)}")

    config_path = REPO / ".workforce/config.json"
    card_path = REPO / f".workforce/cards/{CARD}.json"
    spec_path = REPO / ".workforce/spec.json"
    loop_path = REPO / ".workforce/loop-state.json"
    dev_path = REPO / f".workforce/evidence/{BATCH}/dev-{CARD}.json"
    qa_path = REPO / f".workforce/evidence/{BATCH}/qa-{CARD}.json"

    config = load_json(config_path)
    card = load_json(card_path)
    loop = load_json(loop_path)
    dev = load_json(dev_path)
    qa = load_json(qa_path)

    required = config["gates"]["required_commands"]
    criteria = {item.split(None, 1)[0]: item for item in card["acceptance_criteria"]}
    c4_commands = re.findall(r"cargo [^、。]+", criteria["c4"])
    c5_commands = re.findall(r"`([^`]+)`", criteria["c5"])
    if len(required) != 7:
        raise RuntimeError(f"expected 7 config required_commands, got {len(required)}")
    if len(c4_commands) != 6:
        raise RuntimeError(f"expected 6 c4 commands, got {len(c4_commands)}: {c4_commands}")
    if len(c5_commands) != 1 or not c5_commands[0].startswith("python -c "):
        raise RuntimeError(f"expected one c5 python command, got {c5_commands}")

    actual_spec_sha = sha256(spec_path)
    actual_card_sha = sha256(card_path)
    batch = next(b for b in loop["batches"] if b["id"] == BATCH)
    task = next(t for t in batch["tasks"] if t["card_id"] == CARD)
    head = git_bytes("rev-parse", "HEAD").decode("ascii").strip()
    batch_base = batch["batch_base"]
    base_head = batch.get("base_head", batch_base)
    handoff = batch["handoffs"][CARD]

    qa_artifacts = {a["path"]: a["sha256"] for a in qa.get("artifacts", [])}
    handoff_files = handoff.get("files", {})
    frozen_paths = sorted(
        set(qa_artifacts)
        | set(handoff_files)
        | {a["path"] for a in dev.get("artifacts", [])}
    )
    frozen_actual = {}
    frozen_mismatches = []
    for name in frozen_paths:
        path = REPO / name
        actual = sha256(path) if path.is_file() else None
        frozen_actual[name] = actual
        expected_values = []
        if name in qa_artifacts:
            expected_values.append(qa_artifacts[name])
        if name in handoff_files:
            expected_values.append(handoff_files[name])
        if isinstance(dev.get("artifacts"), list):
            expected_values.extend(a["sha256"] for a in dev["artifacts"] if a.get("path") == name)
        if any(actual != expected for expected in expected_values):
            frozen_mismatches.append({"path": name, "expected": expected_values, "actual": actual})

    protected_paths = [
        ".workforce/spec.json",
        ".workforce/config.json",
        f".workforce/cards/{CARD}.json",
        ".workforce/loop-state.json",
        ".workforce/acceptors.json",
        rel(dev_path),
        rel(qa_path),
    ]
    protected_paths.extend(
        p.relative_to(REPO).as_posix() for p in sorted((REPO / ".workforce/signoffs").glob("*.json"))
    )
    protected_paths.extend(frozen_paths)
    protected_paths = list(dict.fromkeys(protected_paths))
    protected_hashes = {name: sha256(REPO / name) for name in protected_paths}

    checks = {
        "spec_sha256_matches": actual_spec_sha == SPEC_SHA,
        "config_required_command_count_is_7": len(required) == 7,
        "card_c4_command_count_is_6": len(c4_commands) == 6,
        "card_c5_command_count_is_1": len(c5_commands) == 1,
        "config_require_no_mock_true": config["gates"].get("require_no_mock") is True,
        "card_gate_agent_matches": card["gate_agent"] == AGENT,
        "loop_card_sha256_matches": task["card_sha256"] == actual_card_sha,
        "loop_spec_sha256_matches": loop["spec_sha256"] == actual_spec_sha,
        "head_equals_base_head": head == base_head,
        "qa_verdict_pass": qa.get("verdict") == "PASS",
        "qa_receipt_sha256_matches": sha256(qa_path) == QA_RECEIPT_SHA,
        "qa_independent": qa.get("independent") is True and qa.get("declared_by_lead") is False,
        "qa_mock_used_false": qa.get("mock_used") is False,
        "qa_handoff_matches": qa.get("handoff_id") == HANDOFF == handoff.get("handoff_id"),
        "dev_receipt_matches_handoff": sha256(dev_path) == handoff.get("receipt_sha256"),
        "qa_frozen_files_match": not frozen_mismatches,
    }
    failed = [name for name, ok in checks.items() if not ok]
    if failed:
        raise RuntimeError("preflight checks failed: " + ", ".join(failed) + f"; frozen_mismatches={frozen_mismatches}")

    git_status_before = git_bytes("status", "--porcelain=v1", "-z", "--untracked-files=all")
    (PROBE / "git-status-before.bin").write_bytes(git_status_before)
    diff_pathspec = [
        ".",
        f":(exclude).workforce/evidence/{BATCH}/gate.json",
        f":(exclude).workforce/evidence/{BATCH}/probe/**",
    ]
    git_diff_sha_before = hashlib.sha256(git_bytes("diff", "--binary", "--", *diff_pathspec)).hexdigest()
    git_cached_diff_sha_before = hashlib.sha256(git_bytes("diff", "--cached", "--binary")).hexdigest()

    specs = []
    required_ids = [
        "required-01-build",
        "required-02-test-default",
        "required-03-test-all-features",
        "required-04-fmt",
        "required-05-clippy",
        "required-06-rustdoc",
        "required-07-cargo-deny",
    ]
    c4_ids = [
        "c4-01-test-default",
        "c4-02-test-proxy",
        "c4-03-test-all-features",
        "c4-04-build-default",
        "c4-05-fmt",
        "c4-06-clippy",
    ]
    for idx, (cmd_id, cmd) in enumerate(zip(required_ids, required), start=1):
        specs.append({
            "sequence": len(specs) + 1,
            "id": cmd_id,
            "criterion": f"config.gates.required_commands[{idx - 1}]",
            "cmd": cmd,
            "stdout": f"{cmd_id}.stdout.log",
            "stderr": f"{cmd_id}.stderr.log",
            "exit_code_file": f"{cmd_id}.exit.txt",
            "command_file": f"{cmd_id}.cmd",
        })
    for idx, (cmd_id, cmd) in enumerate(zip(c4_ids, c4_commands), start=1):
        specs.append({
            "sequence": len(specs) + 1,
            "id": cmd_id,
            "criterion": f"{CARD}.acceptance_criteria.c4[{idx - 1}]",
            "cmd": cmd,
            "stdout": f"{cmd_id}.stdout.log",
            "stderr": f"{cmd_id}.stderr.log",
            "exit_code_file": f"{cmd_id}.exit.txt",
            "command_file": f"{cmd_id}.cmd",
        })
    specs.append({
        "sequence": len(specs) + 1,
        "id": "c5-01-ledger-chain",
        "criterion": f"{CARD}.acceptance_criteria.c5",
        "cmd": c5_commands[0],
        "stdout": "c5-01-ledger-chain.stdout.log",
        "stderr": "c5-01-ledger-chain.stderr.log",
        "exit_code_file": "c5-01-ledger-chain.exit.txt",
        "command_file": "c5-01-ledger-chain.cmd",
    })

    for spec in specs:
        command_path = PROBE / spec["command_file"]
        if command_path.exists():
            raise RuntimeError(f"refusing to overwrite command wrapper: {rel(command_path)}")
        command_path.write_bytes(b"@echo off\r\n" + spec["cmd"].encode("utf-8") + b"\r\n")
        for key in ("stdout", "stderr", "exit_code_file"):
            if (PROBE / spec[key]).exists():
                raise RuntimeError(f"refusing to overwrite output: {rel(PROBE / spec[key])}")

    manifest = {
        "schema": "agent-workforce-gate-run-manifest/v1",
        "batch_id": BATCH,
        "card_id": CARD,
        "agent": AGENT,
        "written_by": AGENT,
        "spec_sha256": SPEC_SHA,
        "handoff_id": HANDOFF,
        "repo": str(REPO),
        "head": head,
        "batch_base": batch_base,
        "base_head": base_head,
        "serial": True,
        "execution": "one Windows COMSPEC batch per recorded command; stdout/stderr redirected directly to separate binary files; no pipeline",
        "expected_command_count": len(specs),
        "commands": specs,
        "prepared_at_utc": utc_now(),
    }
    MANIFEST_PATH.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    preflight = {
        "schema": "agent-workforce-gate-preflight/v1",
        "batch_id": BATCH,
        "card_id": CARD,
        "agent": AGENT,
        "written_by": AGENT,
        "spec_sha256": actual_spec_sha,
        "card_sha256": actual_card_sha,
        "config_sha256": sha256(config_path),
        "loop_state_sha256": sha256(loop_path),
        "dev_receipt_sha256": sha256(dev_path),
        "qa_receipt_sha256": sha256(qa_path),
        "handoff_id": HANDOFF,
        "qa_verdict": qa["verdict"],
        "head": head,
        "batch_base": batch_base,
        "base_head": base_head,
        "checks": checks,
        "frozen_files": frozen_actual,
        "frozen_files_match_qa": not frozen_mismatches,
        "protected_paths": protected_paths,
        "protected_sha256": protected_hashes,
        "git_status_before": {
            "path": f".workforce/evidence/{BATCH}/probe/git-status-before.bin",
            "sha256": hashlib.sha256(git_status_before).hexdigest(),
            "bytes": len(git_status_before),
        },
        "git_diff_binary_sha256": git_diff_sha_before,
        "git_cached_diff_binary_sha256": git_cached_diff_sha_before,
        "toolchain": {
            "cargo": shutil.which("cargo"),
            "python": shutil.which("python"),
            "git": shutil.which("git"),
            "comspec": os.environ.get("COMSPEC"),
        },
        "prepared_at_utc": utc_now(),
    }
    PREFLIGHT_PATH.write_text(json.dumps(preflight, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({
        "preflight": rel(PREFLIGHT_PATH),
        "manifest": rel(MANIFEST_PATH),
        "expected_command_count": len(specs),
        "all_checks_pass": all(checks.values()),
        "frozen_files_match_qa": not frozen_mismatches,
    }, ensure_ascii=False), flush=True)


def run_commands() -> None:
    manifest = load_json(MANIFEST_PATH)
    commands = manifest["commands"]
    if manifest.get("serial") is not True:
        raise RuntimeError("manifest is not marked serial")
    if len(commands) != manifest["expected_command_count"] != 14:
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
                    raise RuntimeError(f"refusing to overwrite existing output: {rel(path)}")
            expected_batch = b"@echo off\r\n" + spec["cmd"].encode("utf-8") + b"\r\n"
            if command_path.read_bytes() != expected_batch:
                raise RuntimeError(f"command wrapper does not match manifest: {rel(command_path)}")

            started_at = utc_now()
            started_monotonic = dt.datetime.now(dt.timezone.utc)
            with stdout_path.open("wb") as stdout_file, stderr_path.open("wb") as stderr_file:
                cp = subprocess.run(
                    [comspec, "/d", "/s", "/c", str(command_path)],
                    cwd=REPO,
                    stdout=stdout_file,
                    stderr=stderr_file,
                    check=False,
                )
            ended_monotonic = dt.datetime.now(dt.timezone.utc)
            ended_at = utc_now()
            exit_path.write_bytes(str(cp.returncode).encode("ascii"))
            run = {
                "sequence": spec["sequence"],
                "id": spec["id"],
                "criterion": spec["criterion"],
                "cmd": spec["cmd"],
                "started_at_utc": started_at,
                "ended_at_utc": ended_at,
                "duration_seconds": (ended_monotonic - started_monotonic).total_seconds(),
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
            print(
                f"[{spec['sequence']:02d}/{len(commands)}] {spec['id']} exit={cp.returncode} "
                f"stdout={run['stdout_bytes']}B stderr={run['stderr_bytes']}B "
                f"duration={run['duration_seconds']:.3f}s",
                flush=True,
            )

    serial_ok = all(
        dt.datetime.fromisoformat(current["started_at_utc"]) >= dt.datetime.fromisoformat(previous["ended_at_utc"])
        for previous, current in zip(runs, runs[1:])
    )
    if not serial_ok:
        raise RuntimeError("serial ordering violated")
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
    SUMMARY_PATH.write_text(json.dumps(summary, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(summary, ensure_ascii=False), flush=True)


def finalize() -> None:
    if GATE_PATH.exists():
        raise RuntimeError(f"refusing to overwrite existing gate receipt: {GATE_PATH}")
    manifest = load_json(MANIFEST_PATH)
    preflight = load_json(PREFLIGHT_PATH)
    summary = load_json(SUMMARY_PATH)
    runs = [json.loads(line) for line in RUNS_PATH.read_text(encoding="utf-8").splitlines() if line.strip()]
    commands = manifest["commands"]

    checks = {
        "manifest_serial": manifest.get("serial") is True,
        "expected_command_count_is_14": manifest.get("expected_command_count") == 14,
        "recorded_command_count_is_14": len(runs) == 14,
        "run_summary_count_matches": summary.get("recorded_command_count") == len(runs),
        "run_summary_all_exit_zero_matches": summary.get("all_exit_zero") == all(r["exit_code"] == 0 for r in runs),
        "no_skipped_commands": summary.get("skipped_commands") == [],
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

    checks["serial_timestamps_non_overlapping"] = all(
        dt.datetime.fromisoformat(current["started_at_utc"]) >= dt.datetime.fromisoformat(previous["ended_at_utc"])
        for previous, current in zip(runs, runs[1:])
    )
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

    git_status_after = git_bytes("status", "--porcelain=v1", "-z", "--untracked-files=all")
    (PROBE / "git-status-after.bin").write_bytes(git_status_after)
    git_diff_sha_after = hashlib.sha256(git_bytes("diff", "--binary", "--", ".", f":(exclude).workforce/evidence/{BATCH}/gate.json", f":(exclude).workforce/evidence/{BATCH}/probe/**")).hexdigest()
    git_cached_diff_sha_after = hashlib.sha256(git_bytes("diff", "--cached", "--binary")).hexdigest()
    checks["git_status_unchanged_outside_gate_evidence"] = filtered_status(git_status_after) == filtered_status((PROBE / "git-status-before.bin").read_bytes())
    checks["tracked_diff_unchanged"] = git_diff_sha_after == preflight["git_diff_binary_sha256"]
    checks["tracked_cached_diff_unchanged"] = git_cached_diff_sha_after == preflight["git_cached_diff_binary_sha256"]
    current_head = git_bytes("rev-parse", "HEAD").decode("ascii").strip()
    checks["head_unchanged"] = current_head == preflight["head"] == preflight["base_head"]

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
            "path": f".workforce/evidence/{BATCH}/probe/git-status-after.bin",
            "sha256": hashlib.sha256(git_status_after).hexdigest(),
            "bytes": len(git_status_after),
        },
        "git_diff_binary_sha256": git_diff_sha_after,
        "git_cached_diff_binary_sha256": git_cached_diff_sha_after,
        "tracked_diff_scope": "all tracked changes except gate.json and probe/** outputs intentionally overwritten by this gate; includes ledger.json and all business/spec/card/signoffs/loop-state paths",
    }
    POSTFLIGHT_PATH.write_text(json.dumps(postflight, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    # Command wrappers were execution scaffolding, not retained evidence. Raw streams,
    # exit files, their hashes, the manifest, and the reproducible runner remain.
    for spec in commands:
        (PROBE / spec["command_file"]).unlink()
    checks["temporary_command_wrappers_cleaned"] = all(not (PROBE / spec["command_file"]).exists() for spec in commands)
    checks["no_temporary_probe_subdirectories"] = not any(p.is_dir() for p in PROBE.rglob("*"))

    failed_checks = [name for name, ok in checks.items() if not ok]
    config_pass = checks["config_commands_exact_and_in_order"] and checks["config_required_all_exit_zero"]
    c4_pass = checks["card_c4_commands_exact_and_in_order"] and checks["card_c4_all_exit_zero"]
    c5_pass = checks["card_c5_command_exact"] and checks["card_c5_exit_zero"]
    protected_pass = checks["protected_inputs_unchanged"] and checks["head_unchanged"] and checks["tracked_diff_unchanged"] and checks["tracked_cached_diff_unchanged"] and checks["git_status_unchanged_outside_gate_evidence"]
    verdict = "PASS" if config_pass and c4_pass and c5_pass and protected_pass and not failed_checks else "FAIL"

    command_records = []
    test_summaries = {}
    for spec, run in zip(commands, runs):
        command_records.append({
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
            "command_file_sha256": run["command_file_sha256"],
            "started_at_utc": run["started_at_utc"],
            "ended_at_utc": run["ended_at_utc"],
            "duration_seconds": run["duration_seconds"],
        })
        results = parse_test_results(PROBE / spec["stdout"])
        if results:
            test_summaries[spec["id"]] = results

    artifact_names = [
        "run-gate.py",
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
        artifact_names.extend([spec["stdout"], spec["stderr"], spec["exit_code_file"]])
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
    VERIFICATION_PATH.write_text(json.dumps(verification, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    artifacts.append({
        "path": f".workforce/evidence/{BATCH}/probe/verification.json",
        "sha256": sha256(VERIFICATION_PATH),
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
            "disposition": "warnings preserved verbatim; verdict follows the actual cargo deny exit code and output",
        },
        {
            "id": "OBS-DOCTEST-IGNORED-COUNTS",
            "default_doctest_result": test_summaries.get("required-02-test-default", [])[-1] if test_summaries.get("required-02-test-default") else None,
            "all_features_doctest_result": test_summaries.get("required-03-test-all-features", [])[-1] if test_summaries.get("required-03-test-all-features") else None,
            "disposition": "existing doctest ignored counts remain in raw stdout; no gate command was skipped",
        },
    ]

    successful = [r for r in runs if r["exit_code"] == 0]
    failed_commands = [
        {"id": r["id"], "cmd": r["cmd"], "exit_code": r["exit_code"]}
        for r in runs if r["exit_code"] != 0
    ]
    notes = (
        f"独立门禁按配置 7 条 required_commands、卡 c4 六条、c5 一条，共 {len(runs)} 条命令串行实跑；"
        f"config={verification['config_required_commands']}，c4={verification['card_c4']}，c5={verification['card_c5']}。"
        "每条 stdout/stderr/exit 分别以原始文件保存并计算 sha256；无 SKIP、无 mock、未删 FAIL。"
        "command_exit_codes 仅含最终成功项；全部命令的原始结果另见 command_runs/runs.jsonl。"
        "临时 .cmd 包装已清理，未创建 probe 临时子目录；原始日志、exit、JSON 证据与可复现 runner 保留。"
        f"受保护输入与 QA 冻结文件复核={verification['protected_inputs_unchanged']}。"
    )
    if failed_commands:
        notes += " 非零退出：" + json.dumps(failed_commands, ensure_ascii=False)
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
        "handoff_id": HANDOFF,
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
        "command_exit_codes": [{"cmd": r["cmd"], "exit_code": r["exit_code"]} for r in successful],
        "command_runs": command_records,
        "failed_commands": failed_commands,
        "expected_command_count": len(commands),
        "recorded_command_count": len(runs),
        "successful_command_count": len(successful),
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
            "sha256": sha256(VERIFICATION_PATH),
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
    verify_receipt(expected_artifacts=len(artifacts), expected_successes=len(successful))


def verify_receipt(expected_artifacts: int, expected_successes: int) -> None:
    receipt = load_json(GATE_PATH)
    config = load_json(REPO / ".workforce/config.json")
    errors = []
    if receipt.get("schema") != "agent-workforce-evidence/v1" or receipt.get("role") != "gate":
        errors.append("schema_or_role")
    if receipt.get("agent") != AGENT or receipt.get("written_by") != AGENT:
        errors.append("identity")
    if receipt.get("batch_id") != BATCH or receipt.get("card_id") != CARD:
        errors.append("scope")
    if receipt.get("spec_sha256") != SPEC_SHA or receipt.get("handoff_id") != HANDOFF:
        errors.append("spec_or_handoff")
    if receipt.get("independent") is not True or receipt.get("declared_by_lead") is not False:
        errors.append("independence")
    if receipt.get("mock_used") is not False:
        errors.append("mock_used")
    if receipt.get("skipped_commands") != [] or receipt.get("missing_commands") != []:
        errors.append("skip_or_missing")
    if len(receipt.get("command_runs", [])) != 14 or receipt.get("recorded_command_count") != 14 or receipt.get("expected_command_count") != 14:
        errors.append("command_count")
    required = config["gates"]["required_commands"]
    if [c.get("cmd") for c in receipt.get("command_runs", [])[:7]] != required:
        errors.append("required_commands")
    successful_runs = [r for r in receipt.get("command_runs", []) if r.get("exit_code") == 0]
    success_records = receipt.get("command_exit_codes", [])
    if len(success_records) != expected_successes or len(success_records) != len(successful_runs):
        errors.append("successful_command_count")
    if success_records != [{"cmd": r["cmd"], "exit_code": 0} for r in successful_runs]:
        errors.append("command_exit_codes_not_only_final_successes")
    if any(set(c) != {"cmd", "exit_code"} or c.get("exit_code") != 0 for c in success_records):
        errors.append("command_exit_codes_shape")
    artifacts = receipt.get("artifacts", [])
    if len(artifacts) != expected_artifacts:
        errors.append("artifact_count")
    if any(set(a) != {"path", "sha256"} for a in artifacts):
        errors.append("artifact_shape")
    if any("loop-state" in a.get("path", "") for a in artifacts):
        errors.append("loop_state_in_artifacts")
    artifact_paths = {a["path"] for a in artifacts}
    if len(artifact_paths) != len(artifacts):
        errors.append("duplicate_artifact_path")
    for artifact in artifacts:
        path = REPO / artifact["path"]
        if not path.is_file():
            errors.append("artifact_missing:" + artifact["path"])
        elif sha256(path) != artifact["sha256"]:
            errors.append("artifact_sha:" + artifact["path"])
    for command in receipt.get("command_runs", []):
        for stream in ("stdout", "stderr", "exit_code_file"):
            path = command.get(stream)
            if path not in artifact_paths:
                errors.append(f"stream_not_in_artifacts:{command.get('id')}:{stream}")
            elif sha256(REPO / path) != command.get(stream + "_sha256"):
                errors.append(f"stream_sha_mismatch:{command.get('id')}:{stream}")
    expected_verdict = "PASS" if not errors and all(r.get("exit_code") == 0 for r in receipt.get("command_runs", [])) and receipt.get("verification", {}).get("failed_checks") == [] else "FAIL"
    if receipt.get("verdict") != expected_verdict:
        errors.append("verdict")
    result = {
        "gate_path": rel(GATE_PATH),
        "receipt_sha256": sha256(GATE_PATH),
        "receipt_bytes": GATE_PATH.stat().st_size,
        "artifact_count": len(artifacts),
        "command_count": len(receipt.get("command_runs", [])),
        "successful_command_count": len(success_records),
        "all_exit_zero": all(r.get("exit_code") == 0 for r in receipt.get("command_runs", [])),
        "verdict": receipt.get("verdict"),
        "errors": errors,
        "ok": not errors,
    }
    print(json.dumps(result, ensure_ascii=False, indent=2), flush=True)
    if errors:
        raise RuntimeError("receipt verification failed: " + ", ".join(errors))


def main() -> None:
    mode = sys.argv[1] if len(sys.argv) > 1 else "all"
    if mode in ("prepare", "all"):
        prepare()
    if mode in ("run", "all"):
        run_commands()
    if mode in ("finalize", "all"):
        finalize()


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        print(f"gate infrastructure error: {exc}", file=sys.stderr, flush=True)
        raise
