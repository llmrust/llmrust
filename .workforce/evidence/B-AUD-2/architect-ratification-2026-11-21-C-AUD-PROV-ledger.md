# architect-llmrust 台账追认工件：B-AUD-2 / C-AUD-PROV 台账 diff（P1 补链 + 卡四条）

**归档说明（Lead 代落盘）**：本文件为 architect-llmrust 于 2026-11-21T02:42:27+08:00 签发的追认工件原文，应架构师产出流由 Lead 归档进 B-AUD-2 evidence 链（ruling【B】「architect 在 accept 前复核追认」环节的完成凭证）。授权链：Boss 2026-11-20 决策 A → architect-llmrust ruling 2026-11-21【B】→ 本追认。

**追认裁定：成立（ratification granted）。**
复核基准：工作区 `tests/hotspot_ledger.json`（58 条 adjustments，实测于 2026-11-21T02:42:27+08:00）。

## 复核方法声明

未采信任何转述（含 Lead 消息与 qa 复测结论）。三腿独立实测：① 43 项结构机判（一次性 python 脚本）；② append-only 对账（`git show bd33b5a:tests/hotspot_ledger.json` 解析级 + 行级 difflib 形态学比对）；③ 从冻结卡 `C-AUD-PROV.json` acceptance_criteria[5] 原样提取 c6 机判命令逐字执行。

## 【1】P1 补链条目（adjustments[53]）——逐字比对 ruling 模板：通过

| 项 | 判定 | 实测 |
|---|---|---|
| 位置/文件 | PASS | `adjustments[53]`，file==`src/providers/ollama.rs`；58 条总计 = bd33b5a 的 53 + 5，恰为前缀后第一条 |
| old/new | PASS | 829 → 830 |
| date | PASS | `2026-11-21`（= ruling 日，R4 授权日语义） |
| authorized_by | PASS（逐字） | 与 ruling 条目规范全串相等：`architect-llmrust ruling 2026-11-21（SPCC §11.1.4 truth correction；授权链：Boss 2026-11-20 决策 A → architect-llmrust 2026-11-21）` |
| reason | PASS（逐字） | 与 ruling 模板全串相等（`appended 2026-11-21（B-AUD-2 台账写入窗口）。genesis chain-documentation backfill（truth correction）：台账于仓库根 bf7dee9 …`，含 805→829 CAP-002 genesis 叙事、P1-OLLAMA-ADJUSTMENT-CHAIN-NON-CONVERGENCE 发现编号、「本条不承载后续任何行数变化」隔离语） |
| 字段形态 | PASS | 恰六字段（file/old/new/reason/authorized_by/date），无多余键 |

## 【2】R1–R4 硬规则自检：全部通过

- **R1（顺序）**：ollama 完整序列（按出现序）`(0→805), (805→829), (829→830 补链), (830→855 卡条目)`；补链位于 idx53，先于卡全部四条（idx54–57）。前缀性质实测：取前缀 54（补链已入、卡条目未入），ollama 链末 new=830 == bd33b5a 批前基线 830——ruling 要求的「任意前缀时刻链面收敛」成立。
- **R2（防吸收）**：补链 new 恰为 830；卡的 ollama 变化独立成条 830→855（+25），两事分立记账。
- **R3（防歧义）**：补链条目 reason 与 authorized_by 均不含字符串 `C-AUD-PROV`（程序化断言通过）。
- **R4（date 语义）**：补链 date=2026-11-21（授权日=ruling 日）；四张卡条目 date=2026-11-20（授权日，ruling【E】语义），真实追加时刻由各条 reason 首段 `appended 2026-11-21` 披露。

## 【3】C-AUD-PROV 自身四条：全部通过

google.rs 1614→1813、anthropic.rs 1539→1684、ollama.rs 830→855、lib.rs 839→847 逐条实测（共 28 项断言全 PASS）：old/new 与冻结卡 c6 预值一致；date == "2026-11-20"；authorized_by == "Boss 2026-11-20 决策 A"（全串相等）；reason 首段均为 `appended 2026-11-21…`；均含卡 id `C-AUD-PROV` 与对应审计项编号（AUD#4 / AUD#5 / AUD#6 / AUD#8）；baseline_lines 四值均==各条 new。±行数算术全对（+199/+145/+25/+8）；ollama 条目披露注入验红证据（对应冻结卡 c3）；lib 条目正确声明 openai.rs（160 行 < threshold）不入台账。

## 【4】append-only（vs bd33b5a committed 台账）：通过

bd33b5a 恰 53 条。工作区对 bd33b5a 的全部差异：
- 解析级：前 53 条逐条相同；顶层变更字段仅 `adjustments`（+5）与 `baseline_lines`（恰 4 值）；description、threshold、compat 2136 及其余 8 基线零变动。
- 行级形态学（difflib，非 equal opcode 恰 4 个）：3 个 replace 全落在 baseline_lines 四行基线值上；1 个 insert 共 40 行纯尾部追加。53 条既有条目文本零改动。行尾两侧一致（445/445 与 485/485 CRLF），行级相同即字节级相同。

## 【5】附加硬核验（独立证据面）

1. **12/12 链收敛**：P1 正式闭合（ollama 0→805→829→830→855==855）。
2. **K-02 双向数值面**：12/12 baseline==实测；splitlines 与守卫 lines().count() 等价性维持。
3. **openai.rs 不在台账**（baseline 与 adjustments 均无）——符合卡面裁定。
4. **冻结 c6 原样执行 exit 0**：ollama「末条 old==830」判据不被补链干扰，正是 R1 机制在起作用。
5. **环境面**：spec sha 实测仍为 `254d36f2461f28aeb8155719bb8950b7260c0db38d72561e17d290112931911b`（无 SPEC_DRIFT）；C-AUD-PROV 卡文件 sha `8c485ac32b2419cd6f05d09049605df5a517302232503e2bc7fea806e1369016`。

## 追认范围与效力声明

本追认覆盖且仅覆盖：工作区 `tests/hotspot_ledger.json` 相对 bd33b5a 的 5 条新增 adjustment（P1 补链 1 条 + C-AUD-PROV 四条）与 4 项 baseline_lines 值变更。ruling【B】的 dev 代写义务与【C】W5 前向硬条件经本工件实测全部满足。**时效条件**：本追认基于 2026-11-21T02:42:27+08:00 的工作区实测，handoff（2a20273362d54212cfab0dba）冻结须保持该文件此后至 evidence/accept 期间零变更——任何变更即本追认自动失效，须重新复核。台账层据此可判 B-AUD-2 台账面收口；B-AUD-3（C-AUD-PROXY）追加时仍按 ruling【E】的 date 语义与 appended 披露执行。
