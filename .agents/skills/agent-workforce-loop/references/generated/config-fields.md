<!-- 由 scripts/gen-docs.mjs 生成，禁止手改。改 scripts/machine.mjs 或 lib.mjs 后重跑 node scripts/gen-docs.mjs -->

# config.json 字段（生成）

| 字段 | 默认 | 作用 |
|---|---|---|
| `schema` | `"agent-workforce-config/v1"` | 固定 `agent-workforce-config/v1` |
| `business_dirs` | `["src/"]` | 业务代码目录（以 `/` 结尾）。改动必须被交卷方（dev/ci）回执的 files_changed 认领 |
| `roles.design` | `true` | 非 UI 项目设 `false`：不要 design 工种，视图层记 skipped |
| `architect_required_inputs` | `[]` | spec 的 `inputs_read` 必须包含的资料 |
| `gates.required_commands` | `[]` | **必填**。gate 回执必须逐条跑过且 exit 0 |
| `gates.require_no_mock` | `true` | gate 回执须 `mock_used:false` |
| `rule_id_pattern` | `null` | qa 回执 `rule_ids` 的格式正则 |
| `release.overall_file` | `null` | 发布前置的汇总 JSON；`null` 不检查 |
| `release.overall_pointer` | `"/summary/overall"` | 指向必须为 `true` 的字段 |
| `ci.provider` | `"github"` | **只能 `github`**。`command`（只本地跑命令）与 `none` 都是绕开 CI，配置校验直接拒（CONFIG_CI_BYPASS_FORBIDDEN）。CI 是强制项，没有后门 |
| `ci.repo` | `null` | **必填**（provider=github 时）：`owner/name`，CI 跑在哪个远端仓库。release 会用它核对该提交的远端运行记录 |
| `ci.branch` | `null` | 可选：CI 关注的分支名，仅作回执与排查参考 |
| `ci.command` | `null` | **已废弃**（旧版的本地命令口子）。填了会被 CONFIG_CI_LEGACY_FIELDS 拒 |
| `ci.none_reason` | `null` | **已废弃**（旧版的免 CI 口子）。填了会被 CONFIG_CI_LEGACY_FIELDS 拒 |
| `frontend.dirs` | `[]` | 写入范围在这些目录下的卡算前端卡 |
| `frontend.web_guidance` | `true` | 前端卡是否强制 Modern Web Guidance |
| `ledger.command` | `null` | 台账生成命令，accept 时重跑比对 |
| `ledger.none_reason` | `null` | 没有生成器的理由 |
| `acceptance.user_confirm` | `false` | 默认 `false`：用户不做开工确认、不做技术验收，全部机器自证。`true` 时要 `signoffs/user.json` |
| `parallel.max_batches` | `2` | 同时在做的批次上限 |
| `generated` | `[]` | 机器生成文件清单 `[{path, command}]`。accept / release 重跑命令，结果和仓里不同即 GENERATED_HAND_EDITED。改源头再生成，不许手改 |
| `trust.anchor` | `"local_review"` | 默认 `local_review`：用户本地审过规则表后，本人在自己终端跑 `approve.mjs config`，用只有自己知道的审批密码签名同意；没有这份签名，规则表不可信、不能开工。另有 `remote`（受保护分支）、`signed`（git 签名提交）、`none` |
| `trust.remote` | `"origin"` | `remote` 锚的远端 |
| `trust.branch` | `"main"` | `remote` 锚的受保护分支 |
| `trust.allowed_signers` | `null` | `signed` 锚的 SSH allowed_signers 文件 |
| `trust.none_reason` | `null` | `none` 时必填的理由 |
