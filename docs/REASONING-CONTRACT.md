# llmrust Reasoning / Cache 契约（REA-001）

> **文档编号**：`LLMRUST-REASONING-001`  
> **状态**：`APPROVED-BASIS`（REA-001 裁定物；实施任务 REA-002/003/004G/004O 以此为基准，不得越界）  
> **规格依据**：SPCC-0.1.3 §6.3（Reasoning/Thinking 契约）、§6.1（字段处理三态）  
> **核验日期**：2026-08-01（本表所有"官方证据"均于该日直读，版本/URL 见 §4）  

## 1. 目的与 0.1.3 边界

统一概念名：**`reasoning`**（provider wire 层保留各自官方字段名：`thinking`、`reasoning_content`、`thought` 等）。
0.1.3 不为 reasoning 修改公开响应类型（API freeze）。约束：

1. 设置 reasoning 后，支持路径必须**真正写入请求体**；不支持路径必须**发网前返回 `LlmError::Unsupported`**；
2. `ChatResponse` 当前无法无损表达独立 reasoning → **非流 `chat` 一律 Unsupported**（§6.3）；
3. 原始 `stream` 可用已冻结的 `StreamChunk.thinking` / `StreamChunk.thinking_done` 返回 reasoning，顺序不得重排；
4. `stream_collect_full` 遇 reasoning 不得丢弃：返回明确错误并引导调用方消费原始 stream；（已实现，STR-003 Refs #144：`stream_collect`/`stream_collect_full` 任一 chunk 含非空 thinking 增量或 `thinking_done == true` 即返回 `LlmError::Unsupported` 并指向 raw stream）
5. reasoning token 只在上游明确报告时填充，不推算、不与普通 completion token 重复计算；
6. 未核验的 OpenAI-compatible 第三方端点**不得继承** OpenAI 的支持声明。

## 2. 六路径裁定表

### 2.1 OpenAI（chat completions，官方 openapi.yaml + reasoning guide）

| 路径 | 裁定 | 理由与映射 |
|---|---|---|
| request | `Mapped` | `reasoning_effort` 参数；`Enabled{budget_tokens: None}` → `"medium"`；`Disabled` → 省略；**`budget_tokens: Some(_)` 无 OpenAI 对应字段 → Unsupported**（O-5 已实现：发网前拒绝） |
| chat（非流） | `Unsupported` | §6.3：`ChatResponse` 不可表达；发网前返回错误 |
| raw stream | `Mapped`（字段名容错已实现；E2E fixture 验证中） | `reasoning` 与 `reasoning_content` 双字段容错 → `StreamChunk.thinking`（O-1 部分处理）；该字段不在 openapi.yaml schema 中，属 guide 文档化行为，真实端点核验留在 E2E-001 |
| usage | `Mapped` | `prompt_tokens_details.cached_tokens` → `cache_read_tokens`；usage `reasoning_tokens` → `reasoning_tokens`（openapi 示例含 `"reasoning_tokens": 0`） |
| aggregate | `Unsupported`（明确错误） | §6.3 第 4 条：`stream_collect_full` 遇 thinking 返回错误并指向 raw stream |
| proxy（OpenAI wire） | `Mapped`（PRX-003 实施） | OpenAI→OpenAI 流可无损表达 `reasoning_content`；proxy DTO 属 `UNSTABLE` 可扩展 |

### 2.2 DeepSeek / Moonshot / OpenRouter（OpenAI-compatible wrapper）

| 路径 | 裁定 | 理由 |
|---|---|---|
| 全部六路径 | `Unsupported` | §6.3 第 6 条 + REA-003：未核验官方 reasoning/cache 字段，**不得继承** OpenAI 声明；设置 thinking 时发网前 Unsupported、零网络调用 |

### 2.3 Anthropic（Messages API，官方 SDK 类型）

| 路径 | 裁定 | 理由与映射 |
|---|---|---|
| request | `Mapped` | `thinking: {type: "enabled", budget_tokens}`；`Disabled` → 省略；**`budget_tokens` 上游必填**（SDK 类型必填字段）→ `Enabled{budget_tokens: None}` 发网前 `Unsupported`（O-6）；SDK 已引入 `adaptive` 类型（新模型推荐），0.1.3 冻结的 `ThinkingConfig` 只表达 enabled/disabled → 映射 `enabled`，adaptive 记开放问题 O-2 |
| chat（非流） | `Unsupported` | §6.3 |
| raw stream | `Mapped` | `content_block_start`（thinking block 含 signature）→ `thinking` 增量开始；`content_block_delta` 的 `thinking_delta` → `StreamChunk.thinking`；`signature_delta` → `thinking_done`；`redacted_thinking` block → 视为 thinking 结束（signature 本身不在冻结 DTO 中，O-3） |
| usage | `Mapped` | `cache_creation_input_tokens` → `cache_write_tokens`；`cache_read_input_tokens` → `cache_read_tokens`；`thinking_tokens`（usage 内）→ `reasoning_tokens`；`input_tokens` 语义 = input + cache 两字段之和（SDK 注释），`total_tokens` 保留上游 total 不自行修正（§6.4） |
| aggregate | `Unsupported`（明确错误） | §6.3 第 4 条 |
| proxy（Anthropic wire） | `Mapped`（PRX-004 实施） | 目标 wire 原生支持 thinking 生命周期 |

### 2.4 Google Gemini（v1beta discovery + thinking guide）

| 路径 | 裁定 | 理由与映射 |
|---|---|---|
| request | `Mapped` | `thinkingConfig: {thinkingBudget?, includeThoughts: true}`（REA-004G 已实现）；`thinkingBudget` 上游可选 → `Enabled{budget_tokens: None}` 无损省略；非 thinking 模型设置会返回上游错误（官方文档明示）；`thinkingLevel`（Gemini 3+）无对应字段 → 不表达（O-4） |
| chat（非流） | `Unsupported` | §6.3 |
| raw stream | `Mapped` | `part.thought == true` → `StreamChunk.thinking`（M2-16 基座 + REA-004G 补 `thinking_done` 终结标记，至多一次）；`thoughtSignature` 无法在冻结 `StreamChunk` 表达 → 0.1.3 不携带 signature（O-3），多轮 thought 复用不支持 |
| usage | `Mapped` | `usageMetadata.thoughtsTokenCount` → `reasoning_tokens`（REA-004G 已实现）；`totalTokenCount` 含 thoughts（官方描述） |
| aggregate | `Unsupported`（明确错误） | §6.3 第 4 条 |
| proxy（OpenAI wire） | `Unsupported` | Gemini thought → `reasoning_content` 无官方等价证据，非无损 → 0.1.3 不映射（PRX-003 不得猜测） |

### 2.5 Ollama（native /api/chat，官方 api.md）

| 路径 | 裁定 | 理由 |
|---|---|---|
| request | `Unsupported` | wire 提供 `options.think`（bool 或 `low/medium/high/max`），但 `ThinkingConfig.budget_tokens` 无对应字段 → 无无损映射；REA-004O 裁定全路径 Unsupported、**零网络调用**（已实现，REA-004O Refs #141：chat/stream 双入口发网前 `LlmError::Unsupported`） |
| chat / raw stream / usage / aggregate / proxy | `Unsupported` 或 `NotApplicable` | 请求即拒，后续路径不产生；Ollama usage 仅 prompt/eval count，无 reasoning/cache 字段（`NotApplicable`） |

## 3. 能力声明草案（CAP-001 前身）

| Provider | request | stream | usage(cache/reasoning) | 能力表状态（0.1.3） |
|---|---|---|---|---|
| OpenAI | ✅ Mapped | ✅ Mapped（O-1 已实现） | ✅ Mapped | `implemented`（fixture 后），reasoning 仅 stream |
| DeepSeek / Moonshot / OpenRouter | ❌ Unsupported | ❌ | ❌ | `unsupported` |
| Anthropic | ✅ Mapped | ✅ Mapped | ✅ Mapped | `implemented`（fixture 后） |
| Gemini | ✅ Mapped | ✅ Mapped | ✅ Mapped | `implemented`（fixture 后） |
| Ollama | ❌ Unsupported | ❌ | NotApplicable | `unsupported`（官方 wire 无无损映射） |

## 4. 官方证据清单（核验日期 2026-08-01）

| Provider | 证据 | URL / 来源 | 关键字段 |
|---|---|---|---|
| OpenAI | OpenAPI 规范（master） | <https://github.com/openai/openai-openapi/blob/master/openapi.yaml> | `reasoning_effort`；`usage.reasoning_tokens`；`prompt_tokens_details.cached_tokens` |
| OpenAI | Reasoning guide | <https://platform.openai.com/docs/guides/reasoning> | reasoning 模型参数支持现状；`reasoning_content` 流式行为（O-1） |
| Anthropic | SDK 类型（main） | <https://github.com/anthropics/anthropic-sdk-typescript/blob/main/src/resources/messages/messages.ts> | `ThinkingConfigParam`（enabled/disabled/adaptive）；`thinking_delta`/`signature_delta`；`cache_creation_input_tokens`/`cache_read_input_tokens`；usage `thinking_tokens`；`redacted_thinking` |
| Gemini | v1beta REST discovery | <https://generativelanguage.googleapis.com/$discovery/rest?version=v1beta> | `thinkingConfig`（thinkingBudget/includeThoughts/thinkingLevel）；`part.thought`/`thoughtSignature`；`usageMetadata.thoughtsTokenCount`；`MISSING_THOUGHT_SIGNATURE` |
| Ollama | API 文档（main） | <https://github.com/ollama/ollama/blob/main/docs/api.md> | `options.think`（bool/level）；`message.thinking` |

## 5. 开放问题与去向

| ID | 问题 | 处置 |
|---|---|---|
| O-1 | OpenAI chat completions 的 `reasoning_content` 不在 openapi.yaml schema 中（仅 guide 文档化） | REA-003 已实现 `reasoning`/`reasoning_content` 双字段容错解析；E2E-001 用真实 fixture 核验后能力表转 `verified` |
| O-2 | Anthropic `thinking.type=adaptive`（新模型推荐）与冻结的 `ThinkingConfig`（enabled/disabled）不对齐 | 0.1.3 映射 `enabled`；adaptive 记 0.2 候选 |
| O-3 | `thoughtSignature`/thinking signature 无法在冻结 `StreamChunk` 表达 | 0.1.3 不携带 signature，文档明示限制；多轮 thought 复用不支持 |
| O-4 | Gemini `thinkingLevel`（Gemini 3+）无字段表达 | 0.1.3 不表达；0.2 候选 |
| O-5 | OpenAI `budget_tokens` 无对应参数 | REA-003 已实现：`budget_tokens: Some(_)` → 发网前 Unsupported；O-5 关闭 |
| O-6 | Anthropic `budget_tokens` 上游必填（SDK 类型必填），冻结的 `ThinkingConfig.budget_tokens` 可空 | REA-002 已实现：`Enabled{budget_tokens: None}` → 发网前 Unsupported；O-6 关闭 |

## 6. 实施任务范围映射（对 REA-002/003/004G/004O 的约束）

- **REA-002（Anthropic）**：request `thinking` 映射；chat 发网前 Unsupported；raw stream thinking_delta/signature_delta/redacted_thinking 处理；usage cache/thinking 映射；六条路径 fixture。
- **REA-003（OpenAI-compatible）**：OpenAI `reasoning_effort` 映射（budget 规则见 O-5）；chat Unsupported；raw stream `reasoning_content` → thinking（容错）；usage cached/reasoning 映射；三个 wrapper **必须**保持 Unsupported 并有正负矩阵测试。
- **REA-004G（Gemini）**：`thinkingConfig` 映射；chat Unsupported；thought part → thinking；usage thoughtsTokenCount；proxy 不映射。
- **REA-004O（Ollama）**：全路径 Unsupported、零网络；能力表 `unsupported`。（已实现，REA-004O Refs #141）
- 所有实现任务的 DoD 均含：请求 fixture 精确、非流零网络、原始流顺序/终止正确、None/Some(0) 区分、日志无内容。
