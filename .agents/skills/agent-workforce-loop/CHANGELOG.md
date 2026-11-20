# CHANGELOG

## agent-workforce-loop v1.3.0（2026-11-18）

- **用户同意必须有佐证**（用户裁定：没有同意的佐证就不可信）：`local_review` 不再是「执行侧提交即可」，改为必须有用户本人用审批密码签名的同意记录，见下方 v1.2.0 「规则表必须有用户同意的佐证」条（已按 v1.3.0 行为改写）。去掉 `trust.reviewed_by`。
- 新增 `scripts/approve.mjs`（setup / config / spec / whoami），已在真实终端（pty）实测 setup → config → config-check PASS。

## agent-workforce-loop v1.2.0（2026-11-18）

按用户裁定更新：

- **用户不做技术验收**：`acceptance.user_confirm` 默认改为 `false`。
- **规则表必须有用户同意的佐证**：新增 `trust.anchor: "local_review"`（默认）与 `scripts/approve.mjs`。用户本人在终端设审批密码（Ed25519 私钥用密码加密存主目录，公钥 `approver.pub.json` 进仓库），审过规则表后签名同意，生成 `approvals/config-<sha12>.json`。config-check / spec-confirm 只认签名：无记录 `USER_APPROVAL_MISSING`、被改 `USER_APPROVAL_SIGNATURE_INVALID`、非登记钥匙 `USER_APPROVAL_WRONG_APPROVER`；开工后换公钥 `APPROVER_KEY_CHANGED`。approve.mjs 要求真实终端（`APPROVE_NEEDS_TERMINAL`）。`user_confirm:true` 时开工范围同样用 `approve.mjs spec` 签名。`remote` / `signed` / `none` 仍可选。
- **机器生成文件禁止手改**：新增 `config.generated: [{path, command}]`。`accept` 与 `release` 重跑生成命令逐字节比对（LF），不同即 `GENERATED_HAND_EDITED`，比完恢复原文件。
- **skill 自身机器生成**：新增 `scripts/machine.mjs`（状态、动作、配置说明的唯一真源，loop.mjs 直接用）与 `scripts/gen-docs.mjs`，生成 `references/generated/state-machine.md`、`config-fields.md`、`reason-codes.md`、`assets/config.template.json`、`MANIFEST.json`。`validate-skill` 跑 `gen-docs --check`：生成文件被手改、清单过期、手写文档提到不存在的命令或错误码 → FAIL。
- SKILL.md 新增「铁律」一节，汇总用户定下的 6 条规则。
- 自测 103 例（v1.3.0）。


## agent-workforce-loop v1.1.0（2026-11-18）

- **批次并行**：执行阶段改为全局 `EXECUTING` + 每批独立状态（pending → dispatched → evidence_collected → accepted → released → delivered，FAIL → reopened）。互不依赖的批次可同时开工（如前端批 + 后端批），上限 `parallel.max_batches`（默认 2）；并行批次之间写入范围（`PARALLEL_SCOPE_CONFLICT`）、卡（`CARD_TAKEN`）、agent（`AGENT_BUSY`）都不能重叠。批次动作加 `--batch`，多批可做时不指定即 `BATCH_AMBIGUOUS`。新增只读 `ready`。去掉 `next-batch`。
- **一卡一人**：spec 里一张卡只能被一个任务引用（`CARD_ASSIGNED_TWICE`）；卡的开发与测试必须是两个 agent（`CARD_OWNER_IS_QA`）。
- **开发 → 测试交接**：`dispatch` 只唤醒 dev；新增 `handoff --card`，dev 合格交卷后冻结其改动文件的 sha、生成一次性 `handoff_id`，再给出 qa brief。qa 回执须带该 id。交接后 dev 改代码 / 改回执 → 测试作废（`DEV_CHANGED_AFTER_HANDOFF` / `DEV_RECEIPT_CHANGED_AFTER_HANDOFF`）；没交接就测 → `QA_WITHOUT_HANDOFF`；拿旧交接的回执 → `QA_NOT_FOR_CURRENT_HANDOFF`。
- **用户不做技术验收**：`acceptance.user_confirm:false` 时 spec-confirm 不要求用户签署；config 仍须用户提交一次。
- 漂移检查与发布的工作区检查都排除同期其他批次自己的范围。
- 自测 90 例（+ 交接 5 例、并行场景 15 例）。


## agent-workforce-loop v1.0.1（2026-11-18）

修三处外部评审实测出的短板（均有自测反例）：

- **台账可手写通过** → state 层校验 `self_sha256` 自洽（`LEDGER_SELF_SHA_INVALID`）；新增 `ledger.command`，accept 重跑生成命令并比对内容（忽略 generatedAt / self_sha256），不同即 `LEDGER_REGEN_MISMATCH`，跑完恢复原文件。没有生成器须写 `ledger.none_reason`，`status` 永久提示 `LEDGER_UNVERIFIED`。
- **acceptors 首次占位** → 新增 `trust` 信任锚：验收者的提交（acceptors / config / user 签署 / issue）还须进入远端受保护分支（`remote`，默认 origin/main，`git ls-remote` 取 tip）或通过 `git verify-commit`（`signed`）。`none` 须写理由并永久提示 `TRUST_NONE`。
- **漂移基线随回流重置** → 批次新增 `batch_base`，首次派卡时设定；accept FAIL 回流重派不重置，夹在回流间提交的业务改动仍须被回执认领。只有已交付批次被 issue 重开时才取新基线。
- `status` / `config-check` 输出 `warnings`，列出所有降级配置。
- 自测 71 例（+ 无台账命令、未进受保护分支、首次占位、回流夹带提交、手写台账不自洽 / 自洽但与重跑不符、trust=none 提示）。


## agent-workforce-loop v1.0.0（2026-11-17）

由 beeli-workforce-loop v3.1 抽成通用版，可在任何读取 SKILL.md 的 agent harness 中使用（Codex、Claude Code、Gemini CLI、Copilot、Cursor、OpenCode 等）。判据与状态机不变。

- 项目差异全部移入 `<workforce>/config.json`：业务目录、门禁命令、规则编号格式、发布汇总文件、CI 方式、前端目录、架构师必读资料、design 工种开关。
- config 必须由验收者本人 git 提交；`spec-confirm` 锁定其 sha，之后改动即 `CONFIG_DRIFT`。
- 新命令：`scaffold`（生成 config / acceptors 模板）、`config-check`。
- workforce 目录可配置：`--dir` / `WORKFORCE_DIR`，默认 `.workforce/`；旧项目用 `--dir orchestration/workforce` 即可沿用。
- 门禁不再写死 `verify-ui-slice`，改为 `gates.required_commands` 逐条核对。
- 发布汇总不再写死 `docs/ui-ledger.json`，改为 `release.overall_file` + JSON Pointer（可关闭）。
- CI 支持 `github`（gh）、`command`（任意命令，`WORKFORCE_HEAD` 传当前提交）、`none`（须写理由，release 带警告）。
- 非 UI 项目可关闭 design 工种：不要求其签署与回执，view 层记为 skipped。
- 五层名用英文 id（event / logic / interface / state / view），兼容中文写法。
- `dispatch` 为每个工种输出纯文本 `brief`，可交给任何 harness 的独立会话；新增 `references/harness-adapters.md`（安装位置、独立会话做法、工具限制、git 身份）。
- frontmatter 去掉 harness 专有的 `allowed-tools`，增加 `compatibility`；validate-skill 检查目录名与 name 一致。
- 去掉所有项目专有名词（BeeLi、DRD、tech-stack 路径）；原 BeeLi 配置作为示例写在 `references/config.md`。
- 自测 63 例（原 45 例 + config 缺失 / 无门禁命令 / agent 提交 config / config 漂移，以及非 UI 项目全流程：自定义目录、design 关闭、命令式 CI、英文层名、gate 漏跑必需命令）。
