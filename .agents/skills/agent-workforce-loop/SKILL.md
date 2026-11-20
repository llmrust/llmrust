---
name: agent-workforce-loop
description: Multi-agent software delivery loop with separated roles (architect, dev, qa, gate, ci, ledger, design) and a Lead who only dispatches, approves and accepts. A script-enforced state machine (scripts/loop.mjs) gates every step on evidence files, sha256 checks, git authorship and remote CI. CI is mandatory: push a branch, open a PR, let remote CI pass, and a dedicated ci agent records the run — there is no bypass (ci.provider=command/none are rejected at config validation). Use when the user says "别自己写自己测", "按工种跑", "用 loop", "你是 lead 只做验收", "split roles", "lead only reviews", "independent QA", or asks to plan, dispatch, accept or release work with multiple agents. Works with any harness that reads SKILL.md (Claude Code, Codex, Gemini CLI, Copilot, Cursor, OpenCode). Frontend cards must consult Modern Web Guidance before coding.
compatibility: Node.js 18+ and git. gh CLI when ci.provider=github. Network for npx modern-web-guidance on frontend cards (Node 22+).
metadata:
  version: "1.3.0"
  schema: agent-workforce-loop/v1
---

# Agent Workforce Loop（v1，与 harness 无关）

## 铁律（用户定的，优先于本文其他内容）

1. **用户不做技术验收。** 技术结果全部由机器自证（`acceptance.user_confirm` 默认 `false`）。用户只看成品、出 issue。
2. **规则表（config.json）必须有用户同意的佐证，没有就不可信。** 用户本地看过后，本人在自己的终端跑 `approve.mjs config`，输入只有自己知道的审批密码，留下一份签了名的同意记录（绑定规则表当前内容）。没有这份记录、记录被改、签名不是用户的钥匙、规则表在同意后又改过 → 一律不能开工。开工时锁定规则表和审批人，之后改即 `CONFIG_DRIFT` / `APPROVER_KEY_CHANGED`。不需要去 GitHub 点合并。**agent 永远不跑 approve.mjs，不碰、不问审批密码。**
3. **一个任务只归一个人。** 一张卡只被一个任务引用，开发和测试是两个不同的 agent，同一张卡不能同时在两个批次里。
4. **测试等开发交卷。** 开发交卷后代码冻结，测试才开始；测试期间开发再改，这次测试作废。
5. **可并行派批。** 一个相位纵切拆成多个批次，互不依赖、写入范围不重叠、不共用人时同时开工（如前端批 + 后端批），各自走完流程。
6. **机器生成的文件禁止手改。** 项目里的生成文件列进 `config.generated`（路径 + 生成命令），验收和发布时重跑比对，不同即 `GENERATED_HAND_EDITED`。改源头再生成。本 skill 自己的状态表、配置字段表、错误码表、配置模板和 MANIFEST.json 也由 `scripts/gen-docs.mjs` 生成，手改过不了 `validate-skill`。

原则：**每个状态迁移都在脚本内部跑判据**。脚本不放行，就不能往下走；手改状态文件会被 `self_sha256` 识破。脚本只依赖 Node 与 git，不依赖任何 harness 的工具名；项目差异全部写在 `<workforce 目录>/config.json`。

## 加载后第一动作

```bash
SKILL="<本 skill 目录>"     # 例：.agents/skills/agent-workforce-loop
ROOT="<项目仓库根>"
node "$SKILL/scripts/validate-skill.mjs"
node "$SKILL/scripts/selftest.mjs"
node "$SKILL/scripts/loop.mjs" status --repo "$ROOT"
```

- workforce 目录默认 `.workforce/`；用 `--dir <路径>` 或环境变量 `WORKFORCE_DIR` 改（每条命令都要带同一个值）。
- 没有 config 时：`loop.mjs scaffold` 生成 `config.json` 模板 → 执行侧按项目填好并提交 → **交用户本地审核** → 用户本人在自己终端跑 `node "$SKILL/scripts/approve.mjs" setup --repo "$ROOT"`（只第一次）和 `node "$SKILL/scripts/approve.mjs" config --repo "$ROOT"` → 执行侧提交生成的 `approver.pub.json` 与 `approvals/` → `loop.mjs config-check` PASS 才能开工。config-check 会显示审批指纹，请用户核对与自己设置时看到的一致。字段说明见 `references/generated/config-fields.md`，示例见 `references/config.md`。
- `status` 给出 state、当前批次、`allowed_actions`、`last_reasons`。**只做 `allowed_actions` 里的动作**；每个动作前先跑 `guard --action <动作>`，exit 非 0 就停，不绕过、不改脚本、不手改状态文件。
- **用户没确认开哪一批，不起手、不派卡。**

## 角色（详见 references/roles.md）

| 角色 | agent 名 | 只负责 | 禁止 |
|---|---|---|---|
| 架构师 | architect-* | 读既有资料 → 六层 spec + 任务卡 → `architect-spec` / `spec-revise --as architect-*` | 领任务卡、替别人实现 |
| 开发 | dev-*（= 卡的 owner_agent） | 只改本卡 write_scopes，回执列 files_changed；前端卡先跑 Modern Web Guidance | 跨目录写、自测自报 |
| 测试 | qa-*（= 卡的 qa_agent） | 独立测试，先写先 FAIL；回执标 layers_covered + rule_ids | 读开发解释 |
| 门禁 | gate-* | 逐条跑 `config.gates.required_commands`，如实记 exit code | 放宽、标 SKIP、删 FAIL |
| CI | ci-* | 推分支 + 开 PR，用 `gh` 拉**该提交的远端运行**并写 `ci.json`（pushed.head_sha、workflow_runs 的 conclusion、URL） | 本地跑命令冒充 CI、跳过 push、把 `command`/`none` 当 CI |
| 台账 | ledger-* | 脚本生成台账（schema / generatedAt / 自洽的 self_sha256；accept 重跑 `ledger.command` 比对） | 手写、口头补绿 |
| 设计 | design-*（config 可关） | 与原型 / 设计稿并排截图，判一致 | 替开发改代码 |
| Lead | lead | 跑 loop 命令、写 `signoffs/lead.json`、总验收、发布、纠偏 | 写 spec、业务代码、测试、台账、回执；宣布完成 |
| 用户 | 验收者 | 本地审核规则表并用 approve.mjs 签名同意；看成品、出 issue | 技术验收（交给机器） |

同一 agent 不能兼任两个工种；架构师不能兼任执行工种。脚本按 agent 名逐项检查。每个工种必须是**独立的 agent 会话**，怎么在你所在的 harness 里开独立会话见 `references/harness-adapters.md`。

## 审批链（spec 成为执行方案的唯一路径）

```bash
node "$SKILL/scripts/validate-spec.mjs" --repo "$ROOT"
node "$SKILL/scripts/loop.mjs" architect-spec --as architect-<name> --repo "$ROOT"
# 各工种写 signoffs/<role>.json（agent、spec_sha256、agreed:true）
node "$SKILL/scripts/loop.mjs" spec-review --repo "$ROOT"
# Lead 写 signoffs/lead.json（approved:true）
node "$SKILL/scripts/loop.mjs" spec-approve --repo "$ROOT"
# 用户写 signoffs/user.json（confirmed:true）并用自己的邮箱 git commit；config 设 acceptance.user_confirm:false 时省略这步
node "$SKILL/scripts/loop.mjs" spec-confirm --repo "$ROOT"
```

签署绑定 `spec_sha256`：spec 改一个字，全部签署失效，必须 `spec-revise` 重走。`spec-confirm` 同时锁定 config 的 sha，之后改 config 即 `CONFIG_DRIFT`。

## 执行 Loop

```bash
node "$SKILL/scripts/loop.mjs" init --repo "$ROOT"          # 从已确认的 spec 展开批次，逐张校验任务卡
node "$SKILL/scripts/loop.mjs" ready --repo "$ROOT"         # 现在能开工的批次（可并行的会一起列出）
node "$SKILL/scripts/loop.mjs" dispatch [--batch B-x] --repo "$ROOT"  # 只唤醒交卷方（dev/ci）；与在做批次不共用范围 / 卡 / agent，按 parallel.max_batches 并行
# 把 wake 里每个 dev brief 交给一个独立 agent 会话
node "$SKILL/scripts/loop.mjs" handoff --card <id> --repo "$ROOT"     # 交卷方（dev/ci）交卷后冻结代码，才给出 qa 的 brief（测试不与交卷同时进行）
# qa 回执带 handoff_id；本批所有卡测试 PASS 后再唤醒 gate / ci / ledger / design
# ci 工种：先 push 分支 + 开 PR，等远端 CI 跑完，用 gh 拉该提交的运行记录写 ci.json；release 会强制核对（无后门）
node "$SKILL/scripts/check-roles.mjs" [--batch B-x] --repo "$ROOT"    # 只读预检
node "$SKILL/scripts/loop.mjs" evidence [--batch B-x] --repo "$ROOT"  # 回执齐全、独立、sha256 对实物、交接后未改、业务改动全被认领
node "$SKILL/scripts/accept-five-layers.mjs" [--batch B-x] --repo "$ROOT"
node "$SKILL/scripts/loop.mjs" accept [--batch B-x] --repo "$ROOT"    # 五层验收；FAIL 本批回流
node "$SKILL/scripts/loop.mjs" release [--batch B-x] --repo "$ROOT"   # overall 文件 + 工作区干净 + CI（按 config）
node "$SKILL/scripts/loop.mjs" deliver [--batch B-x] --repo "$ROOT"   # 交成品；全部交付后停在 ALL_DELIVERED；不宣布完成
node "$SKILL/scripts/loop.mjs" user-issue --issue "$ROOT/.workforce/issues/ISSUE-001.json" --repo "$ROOT"
node "$SKILL/scripts/loop.mjs" reopen --batch B-x --reason "<原因>" --repo "$ROOT"
node "$SKILL/scripts/loop.mjs" check-drift --repo "$ROOT"   # 随时可跑：业务改动是否都有 dev 回执认领
```

状态与允许动作见 `references/generated/state-machine.md`（生成）与 `references/loop-state-machine.md`（交接、evidence 细则）；错误码见 `references/generated/reason-codes.md`（生成）；文件落点见 `references/file-layout.md`；流程图见 `references/flow.md`。

## 前端卡：Modern Web Guidance（强制）

write_scopes 落在 `config.frontend.dirs` 下的卡是前端卡（`frontend.web_guidance:false` 可关）。dev 在**写代码之前**必须跑 Chrome / Edge / Firefox / MDN 维护的 Modern Web Guidance，`dispatch` 的 wake 会给这类卡带上 `must_first`。

```bash
npx -y modern-web-guidance@latest search "light dismiss dialog"     # 2–5 个英文词，一个能力一次；不带框架名、项目名
npx -y modern-web-guidance@latest retrieve "<id1>,<id2>" > out/mwg-<card>.md
```

- Windows 用 `npx.cmd`；有 pnpm 可用 `pnpx`；离线加 `--offline`。需要外网。
- dev 回执写 `web_guidance`：`queries`、`guide_ids`、`retrieved_artifact`（retrieve 输出存档，同时列进 artifacts 校验 sha）；没有合适指南时 `guide_ids` 留空并写 `none_applicable_reason`。命令原文进 `command_exit_codes`。
- 指南与项目的技术栈裁定冲突时，**裁定优先**，在 `web_guidance.deviations` 写 `guide_id` + `overridden_by`（裁定编号）。指南是 vanilla 写法，落到具体框架时保留特性本身。
- qa 回执的 `guidance_verified` 必须覆盖 dev 的全部 `guide_ids`。
- 目标浏览器写进仓库 `AGENTS.md`（或同等的 agent 说明文件），否则指南默认按 Baseline Widely available 要求做降级。
- 不在单张卡里跑 `legacy-scan`（整库现代化要单独开卡）。

## 五层总验收（references/acceptance-five-layers.md）

event、logic（qa）→ interface（gate 逐条跑 required_commands、mock_used=false，且 qa 覆盖）→ state（ledger 附脚本台账）→ view（design 附截图 + prototype_refs；design 关闭时跳过并记录），外加 config 指定的 overall 文件为 true。任一不满足即 FAIL 并回流。层名也接受中文：事件 / 逻辑 / 接口 / 状态 / 视图。

## 并行与一卡一人

- 一个相位纵切拆成多个批次（如前端批、后端批），互不依赖、写入范围不重叠、不共用 agent 时可以同时开工，上限 `parallel.max_batches`（默认 2）。每个批次自己走 派卡 → 交接 → 回执 → 验收 → 发布 → 交付，互不等待。
- 一张卡只能被 spec 里一个任务引用（`CARD_ASSIGNED_TWICE`），只能有一个开发（owner_agent）和一个不同的测试（qa_agent，`CARD_OWNER_IS_QA`）；同一张卡不能同时在两个批次里（`CARD_TAKEN`）。
- 测试必须等交卷方（dev/ci）交卷：`handoff` 冻结交卷方改动的文件，交接后再改，测试自动作废（按角色给码：`DEV_CHANGED_AFTER_HANDOFF` / `CI_CHANGED_AFTER_HANDOFF`）。详见 `references/loop-state-machine.md`。

## 机器能保证的，和不能保证的

能保证：跳步、一卡多人、测试与开发同时进行、并行批次互相越界、空回执、回执造假（sha 对不上）、agent 兼任、开发越界、确认后改 spec 或 config、手改状态文件、手写台账（配置 `ledger.command` 时重跑比对）、回流期间夹带提交的未认领业务改动（同一轮交付内批次基线不重置）、agent 代用户确认或提 issue，以及 agent 抢先自写 `acceptors.json`（用户提交须过 `trust` 锚：远端受保护分支或签名）。
不能保证：用户把审批密码告诉了别人（含 AI）；第一次 setup 之前 agent 抢先登记了它自己的审批钥匙（防法：config-check 显示审批指纹，用户核对；用户跑 approve.mjs 时也会发现仓库里登记的不是自己）；信任锚本身被绕过（agent 拿到受保护分支的推送权或用户的签名私钥）、伪造 `gh` 或 CI 命令输出、`trust.anchor:"none"` / 无 `ledger.command` 时的对应漏洞。这些降级配置会在 `status` 与 `config-check` 里提示（`TRUST_NONE`、`LEDGER_UNVERIFIED`、`CI_DISABLED`），只在用户明确接受时用。`local_review` 是用户选定的默认。

## 改这个 skill 本身

改流程只改 `scripts/machine.mjs`（状态、动作、配置说明）或 `scripts/lib.mjs`（判据、默认配置），然后：

```bash
node "$SKILL/scripts/gen-docs.mjs"        # 重新生成表格、模板、MANIFEST.json
node "$SKILL/scripts/validate-skill.mjs"  # 含 gen-docs --check：生成文件被手改、文档提到不存在的命令或错误码 → FAIL
node "$SKILL/scripts/selftest.mjs"
```

说明性文字（本文件、`references/*.md`）是人写的，但它提到的命令和错误码必须在代码里真实存在，否则 `validate-skill` 不过。

## Gotchas

- 历史根因：Lead 自己写、自己测、自己判、自己写台账、自己宣布完成 → 必然失败。
- 台账只能脚本产出；叙述性 .md 不能当验收依据。
- 本地与远端门禁结果可能不同（Windows CRLF 等），两边如实记录，不拿远端绿冒充本地绿。
- write_scopes 只靠对比 `files_changed` 与 git 改动，Lead 仍需复核 diff。
- harness 标的「并行」不等于真并行，可能排队，如实记录。
- skill 目录名必须是 `agent-workforce-loop`（与 name 一致），否则部分 harness 不加载。

## 交付口径

只报用户能操作什么、看到什么、与验收口径是否一致、过 / 不过。不谎报完成。

## 硬红线

Lead 写 spec / 业务代码 / 测试 / 台账 / 回执 → 作废；FAIL 标 SKIP、改判据、移出汇总 → 作废；手改 `loop-state.json` → 脚本拒绝一切迁移；以文档、机制、回执、「我准备做」冒充成品 → 作废；在 `release` PASS 之前说「做完 / 完成 / 相位通过」→ 作废。

## 真值源

`<workforce>/spec.json`（已确认版本；项目已有自己的 spec 真源时，用生成脚本从真源产出它，并把它列进 `config.generated`）、`<workforce>/config.json`、仓库的 agent 说明文件（`AGENTS.md` 等）、`config.architect_required_inputs` 列出的项目资料（需求文档、技术栈裁定、原型）。
