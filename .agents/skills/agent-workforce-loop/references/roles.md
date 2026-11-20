# 工种契约（派卡前必读）

六个工种（design 可在 config 关闭）+ Lead + 用户。每个工种各自是独立 agent 会话（开法见 harness-adapters.md），不能由 Lead 兼任，也不能互相兼任（脚本按 agent 名检查）。

| 角色 | agent 名 | 输入 | 输出 | 禁止 |
|---|---|---|---|---|
| 架构师 | architect-* | 项目现状 + 既有资料（AGENTS.md、`config.architect_required_inputs`、需求文档、技术栈裁定、原型、台账、CI 状态） | 六层 spec + 任务卡 + `signoffs/architect.json` | 领任务卡、替别人实现、跳过审批链 |
| 开发 | dev-*（= owner_agent） | 任务卡 | diff + `dev-<card>.json`（files_changed；前端卡含 web_guidance） | 跨 write_scopes、改判据、自测自报通过、前端卡不查 Modern Web Guidance 就动手 |
| 测试 | qa-*（= qa_agent） | diff + 命令输出 | `qa-<card>.json`（layers_covered、rule_ids；前端卡含 guidance_verified） | 读开发解释、替开发补结论 |
| 门禁 | gate-* | `config.gates.required_commands` | `gate.json`（exit code 原文、mock_used） | 放宽判据、标 SKIP、删 FAIL 行 |
| CI | ci-* | 已推送的分支/PR + `gh` | `ci.json`（`provider:github`、`pushed.{remote,branch,head_sha}`、`workflow_runs[].{name,run_id,head_sha,conclusion,url}`、`conclusion`） | 用本地命令冒充 CI、跳过 push、把 `ci.provider=command/none` 当 CI、自报 `bypass_detected` |
| 台账 | ledger-* | 各工种输出 | 脚本生成的台账 + `ledger.json` | 手写、口头补绿 |
| 设计 | design-* | 构建产物 + 原型 / 设计稿 | 并排截图 + `design.json`（prototype_refs） | 替开发改代码 |
| Lead | lead | 回执、spec | `signoffs/lead.json`；跑 loop 命令；验收、发布、纠偏 | 写 spec、业务代码、测试、台账、回执；宣布完成 |
| 用户 | 验收者 | 成品 | 本地审核 `config.json` 后本人跑 `approve.mjs config` 签名同意；`issues/*`；`acceptance.user_confirm:true` 时还要 `approve.mjs spec` | 技术验收（全部机器自证）；把审批密码告诉任何人 |

## 审批链

```
architect-* 产出 spec → architect-spec --as architect-*
  → 各工种写 signoffs/<role>.json → spec-review
  → Lead 写 signoffs/lead.json → spec-approve
  → 用户本人提交 signoffs/user.json → spec-confirm
  → init → dispatch
```

默认（`acceptance.user_confirm:false`）用户不做开工确认、不做技术验收：spec 由工种签署 + Lead 审批生效，技术结果由机器自证。用户只本地审核一次规则表，之后看成品、出 issue。项目要求开工前用户确认时设 `true`，这一步只确认开工范围，不是技术签收。

测试（qa）只能在 Lead 跑过 `handoff --card` 之后开始，回执必须带该次的 `handoff_id`；开发交卷后不得再改，要改须重新交接、测试重做。
