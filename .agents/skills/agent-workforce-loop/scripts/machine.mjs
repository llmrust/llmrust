/**
 * 状态机与配置字段的唯一真源。loop.mjs 用它判定迁移，gen-docs.mjs 用它生成 references/generated/*.md。
 * 改流程只改这里，然后跑 node scripts/gen-docs.mjs；生成的文档禁止手改（validate-skill 会拦）。
 */

/** 全局状态：动作 + 迁移时脚本内部检查 */
export const STATES = {
  IDLE: { allowed: ['architect-spec'], checks: '六层结构；`produced_by` = 执行者（--as architect-*）；一张卡只能被一个任务引用（CARD_ASSIGNED_TWICE）' },
  SPEC_DRAFT: { allowed: ['spec-review', 'spec-revise'], checks: '各工种 `signoffs/<role>.json` 齐全；agent 名与角色前缀一致、不复用；绑定当前 spec 的 sha256' },
  SPEC_REVIEWED: { allowed: ['spec-approve', 'spec-revise'], checks: '`signoffs/lead.json` approved 且绑定当前 spec' },
  SPEC_APPROVED: { allowed: ['spec-confirm', 'spec-revise'], checks: 'config.json 已提交且无未提交改动；local_review 时必须有用户对规则表当前内容的签名同意（USER_APPROVAL_MISSING / USER_APPROVAL_SIGNATURE_INVALID / USER_APPROVAL_WRONG_APPROVER）；`acceptance.user_confirm:true` 时还要用户签名同意开工范围。通过后锁定 config 与审批人公钥，之后改即 CONFIG_DRIFT / APPROVER_KEY_CHANGED' },
  SPEC_CONFIRMED: { allowed: ['init'], checks: '展开批次；卡文件存在、字段齐、phase 一致、开发 ≠ 测试（CARD_OWNER_IS_QA）；blocked_by 指向存在的卡' },
  EXECUTING: { allowed: ['dispatch', 'handoff', 'evidence', 'accept', 'release', 'premerge', 'deliver', 'reopen', 'user-issue', 'spec-revise'], checks: '每个动作还要过批次状态表；`premerge` = **合并前置门**（批次须 accepted + ci 工种回执有效 + 当前 HEAD 已在远端 + 该 head 全绿），未过此门不许合并 PR（MERGE_BEFORE_ACCEPT / MERGE_BEFORE_CI_RECEIPT / MERGE_CI_NOT_GREEN）' },
  ALL_DELIVERED: { allowed: ['user-issue', 'spec-revise'], checks: '全部批次已交付；仍不宣布完成' },
};

/** 批次状态：每个批次独立走，互不依赖的批次可并行 */
export const BATCH_STATES = {
  pending: { allowed: ['dispatch'], checks: '卡自 init 后未改；blocked_by 已交付；本批交卷方卡（dev/ci）写入范围不交叉；与在做批次不超上限（PARALLEL_LIMIT）、范围不重叠（PARALLEL_SCOPE_CONFLICT）、不共用卡（CARD_TAKEN）和 agent（AGENT_BUSY）；相关业务目录干净' },
  reopened: { allowed: ['dispatch'], checks: '同 pending；同一轮交付内回流不重置漂移基线；交接全部清空' },
  dispatched: { allowed: ['handoff', 'evidence', 'premerge', 'reopen'], checks: 'handoff：交卷方（dev/ci）合格交卷后冻结文件、发 handoff_id；qa 是验证方，qa 卡不可 handoff（CARD_NOT_WORKER_TASK）。evidence：回执齐、独立、sha 对实物、交接后未改、业务改动全被认领' },
  evidence_collected: { allowed: ['accept', 'premerge', 'reopen'], checks: '回执未变；五层验收；机器生成文件重跑一致（GENERATED_HAND_EDITED）；FAIL → reopened' },
  accepted: { allowed: ['release', 'premerge', 'reopen'], checks: 'overall 文件为 true；工作区干净；机器生成文件重跑一致；**CI 强制**：必须存在 ci 工种回执 `evidence/<batch>/ci.json` 且 head_sha = 当前 HEAD、全部 workflow conclusion=success；当前 HEAD 必须已在远端分支上（CI_NOT_PUSHED）；缺回执/未推送/有失败/自报 bypass 一律 RELEASE_BLOCKED，没有后门；**合并前必须再过 premerge**（MERGE_BEFORE_ACCEPT / MERGE_BEFORE_CI_RECEIPT / MERGE_CI_NOT_GREEN）——合并早于验收或早于 ci 回执即违规' },
  released: { allowed: ['deliver', 'premerge'], checks: '记录交付；全部交付 → ALL_DELIVERED。**premerge 是只读合并前置校验（不迁移状态），记录交付后、真正合并前必须可跑**' },
  delivered: { allowed: ['premerge'], checks: '已交付。**合并发生在交付之后 ⇒ premerge 必须在此状态可用**；它是只读校验（premergeCheck：批次 accepted/released/delivered + ci 回执绑当前 HEAD + 该 head 真 PR 通道全绿），不是状态迁移；合并只能经 scripts/merge-pr.mjs。用户 issue 可把它重开为 reopened' },
};

export const ALLOWED = Object.fromEntries(Object.entries(STATES).map(([k, v]) => [k, v.allowed]));
export const BATCH_ALLOWED = Object.fromEntries(Object.entries(BATCH_STATES).map(([k, v]) => [k, v.allowed]));
export const ARCHITECT_ONLY = ['architect-spec', 'spec-revise'];
export const READONLY = ['status', 'guard', 'check-drift', 'validate-skill', 'scaffold', 'config-check', 'ready'];

/** config.json 每个字段的说明。DEFAULT_CONFIG 里有而这里没有（或反之），gen-docs 直接失败 */
export const CONFIG_DOCS = {
  'schema': '固定 `agent-workforce-config/v1`',
  'business_dirs': '业务代码目录（以 `/` 结尾）。改动必须被交卷方（dev/ci）回执的 files_changed 认领',
  'roles.design': '非 UI 项目设 `false`：不要 design 工种，视图层记 skipped',
  'architect_required_inputs': 'spec 的 `inputs_read` 必须包含的资料',
  'gates.required_commands': '**必填**。gate 回执必须逐条跑过且 exit 0',
  'gates.require_no_mock': 'gate 回执须 `mock_used:false`',
  'rule_id_pattern': 'qa 回执 `rule_ids` 的格式正则',
  'release.overall_file': '发布前置的汇总 JSON；`null` 不检查',
  'release.overall_pointer': '指向必须为 `true` 的字段',
  'ci.provider': '**只能 `github`**。`command`（只本地跑命令）与 `none` 都是绕开 CI，配置校验直接拒（CONFIG_CI_BYPASS_FORBIDDEN）。CI 是强制项，没有后门',
  'ci.repo': '**必填**（provider=github 时）：`owner/name`，CI 跑在哪个远端仓库。release 会用它核对该提交的远端运行记录',
  'ci.branch': '可选：CI 关注的分支名，仅作回执与排查参考',
  'ci.command': '**已废弃**（旧版的本地命令口子）。填了会被 CONFIG_CI_LEGACY_FIELDS 拒',
  'ci.none_reason': '**已废弃**（旧版的免 CI 口子）。填了会被 CONFIG_CI_LEGACY_FIELDS 拒',
  'frontend.dirs': '写入范围在这些目录下的卡算前端卡',
  'frontend.web_guidance': '前端卡是否强制 Modern Web Guidance',
  'ledger.command': '台账生成命令，accept 时重跑比对',
  'ledger.none_reason': '没有生成器的理由',
  'acceptance.user_confirm': '默认 `false`：用户不做开工确认、不做技术验收，全部机器自证。`true` 时要 `signoffs/user.json`',
  'parallel.max_batches': '同时在做的批次上限',
  'generated': '机器生成文件清单 `[{path, command}]`。accept / release 重跑命令，结果和仓里不同即 GENERATED_HAND_EDITED。改源头再生成，不许手改',
  'trust.anchor': '默认 `local_review`：用户本地审过规则表后，本人在自己终端跑 `approve.mjs config`，用只有自己知道的审批密码签名同意；没有这份签名，规则表不可信、不能开工。另有 `remote`（受保护分支）、`signed`（git 签名提交）、`none`',
  'trust.remote': '`remote` 锚的远端',
  'trust.branch': '`remote` 锚的受保护分支',
  'trust.allowed_signers': '`signed` 锚的 SSH allowed_signers 文件',
  'trust.none_reason': '`none` 时必填的理由',
};



