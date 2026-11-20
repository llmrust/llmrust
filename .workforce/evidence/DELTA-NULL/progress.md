# DELTA-NULL progress (agent: llmrust-dev) — ROUND 2 (card C-DELTA-NULL)

- repo: D:\llmrust-0.1.5, branch `fix/delta-null-stream-chunk`
- spec sha256: `acc8f73302842a3e53de4367053408231d9990b34cb99cbe3483fe7400b3ebe3`
- card: `C-DELTA-NULL` / batch: `B-DELTA-NULL` / task: `T-DELTA-NULL`
- HEAD: `5084e93` (not pushed)

## Commits

| sha | subject | files |
|---|---|---|
| `33a8e3e` | fix(providers): accept null/absent `delta` on OpenAI-compatible terminal stream frames | src/providers/compat.rs, tests/hotspot_ledger.json |
| `5084e93` | chore(hygiene): .gitignore 用递归 glob 覆盖 evidence 子目录日志 | .gitignore |

`5084e93` 是 Lead 写的卫生修复（`dc1e754` 的单星 `.workforce/evidence/*.log`
不匹配 `evidence/<CARD>/` 子目录 ⇒ 4 个 .log 进包 ⇒ package_guard 红），
按 Lead 指令由 dev 落 commit。

## 7 条判据自证结果（全部 PASS）

| 判据 | 结果 | 关键实测 |
|---|---|---|
| c1 类型判据 | **PASS** | `#[serde(default)] delta: Option<CompDelta>`；`#[derive(Deserialize, Default)] struct CompDelta`；python 机判 exit 0 |
| c2 三条回归测试 | **PASS** | 三条独立命令各 exit 0，各自 `1 passed; 0 failed; 0 ignored`；`--list` 中三名各出现 1 次、均 `: test`（无 `: ignored`） |
| c3 注入验红 | **PASS** | 注入后 `cargo test --workspace` **exit 101**，`failures:` 同时含 null/absent 两条；正常增量仍 ok；恢复后 exit 0 且 h1==h0（`1feda361…`） |
| c4 四条门禁 | **PASS** | test/build/fmt/clippy 全 exit 0 |
| c5 两个专项守卫 | **PASS** | architecture_guard `15 passed; 0 failed; 0 ignored` exit 0；package_guard `4 passed; 0 failed; 0 ignored` exit 0 |
| c6 台账一致 | **PASS** | 实测 1899 == baseline 1899；adjustments 末条五字段齐、`Boss 2026-11-20`、date `2026-11-20`、new 1899 |
| c7 零放宽 | **PASS** | c7-1 assert 行 111→118、差集空；c7-2 `#[ignore]/#[should_panic]` 基线 1 行 = 当前 1 行、差集空；c7-3 unit/integration ignored 全 0，Doc-tests 2 ignored 经干净 worktree 实测为**基线既有**（`33a8e3e^` 同为 2） |

## 状态机阻断（需 Lead 裁定 R-1）

`loop.mjs dispatch --batch B-DELTA-NULL` → **exit 1**
`{"verdict":"FAIL","reason":"STATE_FORBIDS state=SPEC_DRAFT action=dispatch allowed=spec-review,spec-revise"}`

`loop-state.json` 仍是 `SPEC_DRAFT`，`signoffs/` 只有 `architect.json`。
本卡无法正式 dispatch/claim。dev 已按卡面命令逐条跑完机判并留证，
等状态机推进到 `EXECUTING` 即可入册。

## 需 Lead/architect 裁定

- **R-1**（阻断 accept）状态机 SPEC_DRAFT，无法 dispatch
- **R-2**（不阻断）c7-2 计数口径：去重行 = 1，出现次数 = 2（`src/proxy/mod.rs` 两处同文本）
- **R-3**（阻断 accept）c7-3 字面「ignored 为 0」vs 实测 Doc-tests 2 ignored（基线既有，非本卡引入）
- **R-4**（阻断 accept）台账 1843→1899 的 architect 追认（§11.1.4）

## 回执

`.workforce/evidence/DELTA-NULL/dev-C-DELTA-NULL.json`
sha256 `9809A09A400F6BD694479E6F97699A9886ACF5386ED9DEA04A3FCA0D1DC8F653`
