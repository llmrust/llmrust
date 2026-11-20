# qa-llmrust · C-DELTA-NULL 进度（完成）

- 卡：`.workforce/cards/C-DELTA-NULL.json`（批次 `B-DELTA-NULL`）
- handoff_id：`547de603e7cd8b7e12d3ddc2`（已用 lib.mjs 的 handoffs 核对一致）
- spec_sha256：`f31889df63f6419e9d78d9c58121b1e093e31ccb1ac75af6a62cb767e9f0b33c`（与 loop-state.json 一致）
- HEAD：`5084e935c1b527de326657d3cb6328951dfbf8c4`
- 回执：`.workforce/evidence/B-DELTA-NULL/qa-C-DELTA-NULL.json`（38 artifacts）
- **verdict = PASS**

## 独立实测结论（全部自己跑，未采信 dev 数字）

| 判据 | 结果 | 关键实测 |
|---|---|---|
| c1 类型 | PASS | `#[serde(default)] delta: Option<CompDelta>` + `#[derive(Deserialize, Default)] struct CompDelta` |
| c2 三用例 | PASS | 三条独立命令各 exit 0 且结果行逐字命中；`--list` 中三名字 occurrences=1、ignored=False |
| c5 守卫 | PASS | architecture_guard `15 passed; 0 failed; 0 ignored`；package_guard `4 passed; 0 failed; 0 ignored` |
| c6 台账 | PASS | baseline=1899 = python splitlines 实测 1899；末条 old=1843/new=1899/五字段齐/Boss 2026-11-20/date 对 |
| c7 零放宽 | PASS | c7-1 111→118 差集 0；c7-2 基线 2 处=当前 2 处、新增 0；c7-3 18 个二进制 ignored 全 0、Doc-tests ignored=2=基线 |
| c4 门禁（复证） | PASS | test/build/fmt/clippy 四门全 0 |
| compat.rs sha | — | `1feda36119a3a44f1ed95c76b531162d62f00379297067a944bf46722b8140d9`（= 卡面 h0，= handoff 冻结值） |

## 注入验红（自造变异，非照抄 dev）

- 变异 A 消费侧中和 `unwrap_or(&empty_delta)` → `.expect("delta")`：exit 101，红 **2/3**
- 变异 C 完整回退到修复前形态：exit 101，红 **2/3**
- 变异 B 诊断（保留 `serde(default)` 仅去 `Option`）：exit 101，只红 1/3 —— 证明用例①由 `Option` 钉住、用例③由 `serde(default)` 钉住
- 三次还原后 sha 均逐字节回到 `1feda361…`，三条用例全 ok、exit 0

## 真实端点验证（本卡最有价值部分）

- 任务书指定的 `nous/meituan/longcat-2.5:free` 与 `nous/upstage/solar-mini4:free` **不可用**：
  网关 18778 的 nous upstream `127.0.0.1:8647`（nous_relay → Hermes 8645）在 **17:27 被 `_hermes_uninstall.ps1` 卸载**，
  8645/8647 无监听；网关 zen key 也已过期（401 `api_key_expired`）。已如实留证，未假装做过。
- 改用同网关**仍在线**的真实上游 `codearts/deepseek-v4.1-flash` 与 `codearts/glm-5.3`（华为云 IAM），
  实测终止帧正是缺陷形态：`{"choices":[{"finish_reason":"stop","index":0}]}` —— **delta 字段完全缺失**。
- A/B 对比（仓库外 crate，path 依赖分别指向 HEAD 与 worktree `33a8e3e^`=`4b73d66`，直接调 `OpenAiCompatibleProvider::stream`）：
  - **修复前**：`missing field \`delta\`` ⇒ 整个流被 Parse 中断
  - **修复后**：该终止帧正常解析（`finish=Some(Stop)`），流越过该帧
- 补充活证：本地 llama-server `127.0.0.1:8080` 修复后 **28 chunks、done=true、finish=Some(Length)** 完整跑通。

## 发现的独立残差缺陷（不属本卡，已上报 Lead 裁定）

修复后真实端点仍会在末尾中断：`invalid type: null, expected a sequence`（`{"choices":null}` 的 usage 帧）。
根因：`CompStreamChunk.choices: Vec<CompStreamChoice>` 的 `#[serde(default)]` 只覆盖「字段缺失」不覆盖「显式 null」。
该缺陷在 `33a8e3e^` 上同样存在、`33a8e3e` 未触及 `choices` 字段（git diff 可证），故**不影响 c1–c7 判定**，建议另开卡。

## 纪律

- 产品代码零改动（注入后字节还原，sha 一致；`git status` 非 `.workforce` 路径为空）
- 未提交 git；未改 spec/cards/.agents
- 未钉任何 `signoffs/*`、`loop-state.json`、`spec.json`、`cards/*`（循环依赖）
- 真实端点验证 crate 建在 `E:/Temp/qa-delta-null/`（仓库外），避免 package_guard 收进临时产物
- 回执已用仓库自己的 `lib.mjs` 校验：event/logic 两层 PASS，字段检查 problems=(none)
