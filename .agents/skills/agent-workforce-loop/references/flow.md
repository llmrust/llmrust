# Agent Workforce Loop 流程图（v1.2）

前置：`scaffold` → 执行侧填写并提交 config.json → 用户本地审核 → 用户本人跑 `approve.mjs config` 签名同意 → 提交同意记录 → `config-check` PASS（核对审批指纹）。

```mermaid
flowchart TD
    AR["architect-* 读既有资料 → 六层 spec + 任务卡"] --> AS{"architect-spec --as architect-*<br/>六层结构"}
    AS -- FAIL --> HALT["停：不绕过、不改脚本"]
    AS -- PASS --> SD["SPEC_DRAFT"]
    SD --> SR{"spec-review<br/>各工种签署绑定 spec sha"}
    SR -- PASS --> SA{"spec-approve<br/>signoffs/lead.json"}
    SA -- PASS --> SC{"spec-confirm<br/>须有用户签名同意规则表；锁定规则表和审批人；user_confirm:true 时还要 signoffs/user.json"}
    SC -- PASS --> INIT["init：从 spec 展开批次，校验卡"]
    INIT --> DISP{"dispatch<br/>blocked_by 已交付 · write_scopes 不交叉 · 业务目录干净"}
    INIT -. "互不依赖、范围不重叠的批次可并行（如前端批 + 后端批）" .-> DISP
    DISP --> WAKE["只唤醒交卷方 dev/ci（前端卡先跑 modern-web-guidance）"]
    WAKE --> HO{"handoff --card：交卷方（dev/ci）交卷 → 冻结代码"}
    HO --> QA["唤醒 qa（回执带 handoff_id）<br/>交接后 dev 再改 → 测试作废"]
    QA --> BR["本批卡全部测完 → gate / ledger / design"]
    BR --> EV{"evidence<br/>回执齐 · 独立 · sha 对实物 · 交接后未改 · 改动全被认领"}
    EV -- FAIL --> WAKE
    EV -- PASS --> ACC{"accept<br/>五层 + overall 文件 + 机器生成文件重跑一致"}
    ACC -- FAIL --> REOPEN["REOPEN 回流本批"]
    ACC -- PASS --> REL{"release<br/>工作区干净 + CI（github / command）"}
    REL -- PASS --> DEL["deliver → DELIVERED：交成品"]
    DEL --> NB["下一个能开工的批次（ready）/ 全部交付 → ALL_DELIVERED"]
    NB --> DISP
    DEL --> U["用户：只出 issue（本人 git 提交）"]
    U --> UI["user-issue → 定位 trace_to_card 所在批次"]
    UI --> REOPEN
    REOPEN --> DISP
    REOPEN -- "需改 spec" --> REV["spec-revise --as architect-*"]
    REV --> SD
    DRIFT["任何时候 spec 被改 → SPEC_DRIFT、config 被改 → CONFIG_DRIFT 拒绝<br/>状态文件被手改 → STATE_TAMPERED 拒绝"]
```

## 纯文本版

```
architect-spec → SPEC_DRAFT → spec-review(工种签署) → spec-approve(Lead) → spec-confirm(锁规则表)
  → init → dispatch(只 dev) → handoff(冻结) → qa → gate/ledger/design → evidence → accept(五层) → release(CI) → deliver
        ↑   （每个批次各走一遍；互不依赖的批次可并行）            │FAIL                    │
        └──────────── 本批 reopened ◄────────────────────────────┘     下一批 / ready ◄───┤
                        ▲                                                  │
                        └──────── user-issue ◄── 用户出 issue ◄────────────┘
REOPEN 需改 spec → spec-revise（架构师）→ SPEC_DRAFT，旧签署全部失效
```
