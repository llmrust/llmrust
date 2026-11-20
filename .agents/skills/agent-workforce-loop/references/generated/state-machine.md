<!-- 由 scripts/gen-docs.mjs 生成，禁止手改。改 scripts/machine.mjs 或 lib.mjs 后重跑 node scripts/gen-docs.mjs -->

# 状态机（生成）

## 全局状态

| 状态 | 允许动作 | 脚本内部检查 |
|---|---|---|
| `IDLE` | `architect-spec` | 六层结构；`produced_by` = 执行者（--as architect-*）；一张卡只能被一个任务引用（CARD_ASSIGNED_TWICE） |
| `SPEC_DRAFT` | `spec-review`、`spec-revise` | 各工种 `signoffs/<role>.json` 齐全；agent 名与角色前缀一致、不复用；绑定当前 spec 的 sha256 |
| `SPEC_REVIEWED` | `spec-approve`、`spec-revise` | `signoffs/lead.json` approved 且绑定当前 spec |
| `SPEC_APPROVED` | `spec-confirm`、`spec-revise` | config.json 已提交且无未提交改动；local_review 时必须有用户对规则表当前内容的签名同意（USER_APPROVAL_MISSING / USER_APPROVAL_SIGNATURE_INVALID / USER_APPROVAL_WRONG_APPROVER）；`acceptance.user_confirm:true` 时还要用户签名同意开工范围。通过后锁定 config 与审批人公钥，之后改即 CONFIG_DRIFT / APPROVER_KEY_CHANGED |
| `SPEC_CONFIRMED` | `init` | 展开批次；卡文件存在、字段齐、phase 一致、开发 ≠ 测试（CARD_OWNER_IS_QA）；blocked_by 指向存在的卡 |
| `EXECUTING` | `dispatch`、`handoff`、`evidence`、`accept`、`release`、`premerge`、`deliver`、`reopen`、`user-issue`、`spec-revise` | 每个动作还要过批次状态表；`premerge` = **合并前置门**（批次须 accepted + ci 工种回执有效 + 当前 HEAD 已在远端 + 该 head 全绿），未过此门不许合并 PR（MERGE_BEFORE_ACCEPT / MERGE_BEFORE_CI_RECEIPT / MERGE_CI_NOT_GREEN） |
| `ALL_DELIVERED` | `user-issue`、`spec-revise` | 全部批次已交付；仍不宣布完成 |

## 批次状态

批次动作用 `--batch <id>` 指定；只有一个批次能做该动作时可省略。

| 批次状态 | 允许动作 | 脚本内部检查 |
|---|---|---|
| `pending` | `dispatch` | 卡自 init 后未改；blocked_by 已交付；本批交卷方卡（dev/ci）写入范围不交叉；与在做批次不超上限（PARALLEL_LIMIT）、范围不重叠（PARALLEL_SCOPE_CONFLICT）、不共用卡（CARD_TAKEN）和 agent（AGENT_BUSY）；相关业务目录干净 |
| `reopened` | `dispatch` | 同 pending；同一轮交付内回流不重置漂移基线；交接全部清空 |
| `dispatched` | `handoff`、`evidence`、`premerge`、`reopen` | handoff：交卷方（dev/ci）合格交卷后冻结文件、发 handoff_id；qa 是验证方，qa 卡不可 handoff（CARD_NOT_WORKER_TASK）。evidence：回执齐、独立、sha 对实物、交接后未改、业务改动全被认领 |
| `evidence_collected` | `accept`、`premerge`、`reopen` | 回执未变；五层验收；机器生成文件重跑一致（GENERATED_HAND_EDITED）；FAIL → reopened |
| `accepted` | `release`、`premerge`、`reopen` | overall 文件为 true；工作区干净；机器生成文件重跑一致；**CI 强制**：必须存在 ci 工种回执 `evidence/<batch>/ci.json` 且 head_sha = 当前 HEAD、全部 workflow conclusion=success；当前 HEAD 必须已在远端分支上（CI_NOT_PUSHED）；缺回执/未推送/有失败/自报 bypass 一律 RELEASE_BLOCKED，没有后门；**合并前必须再过 premerge**（MERGE_BEFORE_ACCEPT / MERGE_BEFORE_CI_RECEIPT / MERGE_CI_NOT_GREEN）——合并早于验收或早于 ci 回执即违规 |
| `released` | `deliver`、`premerge` | 记录交付；全部交付 → ALL_DELIVERED。**premerge 是只读合并前置校验（不迁移状态），记录交付后、真正合并前必须可跑** |
| `delivered` | `premerge` | 已交付。**合并发生在交付之后 ⇒ premerge 必须在此状态可用**；它是只读校验（premergeCheck：批次 accepted/released/delivered + ci 回执绑当前 HEAD + 该 head 真 PR 通道全绿），不是状态迁移；合并只能经 scripts/merge-pr.mjs。用户 issue 可把它重开为 reopened |

只读命令（任何状态可跑）：`status`、`guard`、`check-drift`、`validate-skill`、`scaffold`、`config-check`、`ready`。只有架构师（--as architect-*）能跑：`architect-spec`、`spec-revise`。
