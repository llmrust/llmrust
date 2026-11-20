# PR #251 CI 诊断报告（DELTA-NULL）

- **仓库**：`llmrust/llmrust`
- **PR**：#251 — `fix(providers): 接受 OpenAI 兼容流末帧的 delta:null / 字段缺失（真调工具必失败）`
- **head**：`fix/delta-null-stream-chunk` @ `dc1e754af2b24ed28849a332585301e3909d621b`
- **base**：`main`
- **诊断时间**：2026-11-20 16:56–17:0x（Asia/Shanghai）
- **诊断者**：`ci-llmrust`（ci 工种，**只读**：未合并、未改代码/配置/workflow、未 git 提交）

---

## 一、逐 job 结论表

`gh pr checks 251 --repo llmrust/llmrust`（末次，exit 0）

| # | Job | 结论 | 耗时 | Workflow (run id) | Job id |
|---|---|---|---|---|---|
| 1 | API-002 semver gate (vs 0.1.2 baseline) | **pass** | 4m33s | CI (44706062447) | 136185848676 |
| 2 | Architecture & package guards (CI-003) | **pass** | 49s | CI (44706062447) | 136185848894 |
| 3 | Test (stable) | **pass** | 1m16s | CI (44706062447) | 136185849034 |
| 4 | MSRV (1.86) | **pass** | 44s | CI (44706062447) | 136185849102 |
| 5 | Secret scan (gitleaks CLI) | **pass** | 6s | Security Gates (44706062413) | 136185848475 |
| 6 | Supply-chain policy (cargo-deny) | **pass** | 51s | Security Gates (44706062413) | 136185848609 |
| 7 | RustSec advisories | **pass** | 28s | Security Gates (44706062413) | 136185849025 |

**统计：7 pass / 0 fail / 0 pending。**（首轮采集时 API-002 为 `pending`，已轮询至结论：`16:57:30 in_progress → 16:57:51 completed / success`，未超 15 分钟预算，无失败需拉日志。）

交叉核验：`gh api repos/llmrust/llmrust/commits/dc1e754.../check-runs` → `total_count=7`，7 条全部 `completed / success`，无遗漏的 check-run。

---

## 二、run 级门（第 4 项要求）

`gh api "repos/llmrust/llmrust/actions/runs?head_sha=dc1e754af2b24ed28849a332585301e3909d621b"` → `total_count=2`

| Workflow | run id | event | status | conclusion | head_sha |
|---|---|---|---|---|---|
| CI | 44706062447 | **pull_request** | completed | **success** | dc1e754a…621b |
| Security Gates | 44706062413 | **pull_request** | completed | **success** | dc1e754a…621b |

**判定：`event=pull_request` 且 `conclusion=success` → 满足。** 无 push 事件 run、无其它 sha 的 run 混入。

> 说明：`e2e-smoke.yml` 只挂 `workflow_dispatch` + cron（`0 3 * * 1`），**不挂 pull_request**，因此本 PR 不产生该 run，属设计如此、不是缺失。`release.yml` 仅 tag 触发，同样不适用。
>
> 说明：`gh api repos/.../commits/<sha>/status`（旧 status API）返回 `state=pending, statuses=[], total_count=0` —— 这是**空集**，不是失败：本仓只用 check-runs，没有 legacy commit status。不影响门。

---

## 三、API-002 semver gate 专项（重点关注项）

**结论：绿。原文如下（job 136185848676 日志）：**

```
2026-11-20T08:56:50.1755439Z expected=1dfb0e25b02af20ad562fdf4c8c4492b71bdb37d9a66b6dc181a951b4879c481
2026-11-20T08:56:50.1756470Z actual  =1dfb0e25b02af20ad562fdf4c8c4492b71bdb37d9a66b6dc181a951b4879c481
2026-11-20T08:56:50.1844352Z ##[group]Run cargo semver-checks check-release --baseline-root /tmp/llmrust-0.1.2/llmrust-0.1.2
2026-11-20T08:57:08.5334832Z     Building llmrust v0.1.2 (baseline)
2026-11-20T08:57:25.0197540Z        Built [  16.486s] (baseline)
2026-11-20T08:57:25.0197980Z      Parsing llmrust v0.1.2 (baseline)
2026-11-20T08:57:25.0344589Z       Parsed [   0.015s] (baseline)
2026-11-20T08:57:25.0517406Z     Checking llmrust v0.1.2 -> v0.1.5 (minor change)
2026-11-20T08:57:25.1017856Z      Checked [   0.050s] 196 checks: 196 pass, 57 skip
2026-11-20T08:57:25.1018532Z      Summary no semver update required
2026-11-20T08:57:25.1021217Z     Finished [  34.172s] llmrust
```

逐 step 明细（全部 success）：`Set up job` / `checkout` / `rust-toolchain` / `rust-cache` / `Install cargo-semver-checks (pinned, 0.48.0)` / `Fetch 0.1.2 baseline source` / `Run semver gate against 0.1.2 baseline` / 两个 Post / `Complete job`。

关键点三条，全部对上：
1. **基线根信任未被绕过**：`EXPECTED_SHA256 == ACTUAL_SHA256 == 1dfb0e25…9c481`，SHA 校验通过（若 mismatch 会走 `::error::Baseline crate SHA-256 mismatch` 分支，本次未触发）。
2. **196 checks: 196 pass, 57 skip** —— 零 fail，`Summary no semver update required`。
3. 与本次改动性质自洽：`CompStreamChoice` / `CompDelta` 均为**私有 struct**（`src/providers/compat.rs` 内非 `pub`），对外 API 面零变更；新增的 3 个 `#[test]` 不进 API 面；`tests/hotspot_ledger.json` 只是台账。→ 理论 semver-clean，实测亦 semver-clean。

`hotspot_ledger` 台账相关门在 CI-003 里另行验证并通过：`hotspot_files_do_not_grow ... ok`、`hotspot_ledger_is_consistent ... ok`、`hotspot_coverage_is_complete ... ok`（15 passed / 0 failed），`cargo test --test package_guard` 4 passed / 0 failed —— 即 `4b73d66` 引入的 package_guard 红已被 `dc1e754` 修掉且复验通过。

---

## 四、其它 job 的实质证据（非只看绿灯）

- **Test (stable)**：本次三个回归测试**真实跑过且 ok**（日志逐条）：
  - `test providers::compat::tests::stream_terminal_chunk_with_null_delta_parses ... ok`
  - `test providers::compat::tests::stream_terminal_chunk_with_absent_delta_parses ... ok`
  - `test providers::compat::tests::stream_delta_content_still_parses_with_option ... ok`
  - 全量：lib `259 passed; 0 failed`（default features）/ `382 passed; 0 failed`（all features），各集成测试 target 全 `0 failed`。
- **MSRV (1.86)**：`rustc 1.86.0 (05f9846f8 2025-03-31)` 安装后构建通过（success）。
- **Secret scan / cargo-deny / RustSec**：三个 Security Gates job 全 success，无 advisory。

---

## 五、合并前状态（只记录，未执行任何合并）

`gh pr view 251 --json mergeable,mergeStateStatus,reviewDecision`：

- `mergeable = MERGEABLE`
- `mergeStateStatus = **CLEAN**`（首轮采集时曾是 `UNSTABLE`，原因是 API-002 仍 pending；其转 success 后已变 CLEAN）
- `reviewDecision = ""`（空）—— `gh api repos/llmrust/llmrust/branches/main/protection` 返回 **404 Not Found**，即 main **无分支保护规则**，因此不存在"必需 review 数"这类硬门。这意味着**技术上现在就能合**，人/流程门只能靠 skill 的 `premerge` 门来兜。

---

## 六、我的判断

**CI 侧：可以合（CI-green，全部 7/7 pass，API-002 semver gate 绿，`mergeStateStatus=CLEAN`）。**

但**我不下合并指令、也不执行合并**，理由与边界：
1. 任务纪律明确：合并要等 skill 的 `premerge` 门 + Owner 指令；本报告只做 CI 事实认定。
2. CI 绿 ≠ 交付验收。本次是 provider 解析层的真实 bugfix，其**真实价值只有在真实上游端点（`delta:null` 末帧）上跑通工具调用才能证明**；CI 里的 3 个测试是 fixture 级回归，不是 E2E 实测。建议 premerge 门另行取证：真实端点（台账里记的 `http://127.0.0.1:18778/v1` + `nous/meituan/longcat-2.5:free` 或 `nous/upstage/solar-mini4:free`）跑一次流式 tool-call，确认不再出 `Invalid response`。
3. `main` 无分支保护（404）是**结构性风险点**：任何本地 `gh pr merge` 都无远端硬门拦截，所以"不许合并"这条纪律此刻完全依赖执行者自律。建议后续单独开 issue 加保护（本任务只读，不改配置）。
4. 提醒：本 PR 夹带 `4b73d66`（引入 agent-workforce-loop skill 体系，+4525 行、43 文件）。CI 已绿，但这属于**与 DELTA-NULL 修复不同性质的大体量改动**；若 Owner 希望 PR 单一职责，需在 premerge 阶段单独裁定，而不是由 CI 结论代答。

**一句话：CI 无阻塞项，API-002 绿，可进入 premerge 门；是否合并等 Owner 指令。**

---

## 七、证据文件清单

目录：`.workforce/evidence/DELTA-NULL/`

| 文件 | 内容 |
|---|---|
| `pr-view.json` | PR 元数据（head sha / base / draft / mergeable） |
| `pr-checks.txt` / `pr-checks-final.txt` | 首轮（含 1 个 pending）与末轮（7 pass）`gh pr checks` 原文 |
| `api002-job.json` / `api002-job-final.json` | API-002 job 前后状态与逐 step |
| `api002-poll.txt` | 轮询轨迹（in_progress → success） |
| `api002.log` | API-002 完整日志（82,866 B），含 SHA 校验与 semver-checks 原文 |
| `test-stable.log` | Test (stable) 完整日志（154,048 B），含 3 个新测试 ok |
| `ci003.log` / `msrv.log` | CI-003 与 MSRV 日志 |
| `run-CI-jobs.json` / `run-Security-jobs.json` | 两次 run 的 job 列表 |
| `run-CI.json` / `run-Security.json` / `runs.json` / `runs-final.json` | run 级 event/conclusion 核验 |
| `check-runs.json` | head sha 上 7 条 check-run 全量 |
| `pr-state-final.json` | mergeable / mergeStateStatus / statusCheckRollup |
| `pr-files.txt` / `pr-stats.json` / `pr-251.diff` | 43 文件、+4525/−9、3 提交、完整 diff |
| `branch-protection.json` | main 分支保护 404（无保护） |
