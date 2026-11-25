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
SPEC_SHA256 = "0a691cf6a52b048239d6225c76ad1c34bb3729a931ebeced2bf16906d39fcf80"


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
manifest = load_json(PROBE / "run-manifest.json")
runs = manifest.get("runs", [])
by_id = {run.get("id"): run for run in runs}

expected_commands = [
    ("required-01-build", "config.gates.required_commands[0]", config["gates"]["required_commands"][0]),
    ("required-02-test-default", "config.gates.required_commands[1]", config["gates"]["required_commands"][1]),
    ("required-03-test-all-features", "config.gates.required_commands[2]", config["gates"]["required_commands"][2]),
    ("required-04-fmt", "config.gates.required_commands[3]", config["gates"]["required_commands"][3]),
    ("required-05-clippy", "config.gates.required_commands[4]", config["gates"]["required_commands"][4]),
    ("required-06-rustdoc", "config.gates.required_commands[5]", config["gates"]["required_commands"][5]),
    ("c4-01-test-default", "C-AUD-PROXY.c4[0]", "cargo test --workspace"),
    ("c4-02-test-proxy", "C-AUD-PROXY.c4[1]", "cargo test --workspace --features proxy"),
    ("c4-03-build-default", "C-AUD-PROXY.c4[2]", "cargo build --workspace"),
    ("c4-04-fmt", "C-AUD-PROXY.c4[3]", "cargo fmt --all --check"),
    ("c4-05-clippy-default", "C-AUD-PROXY.c4[4]", "cargo clippy --workspace --all-targets -- -D warnings"),
    ("c5-01-architecture-guard", "C-AUD-PROXY.c5[0]", "cargo test --features proxy --test architecture_guard"),
    ("c5-02-package-guard", "C-AUD-PROXY.c5[1]", "cargo test --features proxy --test package_guard"),
    ("c5-03-responses-proxy-tests", "C-AUD-PROXY.c5[2]", "cargo test --features proxy --test responses_proxy_tests"),
    ("c5-04-proxy-lib", "C-AUD-PROXY.c5[3]", "cargo test --features proxy --lib"),
    ("c5-05-contract-tests", "C-AUD-PROXY.c5[4]", "cargo test --features proxy --test contract_tests"),
]

errors: list[str] = []
if len(config["gates"]["required_commands"]) != 6:
    errors.append("config.gates.required_commands 不是六条")
if len(runs) != 16:
    errors.append(f"实跑命令数为 {len(runs)}，预期 16")
if len(by_id) != len(runs):
    errors.append("run id 重复")

for index, (run_id, criterion, command) in enumerate(expected_commands):
    run = by_id.get(run_id)
    if run is None:
        errors.append(f"缺少命令 {run_id}")
        continue
    if index < len(runs) and runs[index].get("id") != run_id:
        errors.append(f"命令顺序不符：第 {index + 1} 条为 {runs[index].get('id')}，预期 {run_id}")
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
        actual = sha256(path)
        if actual != info.get("sha256"):
            errors.append(f"{run_id} {stream} sha256 不符")
        if path.stat().st_size != info.get("bytes"):
            errors.append(f"{run_id} {stream} 字节数不符")

spec_actual = sha256(ROOT / ".workforce" / "spec.json")
if spec_actual != SPEC_SHA256 or manifest.get("spec_sha256") != SPEC_SHA256 or preflight.get("spec_sha256") != SPEC_SHA256:
    errors.append("spec_sha256 不符")

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
for run_id, _, _ in expected_commands:
    run = by_id.get(run_id)
    if run:
        stdout_by_id[run_id] = (ROOT / run["stdout"]["path"]).read_text(encoding="utf-8-sig", errors="replace")

c5_details = []
c5_checks = {}
for run_id in ("c5-01-architecture-guard", "c5-02-package-guard", "c5-03-responses-proxy-tests", "c5-04-proxy-lib", "c5-05-contract-tests"):
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
        errors.append(f"{run_id} 未满足卡面 c5 输出条件")

ignored_details = []
for run_id, doc_limit in (("required-02-test-default", 2), ("required-03-test-all-features", 4), ("c4-01-test-default", 2), ("c4-02-test-proxy", 4)):
    unit_ignored, doc_ignored = ignored_counts(stdout_by_id.get(run_id, ""))
    ok = unit_ignored == 0 and doc_ignored <= doc_limit
    ignored_details.append({"id": run_id, "unit_integration_ignored": unit_ignored, "doc_tests_ignored": doc_ignored, "doc_tests_limit": doc_limit, "pass": ok})
    if not ok:
        errors.append(f"{run_id} ignored 数越过零放宽口径")

all_exit_zero = len(runs) == 16 and all(isinstance(run.get("exit_code"), int) and run["exit_code"] == 0 for run in runs)
verdict = "PASS" if all_exit_zero and not errors else "FAIL"

artifact_paths = [
    PROBE / "preflight.json",
    PROBE / "run-commands.ps1",
    PROBE / "finalize-gate.py",
    PROBE / "run-manifest.json",
    PROBE / "runs.jsonl",
]
for run in runs:
    artifact_paths.extend([ROOT / run["stdout"]["path"], ROOT / run["stderr"]["path"]])
artifacts = [{"path": rel(path), "sha256": sha256(path)} for path in artifact_paths]

command_exit_codes = []
for run in runs:
    command_exit_codes.append({
        "id": run["id"],
        "criterion": run["criterion"],
        "cmd": run["cmd"],
        "exit_code": run["exit_code"],
        "stdout": run["stdout"]["path"],
        "stderr": run["stderr"]["path"],
    })

verification = {
    "schema": "gate-verification/v1",
    "role": "gate",
    "agent": "gate-llmrust",
    "batch_id": "B-AUD-3",
    "card_id": "C-AUD-PROXY",
    "spec_sha256": SPEC_SHA256,
    "command_count": len(runs),
    "all_exit_codes_zero": all_exit_zero,
    "protected_files_unchanged": all(item["unchanged"] for item in protected_results),
    "protected_files": protected_results,
    "c5": c5_details,
    "ignored_tests": ignored_details,
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
    "inputs_seen": [
        ".workforce/spec.json",
        ".workforce/config.json",
        ".workforce/cards/C-AUD-PROXY.json",
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
    ],
    "mock_used": False,
    "artifacts": artifacts,
    "command_exit_codes": command_exit_codes,
    "criteria_results": {
        "config_required_commands": "PASS" if all(by_id.get(run_id, {}).get("exit_code") == 0 for run_id, _, _ in expected_commands[:6]) else "FAIL",
        "card_c4": "PASS" if all(by_id.get(run_id, {}).get("exit_code") == 0 for run_id, _, _ in expected_commands[6:11]) else "FAIL",
        "card_c5": "PASS" if all(c5_checks.values()) else "FAIL",
        "zero_relaxation_ignored_counts": "PASS" if all(item["pass"] for item in ignored_details) else "FAIL",
        "protected_inputs_unchanged": "PASS" if all(item["unchanged"] for item in protected_results) else "FAIL",
    },
    "verdict": verdict,
    "independent": True,
    "declared_by_lead": False,
    "generated_at_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
    "notes": "独立 gate 实跑 config.gates.required_commands 六条与 C-AUD-PROXY c4/c5 共十条；required 命令保持卡面/config 的 --locked 与 feature 口径，c4/c5 严格按卡面原文。每条 stdout/stderr 原始字节分别留存并计算 sha256；全部 16 条 exit code 为 0，且 c5 输出条件、ignored 零放宽口径与受保护文件哈希复核通过，故 verdict=PASS。未修改业务代码、测试、spec、card、signoffs 或 loop-state；mock_used=false 表示未伪造或替代实跑结果。",
}
if verdict == "FAIL":
    gate["notes"] = "独立 gate 已逐条实跑 16 条命令并留存原始输出；存在非零退出码、卡面输出条件不符、ignored 越界或受保护文件变化，故 verdict=FAIL。详见 verification.json 的 errors。"

GATE.write_text(json.dumps(gate, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

print(json.dumps({"gate": rel(GATE), "gate_sha256": sha256(GATE), "verdict": verdict, "commands": len(runs), "nonzero": [run["id"] for run in runs if run["exit_code"] != 0], "errors": errors}, ensure_ascii=False))
if verdict != "PASS":
    sys.exit(1)
