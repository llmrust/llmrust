#!/usr/bin/env python3
"""Independently re-check the finalized B-018-ROUTER gate receipt."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import re

REPO = Path.cwd()
BATCH = "B-018-ROUTER"
CARD = "C-018-ROUTER-COST"
GATE = REPO / f".workforce/evidence/{BATCH}/gate.json"
SPEC_SHA = "f068a5a470bf219776b1a4d0a95aed64cd18f86ffbd8346004135257e967dcf7"


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def main() -> None:
    receipt = json.loads(GATE.read_text(encoding="utf-8"))
    config = json.loads((REPO / ".workforce/config.json").read_text(encoding="utf-8"))
    card = json.loads((REPO / f".workforce/cards/{CARD}.json").read_text(encoding="utf-8"))
    criteria = {item.split(None, 1)[0]: item for item in card["acceptance_criteria"]}
    errors = []
    if receipt.get("schema") != "agent-workforce-evidence/v1":
        errors.append("schema")
    if receipt.get("role") != "gate":
        errors.append("role")
    if receipt.get("agent") != "gate-llmrust" or receipt.get("written_by") != "gate-llmrust":
        errors.append("identity")
    if receipt.get("batch_id") != BATCH or receipt.get("card_id") != CARD:
        errors.append("scope")
    if receipt.get("spec_sha256") != SPEC_SHA:
        errors.append("spec")
    if receipt.get("independent") is not True or receipt.get("declared_by_lead") is not False:
        errors.append("independence")
    if receipt.get("mock_used") is not False:
        errors.append("mock_used")
    if receipt.get("skipped_commands") != [] or receipt.get("missing_commands") != []:
        errors.append("skip_or_missing")
    if receipt.get("command_serial") is not True:
        errors.append("serial")
    commands = receipt.get("command_exit_codes", [])
    if len(commands) != 13 or receipt.get("recorded_command_count") != 13 or receipt.get("expected_command_count") != 13:
        errors.append("command_count")
    required = config["gates"]["required_commands"]
    if [c.get("cmd") for c in commands[:7]] != required:
        errors.append("required_commands")
    c4 = re.findall(r"cargo [^、。]+", criteria["c4"])
    c5 = re.findall(r"`([^`]+)`", criteria["c5"])
    if [c.get("cmd") for c in commands[7:12]] != c4:
        errors.append("card_c4")
    if [c.get("cmd") for c in commands[12:]] != c5:
        errors.append("card_c5")
    for command in commands:
        if command.get("exit_code") != 0:
            errors.append("nonzero_exit:" + str(command.get("id")))
    for artifact in receipt.get("artifacts", []):
        path = REPO / artifact["path"]
        if not path.is_file():
            errors.append("artifact_missing:" + artifact["path"])
        elif sha256(path) != artifact["sha256"]:
            errors.append("artifact_sha:" + artifact["path"])
    artifact_paths = {a["path"] for a in receipt.get("artifacts", [])}
    artifact_sha = {a["path"]: a["sha256"] for a in receipt.get("artifacts", [])}
    for command in commands:
        for stream in ("stdout", "stderr", "exit_code_file"):
            path = command.get(stream)
            if path not in artifact_paths:
                errors.append(f"stream_not_in_artifacts:{command.get('id')}:{stream}")
            elif path and sha256(REPO / path) != artifact_sha[path]:
                errors.append(f"stream_sha_mismatch:{command.get('id')}:{stream}")
    expected_verdict = "PASS" if not errors else "FAIL"
    if receipt.get("verdict") != expected_verdict:
        errors.append("verdict")
    result = {
        "gate_path": f".workforce/evidence/{BATCH}/gate.json",
        "receipt_sha256": sha256(GATE),
        "receipt_bytes": GATE.stat().st_size,
        "artifact_count": len(receipt.get("artifacts", [])),
        "command_count": len(commands),
        "all_exit_zero": all(c.get("exit_code") == 0 for c in commands),
        "verdict": receipt.get("verdict"),
        "errors": errors,
        "ok": not errors,
    }
    print(json.dumps(result, ensure_ascii=False, indent=2))
    if errors:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
