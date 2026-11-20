# llmrust × agent-workforce-loop 移植进度

- 席位：llmrust-bootstrap（引导席位）
- 目标仓：`D:\llmrust-0.1.5`（v0.1.5 tag，HEAD `bf7dee9`）
- 来源：`D:\beeli-new-0.1.0\.agents\skills\agent-workforce-loop\`
- 纪律：只做移植与配置；`src/**` 一行未动；不提交 git

## 1. 复制

- `.agents/skills/agent-workforce-loop/` 整目录复制，**39 个文件**（复制时含来源仓的 `.selftest.log`，已删除，不计入交付）。
- 实际落盘 **37 个文件**：`SKILL.md`、`CHANGELOG.md`、`incidents.json`、`MANIFEST.json`、`scripts/*.mjs` ×15、
  `references/*.md` ×8、`references/generated/*.md` ×4、`assets/*.template.json` ×7。
- 删除来源的 `.selftest.log`：那是 beeli 侧的历史跑分，带过来会与 llmrust 实测不符（MANIFEST 也把它排除在外）。

## 2. 适配（只动两处目录口径，判据逻辑零改动）

| 文件 | 改动 | 性质 |
|---|---|---|
| `scripts/check-card-scopes.mjs` | `GLOBAL` 白名单由 `evidence/ .workforce/ crates/ tools/ scripts/` 扩为**并集**再加 `src/ tests/ benches/ examples/` | 目录口径，只加不减 |
| `scripts/check-criteria.mjs` | `RS_PATH` 的 `.rs` 前缀由硬编码 `crates|tools|scripts|.workforce` 改为「静态基线 ∪ `config.business_dirs` ∪ Rust 常见目录 `src tests benches examples`」 | 目录口径，只加不减 |

未改：`lib.mjs`（校验规则）、`loop.mjs`（状态机）、`machine.mjs`（状态表）、`selftest.mjs`（用例）、`incidents.json`、所有 `references/*.md`。

改完按 skill 自身规矩跑了 `node scripts/gen-docs.mjs` 重生成机器文档与 `MANIFEST.json`（否则 `validate-skill` 会报 `MANIFEST_DRIFT`）。
重生成产物：`references/generated/{state-machine,config-fields,reason-codes,incidents}.md`、`assets/config.template.json`、`MANIFEST.json`。

## 3. 配置

`.workforce/config.json`（schema `agent-workforce-config/v1`）关键字段：

- `business_dirs`: `["src/"]`
- `gates.required_commands`: `["cargo build --workspace", "cargo test --workspace"]`
- `ci.provider`: `github`；`ci.repo`: `llmrust/llmrust`
- `trust.anchor`: `none`，`trust.none_reason` = 零人签模式，验收以 CI + 判据机器输出为准
- `roles.design`: `false`（库，无 UI）
- `frontend.web_guidance`: `false`、`frontend.dirs`: `[]`（无前端）
- `ledger.none_reason`：本仓无台账生成脚本，台账层只验 `self_sha256` 自洽
- `architect_required_inputs`：`AGENTS.md`、`AGENT_MANIFESTO.md`、`docs/{PROJECT_MAP,CONTRACTS,CAPABILITIES}.md`
- `acceptance.user_confirm`: `false`；`parallel.max_batches`: `2`

`.workforce/acceptors.json`：`{"schema":"agent-workforce-acceptors/v1","emails":["exec@local"],"note":"零人签模式"}`

`.gitignore` 追加 `.workforce/loop-state.json`（原文件无此条）。

## 4. 验证

| 项 | 命令 | 结果 |
|---|---|---|
| 结构自检 | `node .agents/skills/agent-workforce-loop/scripts/validate-skill.mjs` | **PASS**（checked_files 37，reasons []） |
| 自测 | `node .agents/skills/agent-workforce-loop/scripts/selftest.mjs` | `{"verdict":"FAIL","cases":128,"failed":15}` |
| beeli 基线（同机同 Node 复跑） | 来源目录同一命令 | `{"verdict":"FAIL","cases":128,"failed":15}` |
| 状态 | `node .../loop.mjs status --repo .` | `state: IDLE`，`config_problems: []` |

selftest 的 15 个失败与 beeli 基线**逐行完全一致**（用例名 + exit code + reason 载荷），零新增失败；
`cases` 也是 128（来源目录里 `.selftest.log` 记录的 103/16 是旧版历史日志，不是当前代码的跑分）。
该 skill 的 selftest 期望 `verdict: PASS`，但基线的 [4] 并行/审批场景 15 例在 Windows 上本就红，属既有基线红，不是本次适配引入。

补充验证：

- `loop.mjs config-check` 在**未提交**的当前仓里报 `CONFIG_ACCEPTORS_NOT_COMMITTED .workforce/acceptors.json`（预期：文件还没 git commit）；
  在临时 git 仓里用同一份 config/acceptors 以 `exec@local` 提交后 → **PASS**，仅余 `TRUST_NONE` / `LEDGER_UNVERIFIED` 两条已知降级提示。
- 适配后的守卫仍咬人（负例实测）：判据点名 `src/types.rs` 但 scope 只给 `src/providers/` → `CRITERIA_SCOPE_GAP` exit 1；
  scope 写 `src/nope/missing.rs` → `CARD_SCOPE_DEAD_PATH` exit 1。
- 门禁命令实测可跑：`cargo build --workspace` exit 0；`cargo test --workspace` exit 0（全绿）。

## 5. 需要裁定

无。两处适配都是目录口径的**并集扩张**，不触碰任何判据逻辑，不需要放宽规则。
