# Spec 结构：宪章 → 总纵切 → 相位 → 批次 → 工种 → 任务卡

架构师产出 spec，必须是这六层，机器校验见 `scripts/validate-spec.mjs`。

## 六层（自上而下，上层约束下层）

| 层 | 内容 | 机器判据 |
|---|---|---|
| **1 宪章** | 最高权限、不可违反的条文 | `constitution.priority=highest` 且每条有 id/text/enforced_by |
| **2 总纵切** | 整个项目所有模块都为它服务 | `total_vertical_slice.serves_all_modules=true`，有 goal/acceptance |
| **3 相位** | 每个相位有自己的**相位纵切**，服务于总纵切 | `phases[].phase_vertical_slice.{id,goal,serves}`，`serves` 必须等于总纵切 id |
| **4 批次** | 该相位纵切下有哪些批次任务 | `phases[].batches[].{id,goal,tasks}` |
| **5 工种** | 每个任务归哪个工种 | `tasks[].role` ∈ dev/qa/gate/ledger/design（design 关闭时不可用）；architect、lead 不能领卡 |
| **6 任务卡** | 给到具体工种的 `assets/card.template.json` 字段 | `tasks[].card` = 卡 id，对应 `<W>/cards/<id>.json`，字段齐全且 `phase` 与所在相位一致 |

## 审批链

见 `roles.md`。签署不在 spec 里，而是各方各自的 `signoffs/<role>.json`，绑定 spec 的 sha256；spec 改动后签署全部失效。phase / batch / task id 全局唯一（脚本检查重复）。

## 相位纵切与批次的关系

- **总纵切**：整个项目不管多少模块，最终都为它服务。
- **相位纵切**：该相位内的一条端到端可验收能力链（例如 PH-1 的纵切 = 注册→登录→进首页）；这一相位纵切必须能回溯到总纵切。
- **批次**：该相位纵切拆成哪几批任务（含 `[P]` 并行标记与依赖）。
- **任务卡**：批内每张卡写清：工种、精确文件路径、验收命令、判据、独立 QA、阻塞项、checkpoint。

## 与既有资料的关系

架构师必须先读：仓库的 agent 说明文件（`AGENTS.md` 等，含红线与目标浏览器）、`config.architect_required_inputs` 列出的资料（脚本强制）、需求文档、技术栈裁定、原型 / 设计稿、现有台账与 CI 状态。基于**项目现状**制定，不是凭空写。

## 文件

- 模板：`assets/spec.template.json`
- 校验：`scripts/validate-spec.mjs`（结构 + 卡文件 + 审批进度）
- 状态：`scripts/loop.mjs`
- 流程图：`references/flow.md`
