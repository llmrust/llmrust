# 仓库内文件落点（v1）

`<W>` = workforce 目录，默认 `.workforce/`，可用 `--dir` / `WORKFORCE_DIR` 改。

```
<W>/
  config.json                    项目配置（用户本人提交）                ← assets/config.template.json
  acceptors.json                 验收者 git 邮箱（仅 trust.anchor=remote/signed 时需要）← assets/acceptors.template.json
  approver.pub.json              用户的审批公钥（approve.mjs setup 生成）
  approvals/<kind>-<sha12>.json  用户签名的同意记录（approve.mjs config / spec 生成）
  spec.json                      架构师六层 spec                          ← assets/spec.template.json
  cards/<card_id>.json           任务卡                                   ← assets/card.template.json
  signoffs/<role>.json           architect/dev/qa/gate/ledger/design/lead/user 各自签署 ← assets/signoff.template.json
  evidence/<batch_id>/
    dev-<card_id>.json           开发回执（files_changed；前端卡含 web_guidance）
    qa-<card_id>.json            测试回执（layers_covered、rule_ids、handoff_id）
    gate.json                    门禁回执（required_commands、mock_used=false）
    ledger.json                  台账回执（附脚本台账 JSON）
    design.json                  设计回执（截图、prototype_refs；design 关闭时不需要） ← assets/evidence.template.json
  issues/<issue_id>.json         用户 issue（用户本人提交）              ← assets/issue.template.json
  loop-state.json                只由 loop.mjs 写
<config.release.overall_file>   可选：其 overall_pointer 字段须为 true 才能发布
```

谁写什么：

| 文件 | 写入者 | 脚本如何确认 |
|---|---|---|
| spec.json | 架构师 | `produced_by` = `--as`；sha 绑定签署 |
| cards/* | 架构师（随 spec） | init 记录 sha，dispatch 时比对 |
| signoffs/<工种>.json | 对应工种 agent | agent 前缀、spec_sha256 |
| signoffs/lead.json | Lead（Lead 唯一可写的文件） | approved、spec_sha256 |
| config.json | 执行侧 | local_review：须有用户签名同意（approvals/config-*），开工时锁 sha；remote/signed：作者 ∈ acceptors 且过锚 |
| approver.pub.json、approvals/* | 用户本人跑 approve.mjs（执行侧只负责提交） | 验 Ed25519 签名；开工后公钥被换即 APPROVER_KEY_CHANGED |
| acceptors.json、signoffs/user.json、issues/* | 用户 | remote/signed 时：作者 ∈ acceptors 且过锚 |
| config.generated 列出的文件 | 生成命令 | 验收 / 发布时重跑比对，不许手改 |
| evidence/* | 对应工种 agent | 见 loop-state-machine.md |
