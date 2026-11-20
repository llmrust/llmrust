# architect-llmrust 正式 ruling：P1-OLLAMA-ADJUSTMENT-CHAIN-NON-CONVERGENCE（SPCC §11.1.4 truth correction）

**归档说明（Lead 代落盘）**：本文件为 architect-llmrust 于 2026-11-21 签发的正式 ruling 原文，应架构师本人明确请求由 Lead 归档至 B-AUD-2 evidence 链（架构师无 .workforce/evidence/ 写权）。授权链：Boss 2026-11-20 决策 A → architect-llmrust 2026-11-21 本 ruling。

**签发**：architect-llmrust，2026-11-21。本文件即授权工件，ledger-llmrust 据此独立复核；B-AUD-2 执行时据此补链。

---

验证闭环（第一次调用因我转抄根 hash 手误作废，已用正确 hash 重验）。四态实测全部吻合台账工种的溯源：root bf7dee9 出生态 baseline=830 且文件实测 830 行、链止于 829；b008185 / HEAD(39928fc) / 工作区三态同形；ollama.rs 自根以来零改动；compat 末条 1899→2136 各字段绿。以下为正式 ruling。

## 查证记录（本人独立实测，ruling 的事实基础）

1. 仓库根 `git rev-list --max-parents=0 HEAD` = `bf7dee949fb3527c81b01da00dceb14deee19295`（0.1.5 发版 #250）。
2. **出生态**：`git show bf7dee9:tests/hotspot_ledger.json` 实测——`baseline_lines["src/providers/ollama.rs"]=830`，adjustments 共 51 条，ollama 链恰两条：0→805（#209 注册）、805→829（CAP-002），均 2026-09-11；`git show bf7dee9:src/providers/ollama.rs` 实测 **830 行**（出生时数值面自洽，链面不自洽）。
3. 三后续态：b008185（审计基线）、HEAD=39928fc（52 条）、工作区交付态（53 条）ollama 均 baseline=830、链末 829；`git log -- src/providers/ollama.rs` 仅根 commit 一条——**自根以来零改动**。
4. 工作区第 53 条（C-AUD-COMPAT 1899→2136）六字段齐备，authorized_by「Boss 2026-11-20 决策 A」、date 2026-11-20、reason 含卡 id，new==实测 2136；回执 `batch_ledger_delta_vs_head` 判 PASS，其余 11 项基线与前 52 条前缀逐字相同。
5. 回执链收敛面：12 链 11 收敛，唯一不收敛 = ollama.rs；守卫（architecture_guard L239-313）确实只验 baseline==实测与 threshold/存在性，不校验链——链校验面系台账工种本批新增（回执 guard_semantics_note，与本人对该守卫源码的既有阅读一致）。

## 【A】定性裁定：同意——**genesis chain-documentation gap（基因性链文档缺口）**

**结论**：P1 定性为「台账于仓库出生态（bf7dee9）即携带的链文档缺口」，非基线漂移、非本批违规、亦非 committed 历史上任何一次 append-only 违规。

**依据**：① 出生态 baseline 已是 830 而链终于 829，此后 11 个 committed 状态从未存在 829 作为基线值——即「829→830 的未记账变更」这一事件**从未发生**，缺口不是变更事件，是出生记录自身不闭合；② 数值面（K-02 机器锚）在全部四个已验证态恒真（830==830），从未漂移、从未红过；③ ollama.rs 自根零改动，排除任何后置改文件改基线的可能；④ 该缺口对既有全部门禁不可见（守卫无链校验），直到 B-AUD-1 台账工种建立链收敛面——发现即工种尽责，属预期行为。

**边界（不淡化）**：这是自出生起就存在的真实缺陷（预 git 时代 CAP-002 记账与 0.1.5 入库之间 +1 行被直接吸收进出生基线，具体成因不可考也不必断言），必须以 append-only truth correction 清偿，不得改写、不得搁置。台账工种溯源质量高，本人追认其 P1 记录与 provenance 全部成立。

## 【B】处置时序裁定：同意——补链 entry 落 **B-AUD-2 / C-AUD-PROV 台账写入窗口**

**结论**：追加一条 ollama.rs **829→830** 的 truth correction adjustment，由 C-AUD-PROV 的 dev 代写、在其本批台账 append 序列中**先行追加**，本人在 B-AUD-2 accept 前复核追认（机制：Lead 将 B-AUD-2 台账 diff 转我复核）。

**依据**：① B-AUD-1 内不可修——台账被 C-AUD-COMPAT handoff 冻结，批内任何写入触发 DEV_CHANGED_AFTER_HANDOFF、作废三卡 QA，代价（三卡重验）远高于收益（零数值变化，K-02 机器面全绿）；② 批间间隙同样不可写——B-AUD-1 accept 后、B-AUD-2 dispatch 前无活动卡持有 `tests/hotspot_ledger.json` 写入范围，任何写入都是无卡授权裸写，违反 §11.1.4；③ C-AUD-PROV 是首个机制合法窗口：冻结 scope 本就含 ollama.rs + 台账，其 c6 已强制 ollama 末条 adjustment `old==830`——与补链**完全相容**；④ 回执 remediation_hint 的备选「勘误 CAP-002 条目 new 值」**明确拒绝**——改写已提交条目违反 K-02 append-only 纪律，只能向前追加。

**条目规范**（供 dev 代写、ledger 复核逐字执行）：

```json
{
  "file": "src/providers/ollama.rs",
  "old": 829,
  "new": 830,
  "reason": "appended <真实追加日>（B-AUD-2 台账写入窗口）。genesis chain-documentation backfill（truth correction）：台账于仓库根 bf7dee9 首次入库时 baseline_lines[ollama.rs] 已为 830 且文件实测 830 行，而 adjustments 链止于 829（0→805 #209、805→829 CAP-002，均 2026-09-11）；committed 历史从未存在 829 作为基线值，ollama.rs 自根以来零改动。缺口由 ledger-llmrust 于 B-AUD-1 台账核验（P1-OLLAMA-ADJUSTMENT-CHAIN-NON-CONVERGENCE）发现，architect-llmrust 2026-11-21 ruling 定性为 genesis 缺口并授权本补链。本条不承载后续任何行数变化（由各自条目记账）。",
  "authorized_by": "architect-llmrust ruling 2026-11-21（SPCC §11.1.4 truth correction；授权链：Boss 2026-11-20 决策 A → architect-llmrust 2026-11-21）",
  "date": "2026-11-21"
}
```

**硬规则**（违反任一即 B-AUD-2 台账层 FAIL）：R1 **顺序**——补链条目必须先于 C-AUD-PROV 自身四条 adjustment 追加（保证任意前缀时刻链面收敛：补链后链末 830 == 批前基线 830）；R2 **防吸收**——`new` 必须恰为 830，C-AUD-PROV 自身造成的 ollama.rs 行数移动一律由其自己的条目（old==830，c6 已强制）承载，禁止合并成 829→N；R3 **reason 不得含字符串 "C-AUD-PROV"**（引用批次窗口即可）——保证冻结判据的机器语义无歧义；R4 date=授权日（本 ruling 日 2026-11-21，语义见【E】），真实追加日在 reason 首段披露。

**泛化条款**：本次全扫已排除其余 11 链的同类缺口；若 B-AUD-2 核验时链面再发现 genesis 类缺口，按同一规范处理（逐条报本人授权、同窗口补链），不得各自发挥。

**非约束偏好**：回执观察到工作区台账被 CRLF 整文件重写（445 行 vs HEAD LF 437 行）；B-AUD-2 追加时建议以 LF 行尾保存，减少整文件 diff 噪音——记录性偏好，非判据，改不改不影响任何机判。

## 【C】B-AUD-1 批级台账验收口径裁定：同意——**PASS 成立（范围性）**

**结论**：在 A 成立、B 已排期的前提下，B-AUD-1 台账工种验收范围为：① 本批基线变更——compat.rs 1899→2136 条目授权与链完整（回执已逐项判绿）；② 全 ledger 机器自洽——threshold=500 在位、12/12 baseline==实测（K-02 双向）、53/53 六字段齐备、覆盖扫描零未登记超限文件；③ **P1 如实记录**（含 provenance 与修复路径）——记录是 PASS 的前提而非障碍。P1 列为批外 genesis 债务、修复由 B-AUD-2 承载；ledger 回执可在该口径下给出 PASS。

**依据**：① P1 不属于 B-AUD-1 任何冻结判据（三卡 c6 已逐项机判绿；链收敛面是工种自建核验面，非卡判据）；② 唯一的本批内修复路径产生严格更差结果（冻结违规 + 三卡 QA 作废 + 零数值收益）；③ K-05「不虚报」要求的是**如实记录**——P1 原文保留前提下的 PASS 不是虚报，省略 P1 才是；④ 债务不烂尾的保证是**前向执行**，不是本批强红。

**回执重发要件**（ledger-llmrust 若复核同意本 ruling，按此重发 `.workforce/evidence/B-AUD-1/ledger.json`）：W1 顶层 `verdict` 置 **PASS**（批内验收语义），并注明「per architect ruling 2026-11-21」；W2 内层 `chain_convergence.verdict` **保持 FAIL**（全 ledger 链收敛是客观事实，补链落地前不变，B-AUD-2 核验时须翻身）；W3 P1 条目内容逐字保留，仅在其 severity/remediation 字段追加本 ruling 引用与 B-AUD-2 落点；W4 **必须写明机器盲区**：C-AUD-PROV 冻结 c6 对「缺补链条目」不敏感（其 fix 条目 old==830 照样过机）——B-AUD-2 台账核验面 + 本 ruling 是补链在位的**唯一**执行线，故 W5 前向硬条件：B-AUD-2 台账回执必须逐项核验补链条目（file/old==829/new==830/顺序在 fix 条目之前/reason 规范/授权链），缺任一即 B-AUD-2 台账层 FAIL；W6 重算 self_sha256。事实字段（实测数、sha、command_exit_codes）以 17:17:51Z 回执为准，无需重跑。

**效力边界**：PH-0.1.7 整役的任何后续 accept 都不可能在缺口在场时闭环——B-AUD-2 台账核验是硬闸，债务执行有时点、无豁免。

## 【D】替代路径（仅当 ledger-llmrust 独立复核后仍判 FAIL 时适用）

**本 ruling 不主张本批内解决**（C 生效）。ledger-llmrust 的独立复核权不受本 ruling 强制；若其复核后仍坚持全 ledger 链收敛为 B-AUD-1 硬判据，则按以下路径执行，成本本人认可：

1. Lead 以 loop 支持的 redo/作废机制解除 B-AUD-1 handoff 冻结（明知此步作废三卡已做 QA）；
2. dev-llmrust-cs（B-AUD-1 唯一台账卡持有者）在解冻窗口内按【B】规范**仅追加补链条目**（单文件单条目，本 ruling 即其 §11.1.4 授权），不动其他任何内容；
3. 重新 handoff 冻结；受影响验证面重跑——C-AUD-COMPAT c6 机判（单条 python）、台账链收敛复核、C-AUD-DOCS / C-AUD-GUARD 的 c6 负向断言复核（「无本卡条目」仍为真）——全部为分钟级机判，非人天级；
4. B-AUD-1 按全链收敛口径 accept。

**无论走哪条路径均禁止**：改写 CAP-002 已提交条目（违反 append-only/K-02）；为补链条目伪造成出时代日期（K-05，date 只能记授权/追加时点）；把 829→830 并进 C-AUD-PROV 自身条目（错误归因）；B-AUD-2 在补链缺席时放行。

## 【E】附注裁定（相邻事项，防 B-AUD-2 机判自撞，随本 ruling 生效）

三张台账卡冻结 c6 均钉 `date=='2026-11-20'`，而战役执行已跨入 2026-11-21，B-AUD-2/B-AUD-3 后续追加若按字面「今天」写 date 将撞冻结判据。裁定：**adjustments[].date 的规范语义 = 授权日**（该 truth correction 的授权事件日；本役 C-AUD-PROV/C-AUD-PROXY 各条目的授权事件 = Boss 2026-11-20 决策 A + 本人 2026-11-20 卡内 ruling_refs 预授权，故 `date="2026-11-20"` 为真值，非追加时间戳）；真实追加时刻由 git 历史承载，并要求 B-AUD-2/B-AUD-3 新增条目 reason 首段以 `appended YYYY-MM-DD` 披露（对冻结 c6 的 reason 判据是纯加法，零冲突）。补链条目授权源是本 ruling，故其 date=2026-11-21。B-AUD-1 已写的 compat 条目（date=2026-11-20，回执已判绿）在该语义下自洽，无需任何改动。

---

**ruling 小结**：A 同意（genesis 链文档缺口，四态实测背书）；B 同意（B-AUD-2/C-AUD-PROV 窗口先行补链，条目规范+四硬规则）；C 同意（B-AUD-1 范围性 PASS，回执重发六要件，债务前向硬执行于 B-AUD-2）；D 不主张本批修，fallback 为 redo-handoff 最小补链路径；E date=授权日语义裁定。
