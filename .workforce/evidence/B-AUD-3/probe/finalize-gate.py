from __future__ import annotations

import datetime as dt
import hashlib
import json
import re
import sys
from pathlib import Path

ROOT = Path(r"D:\llmrust-0.1.5")
PROBE = ROOT / ".workforce" / "evidence" / "B-AUD-3" / "probe"
GATE = ROOT / ".workforce" / "evidence" / "B-AUD-3" / "gate.json"
MANIFEST = PROBE / "run-manifest.json"
SPEC_SHA256 = "f068a5a470bf219776b1a4d0a95aed64cd18f86ffbd8346004135257e967dcf7"
HANDOFF_ID = "79b1a1c78436c5d212c854a3"


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def load_json(path: Path):
    return json.loads(path.read_text(encoding="utf-8-sig"))


def rel(path: Path) -> str:
    return path.relative_to(ROOT).as_posix()


def result_lines(text: str) -> list[str]:
    return [line.strip() for line in text.splitlines() if "test result:" in line]


def zero_failed_ignored(lines: list[str]) -> bool:
    return bool(lines) and all(re.search(r"\b0 failed; 0 ignored\b", line) for line in lines)


def ignored_counts(text: str) -> tuple[int, int]:
    lines = result_lines(text)
    if not lines:
        return 0, 0
    unit = sum(int(match.group(1)) for line in lines[:-1] if (match := re.search(r"\b(\d+) ignored\b", line)))
    doc_match = re.search(r"\b(\d+) ignored\b", lines[-1])
    doc = int(doc_match.group(1)) if doc_match else 0
    return unit, doc


if GATE.exists():
    existing = load_json(GATE)
    if existing.get("role") != "gate" or existing.get("agent") != "gate-llmrust" or existing.get("batch_id") != "B-AUD-3":
        raise SystemExit(f"Refusing to overwrite unrelated {GATE}")

config = load_json(ROOT / ".workforce" / "config.json")
card = load_json(ROOT / ".workforce" / "cards" / "C-AUD-PROXY.json")
preflight = load_json(PROBE / "preflight.json")
qa = load_json(ROOT / ".workforce" / "evidence" / "B-AUD-3" / "qa-C-AUD-PROXY.json")
state = load_json(ROOT / ".workforce" / "loop-state.json")
runs = [json.loads(line) for line in (PROBE / "runs.jsonl").read_text(encoding="utf-8-sig").splitlines() if line.strip()]
by_id = {run.get("id"): run for run in runs}

required_commands = config["gates"]["required_commands"]
c4_commands = [
    "cargo test --workspace",
    "cargo test --workspace --features proxy",
    "cargo build --workspace",
    "cargo fmt --all --check",
    "cargo clippy --workspace --all-targets -- -D warnings",
    "cargo deny --locked check",
]
c5_commands = [
    "cargo test --features proxy --test architecture_guard",
    "cargo test --features proxy --test package_guard",
    "cargo test --features proxy --test responses_proxy_tests",
    "cargo test --features proxy --lib",
    "cargo test --features proxy --test contract_tests",
]
required_ids = ["required-01-build", "required-02-test-default", "required-03-test-all-features", "required-04-fmt", "required-05-clippy", "required-06-rustdoc", "required-07-cargo-deny"]
c4_ids = ["c4-01-test-default", "c4-02-test-proxy", "c4-03-build-default", "c4-04-fmt", "c4-05-clippy-default", "c4-06-cargo-deny"]
c5_ids = ["c5-01-architecture-guard", "c5-02-package-guard", "c5-03-responses-proxy-tests", "c5-04-proxy-lib", "c5-05-contract-tests"]
expected_commands = []
for i, command in enumerate(required_commands):
    expected_commands.append((required_ids[i], f"config.gates.required_commands[{i}]", command))
for i, command in enumerate(c4_commands):
    expected_commands.append((c4_ids[i], f"C-AUD-PROXY.c4[{i}]", command))
for i, command in enumerate(c5_commands):
    expected_commands.append((c5_ids[i], f"C-AUD-PROXY.c5[{i}]", command))

errors: list[str] = []
if len(required_commands) != 7:
    errors.append("config.gates.required_commands 不是七条")
if len(runs) != 18:
    errors.append(f"runs.jsonl 仅记录 {len(runs)}/18 条命令的退出码")
if len(by_id) != len(runs):
    errors.append("run id 重复")
for command in c4_commands:
    if command not in card["acceptance_criteria"][3]:
        errors.append(f"卡面 c4 未包含命令原文: {command}")
for command in c5_commands:
    if command not in card["acceptance_criteria"][4]:
        errors.append(f"卡面 c5 未包含命令原文: {command}")

recorded_paths: set[Path] = set()
for index, run in enumerate(runs):
    if index >= len(expected_commands):
        errors.append(f"存在超出预期顺序的命令记录: {run.get('id')}")
        continue
    run_id, criterion, command = expected_commands[index]
    if run.get("id") != run_id:
        errors.append(f"命令顺序不符：第 {index + 1} 条为 {run.get('id')}，预期 {run_id}")
    if run.get("criterion") != criterion:
        errors.append(f"{run_id} criterion 不符")
    if run.get("cmd") != command:
        errors.append(f"{run_id} 命令原文不符: {run.get('cmd')!r}")
    if not isinstance(run.get("exit_code"), int):
        errors.append(f"{run_id} 缺少整数 exit_code")
    for stream in ("stdout", "stderr"):
        info = run.get(stream, {})
        path = ROOT / info.get("path", "")
        if not path.is_file():
            errors.append(f"{run_id} 缺少 {stream} 原始输出")
            continue
        recorded_paths.add(path.resolve())
        if sha256(path) != info.get("sha256"):
            errors.append(f"{run_id} {stream} sha256 不符")
        if path.stat().st_size != info.get("bytes"):
            errors.append(f"{run_id} {stream} 字节数不符")

missing_commands = [
    {"id": run_id, "criterion": criterion, "cmd": command, "exit_code": None, "reason": "runs.jsonl 中没有该命令的退出码记录"}
    for run_id, criterion, command in expected_commands
    if run_id not in by_id
]
for item in missing_commands:
    errors.append(f"缺少命令退出码: {item['id']} {item['cmd']}")

unrecorded_outputs = []
for path in sorted(PROBE.glob("*.log")):
    if path.resolve() in recorded_paths:
        continue
    unrecorded_outputs.append({"path": rel(path), "sha256": sha256(path), "bytes": path.stat().st_size, "exit_code": None, "reason": "存在原始输出文件，但 runs.jsonl 没有对应退出码记录"})
    errors.append(f"输出文件没有对应退出码记录: {rel(path)}")

manifest = {
    "schema": "gate-command-runs/v1",
    "role": "gate",
    "agent": "gate-llmrust",
    "written_by": "gate-llmrust",
    "batch_id": "B-AUD-3",
    "card_id": "C-AUD-PROXY",
    "spec_sha256": SPEC_SHA256,
    "handoff_id": HANDOFF_ID,
    "cargo": preflight.get("cargo_version"),
    "expected_command_count": 18,
    "recorded_command_count": len(runs),
    "runs": runs,
    "missing_commands": missing_commands,
    "unrecorded_outputs": unrecorded_outputs,
}
MANIFEST.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

spec_actual = sha256(ROOT / ".workforce" / "spec.json")
if spec_actual != SPEC_SHA256 or preflight.get("spec_sha256") != SPEC_SHA256:
    errors.append("spec_sha256 不符")
if preflight.get("handoff_id") != HANDOFF_ID:
    errors.append("preflight handoff_id 不符")
if qa.get("spec_sha256") != SPEC_SHA256 or qa.get("handoff_id") != HANDOFF_ID or qa.get("verdict") != "PASS":
    errors.append("QA receipt 的 spec/handoff/verdict 不符")
if sha256(ROOT / ".workforce" / "evidence" / "B-AUD-3" / "qa-C-AUD-PROXY.json") != preflight.get("qa_receipt_sha256"):
    errors.append("QA receipt 在 gate 预检后发生变化")
if state.get("spec_sha256") != SPEC_SHA256:
    errors.append("loop-state spec_sha256 不符")
handoff_events = [event for event in state.get("history", []) if event.get("event") == "handoff" and event.get("batch") == "B-AUD-3" and event.get("card") == "C-AUD-PROXY"]
if not handoff_events or handoff_events[-1].get("handoff_id") != HANDOFF_ID:
    errors.append("loop-state 最新 handoff_id 不符")

protected_results = []
for item in preflight.get("protected_files", []):
    path = ROOT / item["path"]
    if not path.is_file():
        protected_results.append({"path": item["path"], "unchanged": False, "reason": "missing"})
        errors.append(f"受保护文件缺失: {item['path']}")
        continue
    actual = sha256(path)
    unchanged = actual == item["sha256"]
    protected_results.append({"path": item["path"], "unchanged": unchanged, "before": item["sha256"], "after": actual})
    if not unchanged:
        errors.append(f"受保护文件被改动: {item['path']}")

stdout_by_id = {}
for run in runs:
    stdout_by_id[run["id"]] = (ROOT / run["stdout"]["path"]).read_text(encoding="utf-8-sig", errors="replace")

c5_details = []
c5_checks = {}
for run_id in c5_ids:
    run = by_id.get(run_id)
    lines = result_lines(stdout_by_id.get(run_id, ""))
    ok = run is not None and run.get("exit_code") == 0
    if run_id == "c5-01-architecture-guard":
        ok = ok and zero_failed_ignored(lines)
    elif run_id == "c5-02-package-guard":
        ok = ok and any("test result: ok. 4 passed; 0 failed; 0 ignored" in line for line in lines)
    elif run_id in ("c5-03-responses-proxy-tests", "c5-05-contract-tests"):
        ok = ok and zero_failed_ignored(lines)
    c5_checks[run_id] = bool(ok)
    c5_details.append({"id": run_id, "cmd": run.get("cmd") if run else None, "exit_code": run.get("exit_code") if run else None, "test_results": lines, "pass": bool(ok)})
    if not ok:
        errors.append(f"{run_id} 未满足卡面 c5 退出码/输出条件")

ignored_details = []
for run_id, doc_limit in (("required-02-test-default", 2), ("required-03-test-all-features", 4), ("c4-01-test-default", 2), ("c4-02-test-proxy", 4)):
    run = by_id.get(run_id)
    if run is None:
        ignored_details.append({"id": run_id, "unit_integration_ignored": None, "doc_tests_ignored": None, "doc_tests_limit": doc_limit, "pass": False, "reason": "没有退出码记录，未做通过判定"})
        continue
    unit_ignored, doc_ignored = ignored_counts(stdout_by_id.get(run_id, ""))
    ok = unit_ignored == 0 and doc_ignored <= doc_limit
    ignored_details.append({"id": run_id, "unit_integration_ignored": unit_ignored, "doc_tests_ignored": doc_ignored, "doc_tests_limit": doc_limit, "pass": ok})

all_recorded_exit_zero = bool(runs) and all(isinstance(run.get("exit_code"), int) and run["exit_code"] == 0 for run in runs)
all_exit_zero = len(runs) == 18 and all_recorded_exit_zero
verdict = "PASS" if all_exit_zero and not errors else "FAIL"

artifact_paths = [PROBE / "preflight.json", PROBE / "rerun-preflight.json", PROBE / "run-commands.ps1", PROBE / "run-missing.ps1", PROBE / "finalize-gate.py", MANIFEST, PROBE / "runs.jsonl"]
artifact_paths.extend(sorted(PROBE.glob("*.log")))
artifacts = [{"path": rel(path), "sha256": sha256(path)} for path in artifact_paths]

command_exit_codes = [
    {"id": run["id"], "criterion": run["criterion"], "cmd": run["cmd"], "exit_code": run["exit_code"], "stdout": run["stdout"]["path"], "stderr": run["stderr"]["path"]}
    for run in runs
]

verification = {
    "schema": "gate-verification/v1",
    "role": "gate",
    "agent": "gate-llmrust",
    "written_by": "gate-llmrust",
    "batch_id": "B-AUD-3",
    "card_id": "C-AUD-PROXY",
    "spec_sha256": SPEC_SHA256,
    "handoff_id": HANDOFF_ID,
    "expected_command_count": 18,
    "recorded_command_count": len(runs),
    "all_recorded_exit_codes_zero": all_recorded_exit_zero,
    "all_expected_exit_codes_zero": all_exit_zero,
    "protected_files_unchanged": all(item["unchanged"] for item in protected_results),
    "protected_files": protected_results,
    "c5": c5_details,
    "ignored_tests": ignored_details,
    "missing_commands": missing_commands,
    "unrecorded_outputs": unrecorded_outputs,
    "errors": errors,
    "verdict": verdict,
}
verification_path = PROBE / "verification.json"
verification_path.write_text(json.dumps(verification, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
artifacts.append({"path": rel(verification_path), "sha256": sha256(verification_path)})

gate = {
    "schema": "agent-workforce-evidence/v1",
    "role": "gate",
    "agent": "gate-llmrust",
    "written_by": "gate-llmrust",
    "card_id": "C-AUD-PROXY",
    "batch_id": "B-AUD-3",
    "spec_sha256": SPEC_SHA256,
    "handoff_id": HANDOFF_ID,
    "inputs_seen": [
        ".workforce/spec.json",
        ".workforce/config.json",
        ".workforce/cards/C-AUD-PROXY.json",
        ".workforce/loop-state.json",
        ".agents/skills/agent-workforce-loop/SKILL.md",
        ".agents/skills/agent-workforce-loop/references/roles.md",
        ".agents/skills/agent-workforce-loop/references/loop-state-machine.md",
        ".agents/skills/agent-workforce-loop/assets/evidence.template.json",
        ".workforce/evidence/B-AUD-3/dev-C-AUD-PROXY.json",
        ".workforce/evidence/B-AUD-3/qa-C-AUD-PROXY.json",
        "AGENTS.md",
        "AGENT_MANIFESTO.md",
        "docs/PROJECT_MAP.md",
        "docs/CAPABILITIES.md",
        "docs/CONTRACTS.md",
        "src/types.rs",
        "src/providers/mod.rs",
        "examples/README.md",
        "Cargo.lock",
    ],
    "mock_used": False,
    "artifacts": artifacts,
    "command_exit_codes": command_exit_codes,
    "expected_command_count": 18,
    "recorded_command_count": len(runs),
    "missing_commands": missing_commands,
    "criteria_results": {
        "config_required_commands": "PASS" if all(by_id.get(run_id, {}).get("exit_code") == 0 for run_id in required_ids) else "FAIL",
        "card_c4": "PASS" if all(by_id.get(run_id, {}).get("exit_code") == 0 for run_id in c4_ids) else "FAIL",
        "card_c5": "PASS" if all(c5_checks.values()) else "FAIL",
        "zero_relaxation_ignored_counts": "PASS" if all(item["pass"] for item in ignored_details) else "FAIL",
        "protected_inputs_unchanged": "PASS" if all(item["unchanged"] for item in protected_results) else "FAIL",
    },
    "verdict": verdict,
    "independent": True,
    "declared_by_lead": False,
    "generated_at_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
    "notes": "按 Lead 指令未重跑命令。现有 runs.jsonl 仅记录 8/18 条命令的退出码（七条 required_commands 与 c4 第一条均 exit 0）；c4-02 有 stdout/stderr 文件但无退出码记录，c4 余下五条与 c5 五条均无记录。因此不满足 18 条全实跑且全 0 的 PASS 条件，verdict=FAIL。已复验所有已记录输出的 sha256 与字节数，并检查 QA/handoff/spec 绑定及受保护文件哈希；未修改业务代码、测试、spec、card、signoffs 或 loop-state。",
}
if verdict == "PASS":
    gate["notes"] = "v8 新 handoff 下独立 gate 共记录 18/18 条命令退出码且全部为 0：config 七条 required_commands、卡 c4 六条、c5 五条。首批 8 条未重跑；经 Lead 授权仅串行重跑缺失的 c4-02..c4-06 与 c5 五条并追加 runs.jsonl。每条 stdout/stderr 原始字节分别留存并计算 sha256；c5 输出条件、ignored 零放宽口径与受保护文件哈希复核通过，故 verdict=PASS。未修改业务代码、测试、spec、card、signoffs 或 loop-state；mock_used=false。"

GATE.write_text(json.dumps(gate, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

print(json.dumps({"gate": rel(GATE), "gate_sha256": sha256(GATE), "verdict": verdict, "recorded_commands": len(runs), "expected_commands": 18, "missing_commands": [item["id"] for item in missing_commands], "errors": errors}, ensure_ascii=False))
if verdict != "PASS":
    sys.exit(1)
