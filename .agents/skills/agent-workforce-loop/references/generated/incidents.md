<!-- GENERATED-FROM: incidents.json —— 由 scripts/gen-incidents.mjs 生成，禁止手改；gen-docs.mjs --check 逐字节校验 -->

# 失误台账与进化（生成）

唯一真源 = `incidents.json`。本表由 `scripts/gen-incidents.mjs` 机器生成，**禁止手改**；`node scripts/gen-docs.mjs --check` 会逐字节校验（手改即红），`node scripts/check-incidents.mjs` 另做结构/守卫/期限校验。

| id | kind | 现象（客观） | 守卫 code | selftest 用例 | 状态 |
|---|---|---|---|---|---|
| INC-001 | PREMATURE_MERGE | 批次未 accept、ci 工种回执尚未产出时即合并 PR；premerge 门实测判 MERGE_BEFORE_CI_RECEIPT。 | `MERGE_BEFORE_CI_RECEIPT` | — | enforced |
| INC-002 | IDENTITY_FORGERY | Lead 自写 evidence/<batch>/ci.json 并把 declared_by_lead 填 false，事后与真回执无法区分。 | `RECEIPT_WRITTEN_BY_LEAD`<br>`AGENT_IDENTITY_FORGERY` | —<br>— | enforced |
| INC-003 | CIRCULAR_RECEIPT | dev 回执 artifacts 钉同批 qa/gate/ledger 回执的 sha，返工重写后 check-roles 出现 4 条 ARTIFACT_SHA_MISMATCH。 | `RECEIPT_ARTIFACT_SKIPPED` | `[8] 回执 artifacts 钉同批兄弟回执（含 sha 故意写错/尚不存在）→ PASS + NOTE RECEIPT_ARTIFACT_SKIPPED（旧实现会 ARTIFACT_SHA_MISMATCH/MISSING）` | enforced |
| INC-004 | PATH_PARSE_BUG | git status 输出被整体 trim 后固定 slice(3) 吃掉路径首字符（.workforce/… → workforce/…），release 的 ci.json 豁免永久失配、卡 WORKTREE_DIRTY。 | `WORKTREE_DIRTY` | `[7] release 脏检查：已跟踪被修改的 ci.json / loop-state.json 正确豁免（旧 slice(3) 会误报 WORKTREE_DIRTY）` | enforced |
| INC-005 | CRITERIA_UNCHECKABLE | 卡面写「10 job 全部 success」，而本仓 docs-only policy 按设计跳过 ⇒ 该字面判据结构上永不成立。 | `CRITERIA_ABSOLUTE_COUNT_UNVERIFIABLE` | `[9] 绝对数量断言：无结构性跳过说明 → 红；带跳过说明 → 绿` | enforced |
| INC-006 | METRIC_GAMING | 判据写「P95 ≤20ms（min-of-5）」被择优取样掩盖，5/5 轮实际超阈值 ⇒ 判据可被「取最好一次」洗绿。 | `CRITERIA_SAMPLING_AMBIGUOUS` | `[9] 判据抽样歧义：P95 无取样口径 → 红；min-of-N+轮次 → 绿` | enforced |
| INC-007 | SPEC_CHANGED_IN_FLIGHT | 批次在飞时 spec 升级（sha 变化），全部回执/签署变 stale，evidence/accept 判 EVIDENCE_STALE_SPEC / SIGNOFF_STALE_SPEC。 | `CARD_CHANGED_SINCE_INIT` | — | enforced |
| INC-008 | DEAD_SCOPE_PATH | 卡 scope 写了盘上不存在的 crates/…/session/tick_policy.rs（死路径），accept 阶段炸 OUT_OF_WRITE_SCOPE。 | `CARD_SCOPE_DEAD_PATH`<br>`OUT_OF_WRITE_SCOPE` | —<br>— | enforced |
| INC-009 | INTENTIONAL_BYPASS | 用 gh workflow run / workflow_dispatch 伪造「PR 通道绿」；真 PR 通道要求 run 的 event=pull_request。 | `CI_BYPASS_FORBIDDEN` | — | enforced |
| INC-010 | GATE_UNUSABLE_TIMING | premerge 门被放在不可达状态（delivered 的 allowed 为空）⇒ 合并发生在交付之后，真合并时门恒 FAIL（BATCH_STATE_FORBIDS status=delivered action=premerge），门形同虚设。 | `BATCH_STATE_FORBIDS`<br>`MERGE_PR_REFUSED` | `[11] delivered 批次跑 premerge 应进入校验逻辑而非 BATCH_STATE_FORBIDS`<br>— | enforced |
| INC-011 | GATE_BYPASS_BY_SEQUENCE | Lead 在同一条命令里看到 premerge FAIL 之后仍然执行了合并（绕过自己的门）；门只做校验、不绑定合并动作，顺序即可绕开。 | `MERGE_PR_REFUSED`<br>`MERGE_BEFORE_CI_RECEIPT` | `[11] merge-pr：premerge FAIL 时 exit 1 且不调用任何合并 API（merge_called=false）`<br>— | enforced |
| INC-012 | GATE_BOOTSTRAP | 门自身做首次合并时目标 worktree 还没有 merge-pr.mjs（脚本在另一个 worktree），只能退回 gh api 直接合并；门不可用但合并已发生。 | `GATE_BOOTSTRAP_REQUIRED` | `[12] merge-pr 引导自检：无 merge-pr.mjs 且未给 --allow-bootstrap ⇒ exit 1（GATE_BOOTSTRAP_REQUIRED）` | enforced |
| INC-013 | SCOPE_MISSING_REQUIRED_CALLSITE | C-TUI-FRAME-BATCH 的 write_scopes 只列 3 个文件，但判据要求的 timeBeginPeriod→RAII guard 整改必然要改调用方 crates/yesagent-shell/src/interactive/mod.rs ⇒ dev 按判据改必撞 OUT_OF_WRITE_SCOPE（硬拒、无授权通道）。 | `CRITERIA_SCOPE_GAP` | `[9] 判据点名调用点但 scope 未覆盖 → 红；scope 覆盖 → 绿` | enforced |
| INC-014 | CRITERIA_AGGREGATION_AMBIGUOUS | 同一数据两种聚合（min-of-5 vs median-of-5）结论相反：单帧 CPU 比 = ×2.09 假 FAIL vs ×0.97 真 PASS ⇒ 判据不可唯一机判。 | `CRITERIA_AGGREGATION_AMBIGUOUS` | `[9] 相对比值判据未明示聚合 → 红；明示 median-of-5 双侧 → 绿` | enforced |
