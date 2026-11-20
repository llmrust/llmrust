# Loop 状态机（v1.2）

所有迁移前先检查 `config.json`：缺失或无效 → `CONFIG_INVALID`；`spec-confirm` 后被改 → `CONFIG_DRIFT`。

状态存 `<W>/loop-state.json`，只能由 `scripts/loop.mjs` 写；文件带 `self_sha256`，被手改后所有命令 `STATE_TAMPERED` 拒绝。

状态分两层：**全局**（spec 审批链 → 执行中 → 全部交付）和**批次**（每个批次自己走 派卡 → 交接 → 回执 → 验收 → 发布 → 交付）。执行阶段多个批次可以同时在做，例如一个相位纵切拆出的前端批和后端批。

## 状态与允许动作

见 `generated/state-machine.md`（由 `scripts/machine.mjs` 生成，和脚本实际判定同源）。

除 `architect-spec` / `spec-revise` 外，所有动作先比对 `spec_sha256`：确认后 spec 被改 → `SPEC_DRIFT` 拒绝。`dispatch` 不带 `--batch` 时取 spec 顺序里第一个能开工的批次；多个批次都能做同一动作时不带 `--batch` 即 `BATCH_AMBIGUOUS`。

`ready` 命令（只读）列出现在能开工的批次，以及其他批次被什么挡住。

## 开发 → 测试交接

测试不能和开发同时进行，否则一个在改、一个在测，结果对不上。

1. `dispatch` 只唤醒交卷方（dev/ci），输出里 `qa_waits_for_handoff` 列出每张卡的测试在等什么。
2. 交卷方（dev/ci）交卷（`<role>-<card>.json`：owner_agent、PASS、files_changed 在 write_scopes 内）后，Lead 跑 `handoff --card <id>`（`qa` 是验证方，qa 卡不可 handoff，会报 `CARD_NOT_WORKER_TASK`）。脚本记录该回执和每个 files_changed 文件的 sha（冻结），生成一次性的 `handoff_id`，再给出 qa 的 brief。
3. qa 回执必须带这个 `handoff_id`。evidence 时：
   - 没有交接 → `QA_WITHOUT_HANDOFF`
   - 交接后 dev 回执被改 → `DEV_RECEIPT_CHANGED_AFTER_HANDOFF`
   - 交接后冻结的文件被改 → `DEV_CHANGED_AFTER_HANDOFF`（测试作废）
   - qa 回执不是针对最近一次交接 → `QA_NOT_FOR_CURRENT_HANDOFF`
4. dev 需要再改：改完重新交卷、重新 `handoff`，测试重做。批次回流重新 `dispatch` 时，所有交接清空。

## evidence 检查

- 必需回执：每张 dev 卡 `dev-<card>.json` + `qa-<card>.json`；每批 `gate.json`、`ledger.json`、`design.json`（design 关闭时无）。批次级工种在本批所有卡测试 PASS 之后再做。
- 每份回执：字段齐；`role` 与文件名一致；agent 以角色名开头；`independent:true`、`declared_by_lead:false`；`batch_id`、`card_id`、`spec_sha256` 对得上；有命令 exit code；`artifacts` 每个文件存在且 sha256 一致。
- dev 回执 agent = 卡的 owner_agent，qa 回执 agent = 卡的 qa_agent；同一 agent 不能出现在两个角色；架构师不能兼任。
- 前端卡（write_scopes 在 `config.frontend.dirs` 下）：dev 回执必须有 `web_guidance`（见 SKILL.md）；qa 回执 `guidance_verified` 覆盖全部 guide_ids。
- 漂移：批次基线以来的业务改动（`config.business_dirs`），除去同期其他批次自己的写入范围，必须全部被本批 dev 回执认领且落在对应卡的 write_scopes 内。同一轮交付内回流不重置基线。

## 闭环

`dispatch → handoff（每卡）→ evidence → accept → release → deliver`，各批次独立走、可并行；全部交付 → `ALL_DELIVERED`。任何时候用户 issue → 定位到 `trace_to_card` 所在批次 → `reopened` → `dispatch --batch`。
