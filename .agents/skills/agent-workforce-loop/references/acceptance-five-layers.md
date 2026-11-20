# Lead 五层总验收（event → logic → interface → state → view）

层名在回执与 issue 里可写英文 id 或中文（事件 / 逻辑 / 接口 / 状态 / 视图），脚本统一归一。

由 `loop.mjs accept` 内部执行（`accept-five-layers.mjs` 是同一判据的只读预检）。每层的证据都来自其他工种的回执，回执里的 artifacts 已在 evidence 阶段核对过 sha256。

| 层 | 证据 | 脚本判据 | 禁止 |
|---|---|---|---|
| 事件 | 每张 dev 卡的 `qa-<card>.json` | PASS、所有 exit 0、`layers_covered` 含「事件」 | 只有叙述没有命令输出 |
| 逻辑 | 同上 | `layers_covered` 含「逻辑」，`rule_ids` 非空（如 `CHAT-L-03`；可用 `config.rule_id_pattern` 约束格式） | 用截图当逻辑通过 |
| 接口 | `gate.json` + qa | gate 命令逐条包含 `config.gates.required_commands` 且 exit 0；`require_no_mock` 时 `mock_used:false`；qa 覆盖「接口」 | mock、本地回显、模板 JSON、usage 0 tokens |
| 状态 | `ledger.json` | 附一个 JSON 台账，含 `schema` / `generatedAt` / 自洽的 `self_sha256`；配置了 `ledger.command` 时 accept 重跑比对 | 手写 state 叙述 |
| 视图 | `design.json` | 附 png/jpg/webp 截图，`prototype_refs` 非空；`roles.design=false` 时记 skipped | 用几何 / 静态门禁绿当功能通过 |
| 汇总 | `config.release.overall_file`（可选） | `overall_pointer` 指向的值 `=== true` | — |

任一层 FAIL → 状态自动 `REOPEN`，本批回流。不宣布完成、不签字、不谎报绿。

业务口径固定：只报用户能操作什么、看到什么、与验收口径是否一致、过/不过。不谎报完成。
