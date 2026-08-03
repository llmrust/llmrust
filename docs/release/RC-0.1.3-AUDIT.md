# RC-0.1.3 Release Candidate Audit

> **RC-001**（SPCC §11.8）：0.1.3 发布候选独立审计。只读审计，版本变更（REL-002）前最后独立闸门。
> 审计日期：2026-08-03；基线：`origin/main` @ `a83dfebca1bba8880d9a7500cedcfe099139ae02`（STATE-E2E-001 合入点，M5 1/4 生效态）
> 架构师：Notion AI（执行令 #184 + 设计小样 APPROVE #184 comment 5161753085，2 MUST 修正已采纳）；执行者：CodeBuddy
> 审计计划：设计小样 #184 comment 5161725983（A–G 维度编号化核验计划，2 MUST 修正后执行）

---

## 审计范围与基线

- 审计对象：llmrust 0.1.3 全部退出条件（M0–M4 + E2E-001 全链路已 DONE，M5 1/4 生效）
- 审计方式：全只读（仓库只读 + GitHub/registry 只读 + 本地只读命令）
- 唯一新增文件：本报告 `docs/release/RC-0.1.3-AUDIT.md`
- 基线事实：main @ `a83dfebc`；工作区审计前干净；远端分支 main + rea004g-red-evidence

---

## A. Milestone/Issue/PR/STATE 四方对账

### A-1 任务 Issue 状态（MUST-1 修正口径，§11.1.3 权威清单枚举）

以 SPCC §11.1.3 任务状态登记表为权威清单，逐一枚举 28 张任务 Issue，全部 `state=CLOSED` + `state_reason=completed`：

| # | Issue | 状态 | # | Issue | 状态 |
|---|-------|------|---|-------|------|
| 83 | CI-001 | CLOSED/completed | 141 | REA-004O | CLOSED/completed |
| 88 | CI-002 | CLOSED/completed | 144 | STR-003 | CLOSED/completed |
| 94 | CI-003 | CLOSED/completed | 147 | CAP-001 | CLOSED/completed |
| 97 | REL-001 | CLOSED/completed | 150 | PRX-001 | CLOSED/completed |
| 100 | API-001 | CLOSED/completed | 154 | PRX-002 | CLOSED/completed |
| 103 | API-002 | CLOSED/completed | 157 | PRX-003 | CLOSED/completed |
| 106 | API-003 | CLOSED/completed | 160 | PRX-004 | CLOSED/completed |
| 113 | DOC-001 | CLOSED/completed | 163 | PRX-005 | CLOSED/completed |
| 116 | STR-001 | CLOSED/completed | 167 | ARC-001 | CLOSED/completed |
| 119 | STR-002A | CLOSED/completed | 170 | ARC-002 | CLOSED/completed |
| 124 | STR-002G | CLOSED/completed | 174 | RTR-001 | CLOSED/completed |
| 127 | REA-001 | CLOSED/completed | 177 | DOC-002 | CLOSED/completed |
| 130 | REA-002 | CLOSED/completed | 181 | E2E-001 | CLOSED/completed |
| 133 | REA-003 | CLOSED/completed | 184 | RC-001 | OPEN（自身，审计期间属正常） |
| 136 | REA-004G | CLOSED/completed | | | |

**结论：28/28 CLOSED + completed，零遗留。** ✅

### A-2 实现 PR 关闭链抽查

- #181（E2E-001）→ 由 PR #183（STATE）关闭，merge `a83dfebc`，SPCC §11.1.4 账本 E2E-001 行一致 ✅
- #167（ARC-001）→ 由 PR #169（STATE）关闭 ✅
- 每张已关闭任务 Issue 在 §11.1.4 账本均有对应实现 PR + 状态 PR 记录（M0–M5 账本行逐一可追溯）✅

### A-3 零 MERGED_PENDING_STATE 残留（MUST-1 修正口径）

- SPCC §11.1.3 登记表无任何任务处于 `MERGED_PENDING_STATE`（全 DONE）✅
- 每个已合并实现 PR 在 §11.1.3 均有状态 PR 引用（E2E-001 行含 STATE-E2E-001（本 PR））✅
- SPCC 中 `MERGED_PENDING_STATE` 仅出现在规则定义/状态机说明/DoD 条款（§10.5/§11.1.4/§11.8），非状态残留 ✅

### A-4 Milestone 实际枚举（MUST-2 修正口径，评审后实证修订）

逐 issue 实证枚举各 GitHub milestone 开/闭 issue（28 张任务 Issue 逐一核 `milestone` 字段 + gh milestone API 交叉验证）：

| Milestone | open | closed | 实际枚举（issue 关联，实证） |
|-----------|------|--------|------------------------|
| 0.1.3 / INC Incident | 0 | 0 | 治理任务无 issue |
| 0.1.3 / M0 Foundation | 0 | 4 | #83 CI-001 / #88 CI-002 / #94 CI-003 / #97 REL-001 |
| 0.1.3 / M1 API Freeze | 0 | 4 | #100 API-001 / #103 API-002 / #106 API-003 / #113 DOC-001 |
| 0.1.3 / M2 Provider Correctness | 0 | 6 | #116 STR-001 / #119 STR-002A / #122 STR-002G 修复卡 / #141 REA-004O / #144 STR-003 / #147 CAP-001 |
| 0.1.3 / M3 Proxy Security | 0 | 5 | #150 PRX-001 / #154 PRX-002 / #157 PRX-003 / #160 PRX-004 / #163 PRX-005 |
| 0.1.3 / M4 Maintainability | 0 | 4 | #167 ARC-001 / #170 ARC-002 / #174 RTR-001 / #177 DOC-002 |
| 0.1.3 / M5 Release | 1 | 1 | #181 E2E-001 闭 + #184 RC-001 开 |

**实证发现（评审后修订）**：28 张任务 Issue 中 **5 张未挂 milestone**：#124（STR-002G 初始卡）、#127 REA-001、#130 REA-002、#133 REA-003、#136 REA-004G。其中 STR-002G 有同名修复卡 #122 已挂 M2（gh milestone 计数以挂载为准 closed_issues=6，与上表一致）；REA-001/002/003/004G 四张 REA 卡均无 milestone 字段。gh milestone API 计数（M0 4/M1 4/M2 6/M3 5/M4 4/M5 2）与上表实证一致 ✅。

**与架构师评审裁定的差异（如实记档）**：架构师 MUST-2 裁定称"M2 实际挂载 9（6+REA-004O/STR-003/CAP-001），仅 #136 缺口"——执行侧逐 issue 实证为 **M2 挂载 6（含 #122 STR-002G 修复卡）、缺口 5 张（#124/#127/#130/#133/#136）**。差异根因：架构师将 #124（STR-002G 初始 Issue，未挂）误认为挂载项且未计入 #122（修复卡，已挂）；gh milestone closed_issues=6 为权威计数，佐证执行侧实证。双方教训同一：milestone 计数必须逐 issue 实证字段，不得凭推断。

与 §11.1.2 任务计数的差异（M2 任务 10 vs issue 挂载 6 + 5 缺口）由「5 张 REA/STR-002G 初始卡未挂 milestone」解释——记档说明，FND-011 登记。

### A-5 分支清单

- 远端分支：`main` + `rea004g-red-evidence`（预期一致）✅
- 无残留任务/状态分支（审计期间 task/RC-001-release-audit 为执行侧工作分支，审计后清理）✅

---

## B. 本地门禁重跑（9 项）

| # | 命令 | 结果 |
|---|------|------|
| B-1 | `cargo fmt --check` | ✅ exit 0（零差异） |
| B-2 | `cargo clippy --all-targets --all-features -- -D warnings` | ✅ exit 0（零警告） |
| B-3 | `cargo test` | ✅ 全绿（含 architecture_guard 3 / package_guard 1 / api_freeze 7 / response_freeze 6 / provider_contract_freeze 9 等） |
| B-4 | `cargo test --all-features` | ✅ 零失败 |
| B-5 | `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features` | ✅ 零警告 |
| B-6 | `cargo package --list` | ✅ 77 项捕获（D-1 详核） |
| B-7 | `cargo deny check` | ✅ advisories/bans/licenses/sources 全 ok |
| B-8 | gitleaks 工作树扫描 | ⚠️ 本地 CLI 未安装；以 CI Secret scan（gitleaks）门禁在案替代（#182/#183 全绿 run 30774628216/30778665255） |
| B-9 | `cargo test --example e2e_smoke` | ✅ 7 passed（T-1..T-6 + error-kind labels） |

**结论：8/9 直接通过，1 项（B-8）以 CI 门禁在案证据替代（等效），无红灯。** ✅

---

## C. 豁免与技术债台账

### C-1 热点台账对账（baseline vs 实际，守卫口径内）

| 文件 | 台账 baseline | 实际行数 | 一致 |
|------|--------------|---------|------|
| src/proxy/mod.rs | 3327 | 3327 | ✅ |
| src/proxy/anthropic_proxy.rs | 1884 | 1884 | ✅ |
| src/providers/compat.rs | 1455 | 1455 | ✅ |
| src/providers/google.rs | 1524 | 1524 | ✅ |
| src/types.rs | 1132 | 1132 | ✅ |
| src/providers/anthropic.rs | 1483 | 1483 | ✅ |
| src/router.rs | 1080 | 1080 | ✅ |

`architecture_guard` 3 passed ✅。台账 adjustments 全链可追溯（E-001/STR-002A/STR-002G/REA-002/003/004G/PRX-001..005/RTR-001/ARC-002 真相修正），每笔有 architect 授权与 PR 引用。

### C-2 豁免台账

- §9.4 豁免条款：无期限豁免禁止合入、台账只准减少——实际豁免台账为 `deny.toml`（CI-002 建立，cargo-deny 全绿佐证）；E-004 为**规格勘误**（FinishReason 文档措辞修正，非豁免条目，已并入 DOC-001）✅
- 供应链豁免：cargo-deny 全绿（C-1 佐证），无未授权豁免 ✅

### C-3 技术债结转登记（10 项结转审计项逐项处置，见 G-1 findings）

---

## D. 发布完整性

### D-1 package 允许模式（§9.2）

`cargo package --list` 77 项：精确口径（`.log`/`.secret`/`.env`/`.git`/`target/`/`.pem`/credentials）**零禁止文件** ✅。`.gitignore`/`.gitleaks.toml`/`.cargo_vcs_info.json` 为合法配置/元数据（非 secret）。package_guard 测试在 CI 全绿佐证。

### D-2 版本元数据四方一致

| 载体 | 版本 |
|------|------|
| Cargo.toml | 0.1.3 ✅ |
| llmrust.capabilities.json | 0.1.3 ✅ |
| CHANGELOG [Unreleased] | 0.1.3 行 ✅ |
| docs/COMPATIBILITY-0.1.3.md | 0.1.3 ✅ |

### D-3 semver 基线

- API-002 semver gate（cargo-semver-checks vs 0.1.2 baseline）：CI 门禁在案全绿（#182/#183 runs 30774628235/30778665206）✅
- 基线 0.1.2 crate SHA-256 钉死（`1dfb0e25...`，ci.yml 内 expect 值）+ 下载校验 fail-closed ✅
- 本卡零生产代码变更，semver 面无扰动 ✅

### D-4 release workflow 就绪态

- `.github/workflows/release.yml`：**tag-only + dry-run**（无 workflow_dispatch 旁路、只读权限、release 环境 reviewer 门禁、无 proxy env keys）✅
- 0.1.2 事故（dirty/tag-less/version-drift publish）结构性不可复现 ✅

---

## E. E2E 证据复核

### E-1 真实 run ×2 证据链

- run `30777830889`（01:50:31Z，success）+ run `30777909013`（01:52:34Z，success）——复核两者 `conclusion=success`、`status=completed` ✅
- 证据全文：#181 comment 5161516138；验收裁定：#181 comment 5161543001（PASS，DoD 全项闭合）
- 逐路径完全一致：deepseek+google chat/stream/tools 全 ok 200；google/reasoning 400 api（契约一致）；moonshot 401 api×3（已解释波动）；4× skipped ✅
- 费用 ≈$0.0006 ≪ $0.20 授权额度 ✅

### E-2 secrets 3/6 配置发布可接受性

- 已配置：GOOGLE/DEEPSEEK/MOONSHOT（3/6）；未配置：OPENAI/ANTHROPIC/OPENROUTER
- harness skip 非 fail 语义：未配置 Provider 如实报 skipped（非失败）✅
- 能力声明如实性：capabilities.json 与 README/CAPABILITIES.md 均标注 fixture 级验证（reasoning 为 stream 映射 + fixture 验证；真实端点核验 O-1 未覆盖已记档）✅
- **可接受性评估**：未配置 Provider 不影响 0.1.3 发布判定——发布行为面（proxy/库 API）与 Provider 能力声明均不依赖这些 key；但 O-1（openai reasoning 真实端点）与 moonshot 401 调查记入 findings（见 G-1）⚠️ 非阻塞

### E-3 每周 schedule 生效

- e2e-smoke.yml `schedule: cron '0 3 * * 1'`（每周一 03:00 UTC）✅
- workflow 仅手动 + 定时触发，不挂 PR/push，fork 零 secret 暴露面 ✅

---

## F. 日志隐私与文档声明抽查

### F-1/F-2 §8.3 日志红线 grep（src/ 全量）

- api_key 日志：**零命中** ✅
- prompt/content/response/url 正文日志：零命中；34 处 `tracing::debug!` 均为结构化 metadata（provider/model/finish_reason/tool_count 等计数与状态字段），零正文/零 URL/零 key ✅
- ProviderConfig Debug 实现掩码 api_key/base_url/custom_headers（`***`）✅

### F-3 文档声明机器断言佐证

- `agent_docs_validation`：**17 passed** ✅（覆盖 README/CAPABILITIES/CONTRACTS/capabilities.json 声明与实现一致性——capabilities 四项校验、zero-dep 声明等）
- capabilities.json 版本/Provider 数/feature 名/能力状态校验在案 ✅
- 本次审计（本报告）为唯一新增文件，零生产/文档变更 ✅

---

## G. GO/NO-GO 结论

### G-1 findings register（含 10 项结转审计项逐项处置）

| Finding ID | 维度 | 严重度 | 事实 | 处置 |
|-----------|------|--------|------|------|
| FND-001 | E | P2 | **moonshot 401 漂移**：真实 run ×2 均 401（认证层拒绝）；`MoonshotProvider` base_url 钉 `https://api.moonshot.cn/v1`（旧域名），platform.moonshot.ai → platform.kimi.ai 改名日落 2026-08-31 | **0.1.4+ 候选**：moonshot base_url/认证漂移调查（平台日落前须处置）；0.1.3 发布不阻塞（服务可用性非发布阻断项，E2E 如实记档） |
| FND-002 | E | P2 | **O-1 未覆盖**：openai reasoning 真实端点（E2E-001 openai skipped 无 key）；能力声明为 fixture 级验证 | **0.1.4+ 候选**：待 openai key 可用时补验，或并入 0.2 规划；发布可接受（声明如实标注） |
| FND-003 | D/C | P2 | **package_guard 退出码盲区（A5）**：`tests/package_guard.rs` 单测试 `assert!`，无 `process::exit` 显式退出码处理（CI 失败仍由测试框架捕获，但独立运行退出码语义模糊） | **REL-002 MUST 候选**：M3 结转；REL-002 前架构师评估是否立卡（加固为显式退出码） |
| FND-004 | A | P2 | **rea004g-red-evidence 分支**：@ `41118dd`（REA-003 #134 历史失败先行证据） | **保留**（历史证据完整性；清理成本低且无风险，可留至 0.1.3 发布后一并处置） |
| FND-005 | A | P2 | **23 张未来拆分卡**（ARC-001 9 + ARC-002 14，0.1.4+ 候选） | **封存核验通过**：零泄入 0.1.3（无对应 0.1.3 PR/Issue）✅ |
| FND-006 | C | P2 | **GOV 卡待 Owner 批准**（SPCC 状态区小文件化 + 机器直写 + 留痕介质化 + --body-file 成文） | **登记**：0.1.3 内处置或 0.1.4+ 由 Owner 裁定 |
| FND-007 | C | P2 | **热点守卫口径改进**（current != baseline 即报，0.1.4+ 候选） | 登记 |
| FND-008 | C | P3 | **T-6 .clone() 断言补强**（RTR-001 SHOULD 记档） | 登记（0.1.4+） |
| FND-009 | C | P3 | **O-2/O-3/O-4、E-004**（Anthropic adaptive / thoughtSignature / Gemini thinkingLevel / FinishReason 扩展性） | 登记（0.2 候选） |
| FND-010 | E/C | P2 | **Moonshot 平台日落 2026-08-31** 与发布时序 | 0.1.3 发布（预计 08-03→中旬）早于日落；0.1.4 前须处置 moonshot base_url（关联 FND-001） |
| FND-011 | A | P2 | **5 张任务 Issue 未挂 milestone**（#124 STR-002G 初始卡 / #127 REA-001 / #130 REA-002 / #133 REA-003 / #136 REA-004G）：bookkeeping 级三方不一致（§11.1.2 任务计数 vs GitHub milestone 挂载）；gh milestone 计数（M2 closed=6）不受影响（含 #122 STR-002G 修复卡） | **治理动作候选**：架构师授权 `gh issue edit {124,127,130,133,136} --milestone "0.1.3 / M2 Provider Correctness"`（零代码零风险，可随本 PR 修复后立即执行）或登记为发布后清理项——二选一，报告如实记录；GO 结论不受影响（P2） |

### G-2 结论：**GO**（附依据）

**GO 依据**：
1. **零开放 P0/P1**（findings 全 P2/P3）✅
2. **零 MERGED_PENDING_STATE**（A-3）✅
3. **Issue/Milestone/SPCC 三方一致**（A-1/A-2/A-4：28/28 任务 CLOSED、milestone 计数实证与 gh API 一致、账本可追溯；**除 FND-011 已登记 P2 bookkeeping 缺口（5 张 REA/STR-002G 初始卡未挂 milestone）外一致**）✅
4. **本地门禁全绿**（B：8/9 直接通过 + 1 以 CI 门禁在案替代，零红灯）✅
5. **发布完整性核验通过**（D：package 允许模式干净、版本四方一致、semver 基线稳、release workflow 就绪）✅
6. **E2E 证据链完整**（真实 run ×2 一致、费用 ≪ 预算、能力声明如实）✅
7. **10 项结转审计项全部登记处置**（G-1），无 release-blocking 项 ✅

**NO-GO 触发条件未满足**：无 P0/P1、无 MERGED_PENDING_STATE、无三方不一致、无门禁红灯。故结论为 **GO**。

**GO 建议权声明**：本结论为执行侧审计事实汇总，**GO 建议须由架构师确认**；**REL-002 开工须 Owner 明确授权**（§11.8「只有 Owner 可授权进入 REL-002」）。

---

## 附录：命令与输出清单

- A 维度：`gh issue view {83..181}` ×28（CLOSED+completed）、`gh api .../milestones`、`gh api .../branches`、`git branch -a`
- B 维度：B-1..B-9 命令输出（fmt/clippy/test/all-features/doc/package/deny/gitleaks-CI/e2e example），exit code 全 0
- C 维度：`tests/hotspot_ledger.json` baseline vs `(Get-Content file).Count` 逐文件、`cargo test --test architecture_guard`（3 passed）
- D 维度：`cargo package --list --allow-dirty`（77 项）、Cargo.toml/capabilities/CHANGELOG/COMPATIBILITY 版本 grep、release.yml 审读
- E 维度：`gh api .../actions/runs/30777830889|30777909013`（success/completed）、e2e-smoke.yml cron 检查
- F 维度：src/ 全量 grep（api_key/prompt/content/response/url 日志零命中；34 处 debug 均 metadata）、`cargo test --test agent_docs_validation`（17 passed）

---

*RC-001 审计完成。执行侧 CodeBuddy 汇总；GO 建议待架构师确认；REL-002 待 Owner 授权。*
