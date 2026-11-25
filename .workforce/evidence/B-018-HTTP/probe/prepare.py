#!/usr/bin/env python3
"""Prepare the B-018-HTTP gate command manifest and immutable input snapshot."""
from __future__ import annotations

import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess

REPO = Path.cwd()
PROBE = REPO / ".workforce/evidence/B-018-HTTP/probe"
BATCH = "B-018-HTTP"
CARD = "C-018-HTTP-SAFE"
AGENT = "gate-llmrust"
SPEC_SHA = "f068a5a470bf219776b1a4d0a95aed64cd18f86ffbd8346004135257e967dcf7"


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


def main() -> None:
    PROBE.mkdir(parents=True, exist_ok=True)
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
    if len(required) != 7:
        raise RuntimeError(f"expected 7 config required_commands, got {len(required)}")
    criteria = {item.split(None, 1)[0]: item for item in card["acceptance_criteria"]}
    c4_commands = re.findall(r"cargo [^、。]+", criteria["c4"])
    c5_commands = re.findall(r"`([^`]+)`", criteria["c5"])
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

    checks = {
        "spec_sha256_matches": actual_spec_sha == SPEC_SHA,
        "config_required_command_count_is_7": len(required) == 7,
        "card_c4_command_count_is_6": len(c4_commands) == 6,
        "card_c5_command_count_is_1": len(c5_commands) == 1,
        "card_gate_agent_matches": card["gate_agent"] == AGENT,
        "loop_card_sha256_matches": task["card_sha256"] == actual_card_sha,
        "loop_spec_sha256_matches": loop["spec_sha256"] == actual_spec_sha,
        "head_equals_batch_base": head == batch_base,
        "qa_verdict_pass": qa.get("verdict") == "PASS",
        "qa_independent": qa.get("independent") is True and qa.get("declared_by_lead") is False,
        "qa_mock_used_false": qa.get("mock_used") is False,
    }
    failed = [name for name, ok in checks.items() if not ok]
    if failed:
        raise RuntimeError("preflight checks failed: " + ", ".join(failed))

    frozen_expected = qa["artifacts"]["frozen_files_sha256"]
    frozen_actual = {}
    frozen_mismatches = []
    for name, expected in frozen_expected.items():
        path = REPO / name
        actual = sha256(path) if path.is_file() else None
        frozen_actual[name] = actual
        if actual != expected:
            frozen_mismatches.append({"path": name, "expected": expected, "actual": actual})
    if frozen_mismatches:
        raise RuntimeError(f"QA-frozen file mismatch: {frozen_mismatches}")

    protected_paths = [
        ".workforce/spec.json",
        ".workforce/config.json",
        f".workforce/cards/{CARD}.json",
        ".workforce/loop-state.json",
    ]
    protected_paths.extend(p.relative_to(REPO).as_posix() for p in sorted((REPO / ".workforce/signoffs").glob("*.json")))
    protected_paths.extend(sorted(frozen_expected))
    protected_paths = list(dict.fromkeys(protected_paths))
    protected_hashes = {name: sha256(REPO / name) for name in protected_paths}

    git_status_before = git_bytes("status", "--porcelain=v1", "-z")
    (PROBE / "git-status-before.bin").write_bytes(git_status_before)
    git_diff_sha_before = hashlib.sha256(git_bytes("diff", "--binary")).hexdigest()

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
        "c4-02-test-all-features",
        "c4-03-build-default",
        "c4-04-fmt",
        "c4-05-clippy",
        "c4-06-rustdoc",
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
        command_file = PROBE / spec["command_file"]
        if command_file.exists():
            raise RuntimeError(f"refusing to overwrite existing command file: {rel(command_file)}")
        command_file.write_bytes(b"@echo off\r\n" + spec["cmd"].encode("utf-8") + b"\r\n")
        for key in ("stdout", "stderr", "exit_code_file"):
            if (PROBE / spec[key]).exists():
                raise RuntimeError(f"refusing to overwrite existing output: {rel(PROBE / spec[key])}")

    manifest = {
        "schema": "agent-workforce-gate-run-manifest/v1",
        "batch_id": BATCH,
        "card_id": CARD,
        "agent": AGENT,
        "written_by": AGENT,
        "spec_sha256": SPEC_SHA,
        "handoff_id": qa["handoff_id"],
        "repo": str(REPO),
        "head": head,
        "batch_base": batch_base,
        "serial": True,
        "execution": "one Windows COMSPEC batch per recorded command; stdout/stderr redirected to separate binary files; no pipeline",
        "expected_command_count": len(specs),
        "commands": specs,
        "prepared_at_utc": utc_now(),
    }
    (PROBE / "run-manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

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
        "handoff_id": qa["handoff_id"],
        "qa_verdict": qa["verdict"],
        "head": head,
        "batch_base": batch_base,
        "checks": checks,
        "frozen_files": frozen_actual,
        "frozen_files_match_qa": not frozen_mismatches,
        "protected_paths": protected_paths,
        "protected_sha256": protected_hashes,
        "git_status_before": {
            "path": ".workforce/evidence/B-018-HTTP/probe/git-status-before.bin",
            "sha256": hashlib.sha256(git_status_before).hexdigest(),
            "bytes": len(git_status_before),
        },
        "git_diff_binary_sha256": git_diff_sha_before,
        "toolchain": {
            "cargo": shutil.which("cargo"),
            "python": shutil.which("python"),
            "git": shutil.which("git"),
            "comspec": os.environ.get("COMSPEC"),
        },
        "prepared_at_utc": utc_now(),
    }
    (PROBE / "preflight.json").write_text(json.dumps(preflight, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({
        "preflight": rel(PROBE / "preflight.json"),
        "manifest": rel(PROBE / "run-manifest.json"),
        "expected_command_count": len(specs),
        "all_checks_pass": all(checks.values()),
        "frozen_files_match_qa": not frozen_mismatches,
    }, ensure_ascii=False))


if __name__ == "__main__":
    main()
