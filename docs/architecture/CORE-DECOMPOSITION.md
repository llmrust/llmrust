# Core 热点拆分推演（ARC-002）

> **状态**：推演文档（M4 任务卡，纯设计，零生产代码改动）
> **日期**：2026-08-03
> **依据**：SPCC §11.7 ARC-002 卡（初始状态/目标/允许范围/禁止范围/执行步骤/DoD/回证）
> **目标文件**：`src/providers/compat.rs`（1455 行）、`src/providers/google.rs`（1524 行）、`src/providers/anthropic.rs`（1483 行）、`src/types.rs`（1132 行）、`src/router.rs`（851 行）——合计 **6445 行**
> **产出**：行区间映射、依赖图、目标模块树、API 路径影响表、迁移顺序、未来任务卡清单（≥5，含测试外迁方向）、风险清单
> **格式先例**：`docs/architecture/PROXY-DECOMPOSITION.md`（ARC-001 交付物；步骤↔卡严格 1:1、目标模块三处一致）

## 0. 背景与约束

- M3 封板后热点台账复核（ARC-002 开工熔断）发现 `anthropic.rs` 台账 1484 系 REA-002 关账 off-by-one 误记，已由架构师经 GOV PR #171 真相修正为 **1483**（`baseline_lines` + `adjustments` 落地 main @ `e901a08`；「ledger is truth, never headroom」先例）。
- 本卡**只产出推演文档**，`src/**`、`tests/**`、`tests/hotspot_ledger.json` 逐字节零 diff。
- 五个热点文件结构性根因同 ARC-001：**测试与生产同文件 + 职责堆积**。
- **public API freeze（0.1.x）**：`types.rs` 属 L0 公开 API 面（`docs/api-inventory.json` 中 STABLE / STABLE-ADDITIVE 分类载体；`FinishReason` 变体集冻结、`ChatRequest`/`EmbeddingRequest` `#[non_exhaustive]`）。任何未来拆分必须保持公开 module path 与 re-export 兼容（§4 API 路径影响表）。
- **RTR-001 边界**：Router 单计数器跨组干扰是独立行为修复卡（RTR-001），本卡仅列结构设计影响（§7 风险/§6 C13 依赖），**不夹带行为修复方案**。
- 目标模块树**禁止** `common`/`shared`/`utils` 万能层；wire DTO 不得进入 `types.rs`（§4.3）。
- 分层：`types` 属 L0 不得依赖 provider/router/proxy；`providers` 属 L2；`router` 属 L3 可依赖 LmrsClient。
- 迁移顺序：每步 ≤400 行人工 diff、独立回滚点、golden fixture 前置（公开 API freeze 锚点迁移策略单列 §5.0）。
- 未来任务卡是否实施归 Owner 后续选择（0.1.4+ 候选），本卡不实施。

## 1. 现状行区间映射

### 1.1 compat.rs（OpenAiCompatibleProvider，1455 行）

| 行区间 | 职责 | 行数 | 目标模块 |
|---|---|---|---|
| 1–27 | imports | 27 | — |
| 28–82 | CompChatRequest（wire DTO） | 55 | `providers/compat/dto.rs` |
| 83–99 | StreamOptions / CompMessage（wire DTO） | 17 | `providers/compat/dto.rs` |
| 100–126 | `From<&Message> for CompMessage`（请求映射） | 27 | `providers/compat/map.rs` |
| 127–161 | CompResponse / CompChoice / CompUsage / CompPromptTokensDetails（wire DTO） | 35 | `providers/compat/dto.rs` |
| 162–211 | CompStreamChunk / CompStreamChoice / CompDelta / CompToolCallDelta / CompFunctionDelta（wire DTO） | 50 | `providers/compat/dto.rs` |
| 212–262 | CompErrorBody / CompErrorDetail / CompEmbeddingRequest / CompEmbeddingResponse / CompEmbedding / CompEmbeddingUsage（wire DTO） | 51 | `providers/compat/dto.rs` |
| 263–331 | ToolCallAccumulator / ToolCallBuilder（流解析状态机） | 69 | `providers/compat/stream.rs` |
| 332–397 | `parse_sse_line`（流解析） | 66 | `providers/compat/stream.rs` |
| 398–416 | `comp_usage_to_usage`（响应映射） | 19 | `providers/compat/map.rs` |
| 417–499 | OpenAiCompatibleProvider + new/with_reasoning/reasoning_effort（Provider 核心） | 83 | 留驻（入口收口） |
| 500–583 | `build_body`（请求映射） | 84 | `providers/compat/map.rs` |
| 584–615 | `parse_response`（响应映射） | 32 | `providers/compat/map.rs` |
| 616–761 | `impl Provider`（chat/stream/embed 发网） | 146 | 留驻（入口收口） |
| 762–1455 | 测试段（§1.6 测试分组） | 694 | 测试外迁卡 |

### 1.2 google.rs（GoogleProvider，1524 行）

| 行区间 | 职责 | 行数 | 目标模块 |
|---|---|---|---|
| 1–17 | imports + DEFAULT_BASE_URL | 17 | — |
| 18–44 | GoogleProvider + new（Provider 核心） | 27 | 留驻（入口收口） |
| 45–78 | GeminiRequest / GeminiThinkingConfig（wire DTO） | 34 | `providers/google/dto.rs` |
| 79–95 | thinking_to_gemini / thinking_enabled（请求映射） | 17 | `providers/google/map.rs` |
| 96–131 | GeminiContent / GeminiPart / GeminiInlineData（wire DTO） | 36 | `providers/google/dto.rs` |
| 132–143 | GeminiFunctionCallOut / GeminiFunctionResponseOut（wire DTO） | 12 | `providers/google/dto.rs` |
| 144–191 | GeminiGenerationConfig + is_empty（wire DTO） | 48 | `providers/google/dto.rs` |
| 192–221 | GeminiTool / GeminiFunctionDeclaration / GeminiToolConfig / GeminiFunctionCallingConfig（wire DTO） | 30 | `providers/google/dto.rs` |
| 222–268 | GeminiResponse / GeminiCandidate / GeminiLogprobsResult / GeminiLogprobCandidate / GeminiTopCandidatesAtPosition（wire DTO） | 47 | `providers/google/dto.rs` |
| 269–303 | gemini_logprobs_to_logprobs（响应映射） | 35 | `providers/google/map.rs` |
| 304–354 | GeminiContentResponse / GeminiPartResponse / GeminiFunctionCallResponse / GeminiUsageMetadata / GeminiErrorBody / GeminiErrorDetail（wire DTO） | 51 | `providers/google/dto.rs` |
| 355–374 | GeminiStreamEvent / GeminiStreamCandidate（wire DTO） | 20 | `providers/google/dto.rs` |
| 375–425 | GeminiToolAccumulator + impl（流解析状态机） | 51 | `providers/google/stream.rs` |
| 426–439 | map_gemini_role（请求映射） | 14 | `providers/google/map.rs` |
| 440–476 | content_to_parts（请求映射） | 37 | `providers/google/map.rs` |
| 477–495 | gemini_inline_from_url（请求映射） | 19 | `providers/google/map.rs` |
| 496–504 | tool_result_response（请求映射） | 9 | `providers/google/map.rs` |
| 505–543 | to_gemini_tools / to_gemini_tool_config（请求映射） | 39 | `providers/google/map.rs` |
| 544–564 | gemini_response_format（请求映射） | 21 | `providers/google/map.rs` |
| 565–597 | build_generation_config（请求映射） | 33 | `providers/google/map.rs` |
| 598–681 | build_contents（请求映射） | 84 | `providers/google/map.rs` |
| 682–706 | to_finish_reason（响应映射） | 25 | `providers/google/map.rs` |
| 707–808 | `parse_sse_line`（流解析） | 102 | `providers/google/stream.rs` |
| 809–828 | `build_body`（请求映射） | 20 | `providers/google/map.rs` |
| 829–1004 | `impl Provider`（chat/stream/embed 发网） | 176 | 留驻（入口收口） |
| 1005–1524 | 测试段（§1.6 测试分组） | 520 | 测试外迁卡 |

### 1.3 anthropic.rs（AnthropicProvider，1483 行）

| 行区间 | 职责 | 行数 | 目标模块 |
|---|---|---|---|
| 1–17 | imports + DEFAULT_BASE_URL | 17 | — |
| 18–44 | AnthropicProvider + new（Provider 核心） | 27 | 留驻（入口收口） |
| 45–84 | AnthropicRequest / AnthropicThinking（wire DTO） | 40 | `providers/anthropic/dto.rs` |
| 85–108 | thinking_to_anthropic / thinking_enabled（请求映射） | 24 | `providers/anthropic/map.rs` |
| 109–175 | AnthropicMessage / AnthropicMessageContent / AnthropicContentBlock / AnthropicImageSource（wire DTO） | 67 | `providers/anthropic/dto.rs` |
| 176–203 | to_anthropic_tools / to_anthropic_tool_choice（请求映射） | 28 | `providers/anthropic/map.rs` |
| 204–249 | to_anthropic_content / anthropic_image_source（请求映射） | 46 | `providers/anthropic/map.rs` |
| 250–275 | normalize_stop_reason / flush_tool_results（映射） | 26 | `providers/anthropic/map.rs` |
| 276–346 | split_messages（请求映射） | 71 | `providers/anthropic/map.rs` |
| 347–368 | `build_body`（请求映射） | 22 | `providers/anthropic/map.rs` |
| 369–449 | AnthropicResponse / AnthropicContent / AnthropicUsage / AnthropicOutputTokensDetails + usage_from_anthropic（wire DTO 为主，响应映射仅 usage_from_anthropic） | 81 | `providers/anthropic/dto.rs`（计入口径：dto.rs；usage_from_anthropic 的映射职责在 C8 实现时随 dto 数据就地梳理，不再另行搬移） |
| 450–528 | AnthropicErrorBody / AnthropicErrorDetail / AnthropicStreamEvent / AnthropicStreamMessage / AnthropicStreamError / AnthropicStreamContentBlock / AnthropicDelta（wire DTO） | 79 | `providers/anthropic/dto.rs` |
| 529–660 | AnthropicToolAccumulator / AnthropicToolBuilder + impl（流解析状态机） | 132 | `providers/anthropic/stream.rs` |
| 661–785 | `parse_sse_line`（流解析） | 125 | `providers/anthropic/stream.rs` |
| 786–951 | `impl Provider`（chat/stream/embed 发网） | 166 | 留驻（入口收口） |
| 952–1483 | 测试段（§1.6 测试分组） | 532 | 测试外迁卡 |

### 1.4 types.rs（L0 公开 API 域，1132 行）

| 行区间 | 职责 | 行数 | 目标模块 |
|---|---|---|---|
| 1–8 | imports | 8 | — |
| 9–19 | Role（enum，STABLE） | 11 | `types/domain.rs` |
| 20–60 | Tool / FunctionDef（STABLE） | 41 | `types/domain.rs` |
| 61–104 | ToolChoice / ToolChoiceFunction（STABLE） | 44 | `types/domain.rs` |
| 105–126 | ToolCall / FunctionCall（STABLE） | 22 | `types/domain.rs` |
| 127–170 | ContentPart / ImageUrl（STABLE） | 44 | `types/domain.rs` |
| 171–243 | Content + From 实现（STABLE） | 73 | `types/domain.rs` |
| 244–336 | Message + builder（STABLE） | 93 | `types/domain.rs` |
| 337–403 | Usage / LogProbs / TokenLogProb / TopLogProb（STABLE） | 67 | `types/stream.rs` |
| 404–472 | FinishReason + serde（STABLE，变体集冻结） | 69 | `types/stream.rs` |
| 473–525 | ChatResponse / StreamChunk（STABLE） | 53 | `types/stream.rs` |
| 526–592 | ResponseFormat / ThinkingConfig（STABLE） | 67 | `types/stream.rs` |
| 593–833 | ChatRequest + builder（STABLE-ADDITIVE，`#[non_exhaustive]`） | 241 | `types/request.rs` |
| 834–925 | EmbeddingRequest + builder（STABLE-ADDITIVE，`#[non_exhaustive]`）/ Embedding / EmbeddingUsage / EmbeddingResponse（STABLE） | 92 | `types/request.rs` + `types/domain.rs` |
| 926–1132 | 测试段（§1.6 测试分组） | 207 | 测试外迁卡 |

### 1.5 router.rs（Router，851 行）

| 行区间 | 职责 | 行数 | 目标模块 |
|---|---|---|---|
| 1–68 | imports + 常量 | 68 | — |
| 69–93 | RoutingStrategy（enum，STABLE） | 25 | 留驻（入口收口） |
| 94–124 | should_failover / no_deployments（策略辅助） | 31 | `router/state.rs` |
| 125–247 | Router + impl（new/with_strategy/with_cooldown/route/is_in_cooldown/mark_cooldown/clear_cooldown/candidates——路由状态与调度） | 123 | `router/state.rs` |
| 248–344 | `resolve`（round-robin 单计数器现状 + failover/cooldown 决策） | 97 | `router/state.rs` |
| 345–851 | 测试段（§1.6 测试分组） | 507 | 测试外迁卡 |

> **结构设计注记（RTR-001 边界）**：`resolve` 中 round-robin 依赖**单一 `AtomicUsize` 计数器跨 group 共享**（RTR-001 行为修复对象）。本推演仅记录其**结构影响**（per-group counter 生命周期/并发同步属于 RTR-001 卡范围），C13 卡把「per-group counter 结构就位」列为依赖 RTR-001 的后续项，**不在此设计其修复方案**。

### 1.6 测试段分组（五文件合计 2460 行）

| 文件 | 测试段 | 行数 | 目标 |
|---|---|---|---|
| compat.rs | 762–1455 | 694 | `tests/providers/compat/` |
| google.rs | 1005–1524 | 520 | `tests/providers/google/` |
| anthropic.rs | 952–1483 | 532 | `tests/providers/anthropic/` |
| types.rs | 926–1132 | 207 | `tests/types/` |
| router.rs | 345–851 | 507 | `tests/router/` |

**行数守恒核验**：compat 761 生产 + 694 测试 = 1455；google 1004 + 520 = 1524；anthropic 951 + 532 = 1483；types 925 + 207 = 1132；router 344 + 507 = 851；合计 3985 生产 + 2460 测试 = **6445** ✓（与修正后台账一致）。

## 2. 内部依赖图（现状）

```
LmrsClient (src/lib.rs)
    │  set_openai_compatible / set_google / set_anthropic（lib.rs:197/250/220）
    ▼
providers/compat.rs ── OpenAiCompatibleProvider ──► crate::types::{ChatRequest, ChatResponse, StreamChunk, ...}
providers/google.rs ── GoogleProvider ──► crate::types::{...}
providers/anthropic.rs ── AnthropicProvider ──► crate::types::{...}
        │                                    ▲
        │                                    │（L0，无 provider 依赖）
        ▼                                    │
crate::router.rs ── Router ──► LmrsClient（Arc，route 分发）──► 各 provider（dyn Provider）
        │
        └──► crate::providers::{LlmError, Provider, ProviderConfig, Result}
```

**Feature 门控**：`proxy` feature 门控 `src/proxy` 模块（lib.rs:67）；providers/router/types 为常驻模块。provider 挂载经 `lib.rs` 的 `set_*` 方法（`set_openai_compatible` → `OpenAiCompatibleProvider`，`set_google` → `GoogleProvider`，`set_anthropic` → `AnthropicProvider`）。

## 3. 目标模块树与允许边

```
src/
├── lib.rs                    （LmrsClient facade，pub use 再导出保持）
├── prelude.rs                （再导出保持）
├── types.rs                  （L0 收口：门控 + 再导出兼容）
│   ├── domain.rs             （Role/Tool/FunctionDef/ToolChoice/ToolChoiceFunction/ToolCall/FunctionCall/ContentPart/ImageUrl/Content/Message/Embedding/EmbeddingUsage/EmbeddingResponse）
│   ├── stream.rs             （Usage/LogProbs/TokenLogProb/TopLogProb/FinishReason/ChatResponse/StreamChunk/ResponseFormat/ThinkingConfig）
│   └── request.rs            （ChatRequest/EmbeddingRequest + builder）
├── providers/
│   ├── mod.rs                （LlmError/Provider/ProviderConfig/Result 契约，不变）
│   ├── compat.rs             （OpenAiCompatibleProvider 收口：struct + impl Provider + pub use 兼容）
│   │   ├── dto.rs            （全部 Compat wire DTO）
│   │   ├── map.rs            （build_body/parse_response/comp_usage_to_usage/From<&Message>）
│   │   └── stream.rs         （ToolCallAccumulator/ToolCallBuilder/parse_sse_line）
│   ├── google.rs             （GoogleProvider 收口：struct + impl Provider）
│   │   ├── dto.rs            （全部 Gemini wire DTO）
│   │   ├── map.rs            （build_contents/build_generation_config/gemini_response_format/to_*_tools/content_to_parts/...）
│   │   └── stream.rs         （GeminiToolAccumulator/parse_sse_line）
│   ├── anthropic.rs          （AnthropicProvider 收口：struct + impl Provider）
│   │   ├── dto.rs            （全部 Anthropic wire DTO）
│   │   ├── map.rs            （build_body/split_messages/to_anthropic_*/usage_from_anthropic/normalize_stop_reason）
│   │   └── stream.rs         （AnthropicToolAccumulator/AnthropicToolBuilder/parse_sse_line）
│   └── （deepseek/moonshot/ollama/openai/openrouter/retry/http/stream_state/stream_util 非热点，不动）
└── router.rs                 （Router 收口：struct + RoutingStrategy + pub use 兼容）
    └── state.rs              （should_failover/no_deployments/candidates/resolve——路由状态与调度；per-group counter 结构就位依赖 RTR-001）
```

**允许边**（每模块仅依赖）：
- `types/*` → 仅 std + serde（L0，零 provider/router/proxy 依赖）
- `providers/*/dto` → serde + `crate::types`（wire DTO 可引用 L0 域类型做转换，但 wire 类型本身不进入 types）
- `providers/*/map` → `dto` + `types`
- `providers/*/stream` → `dto` + `types` + `providers/mod`（LlmError）
- `providers/*/（入口）` → `map` + `stream` + `dto` + `types` + `providers/mod`
- `router/state` → `types` + `providers/mod`（LlmError）
- `router/（入口）` → `router/state` + `types` + `providers/mod` + `LmrsClient`

**禁止边**：任何模块 → `common`/`shared`/`utils` 万能层（不存在）；`types/*` → `providers/*`/`router`/`proxy`（L0 禁令）；`providers/*/dto` → `router`；`router/state` → `proxy`。

## 4. API 路径影响表（public API freeze 约束）

**原则**：0.1.x 冻结期只做**纯搬移 + 兼容 re-export**；所有公开符号的公开 module path 与 root/prelude 再导出路径**逐字节不变**。下表每行：当前路径 → 未来路径 → re-export 兼容方案（含 `docs/api-inventory.json` 分类）。

### 4.1 lib.rs root 再导出符号（`pub use`，必须全部保持）

| 符号 | 分类 | 当前路径 | 未来路径 | re-export 兼容方案 |
|---|---|---|---|---|
| Role | STABLE | `llmrust::types::Role`（root 再导出） | `llmrust::types::domain::Role` | `types.rs` 保留 `pub use domain::*;` → root 再导出不变 |
| Tool / FunctionDef / ToolChoice / ToolChoiceFunction / ToolCall / FunctionCall / ContentPart / ImageUrl / Content / Message | STABLE | `llmrust::types::*` | `llmrust::types::domain::*` | 同上 |
| Usage / LogProbs / TokenLogProb / TopLogProb / FinishReason / ChatResponse / StreamChunk / ResponseFormat | STABLE | `llmrust::types::*` | `llmrust::types::stream::*` | `types.rs` 保留 `pub use stream::*;` → 不变 |
| ThinkingConfig | STABLE（非 root 再导出） | `llmrust::types::ThinkingConfig` | `llmrust::types::stream::ThinkingConfig` | `types.rs` 保留 `pub use stream::ThinkingConfig;` → `llmrust::types::ThinkingConfig` 不变（D3 已裁定 root 缺口 0.2 处理，不扩） |
| ChatRequest | STABLE-ADDITIVE | `llmrust::types::ChatRequest` | `llmrust::types::request::ChatRequest` | `types.rs` 保留 `pub use request::*;` → 不变 |
| EmbeddingRequest | STABLE-ADDITIVE | `llmrust::types::EmbeddingRequest` | `llmrust::types::request::EmbeddingRequest` | 同上 |
| Embedding / EmbeddingUsage / EmbeddingResponse | STABLE | `llmrust::types::*` | `llmrust::types::domain::*` | 同上 |
| Router / RoutingStrategy | STABLE | `llmrust::router::{Router, RoutingStrategy}` | `llmrust::router::{Router, RoutingStrategy}`（state 内部化） | `router.rs` 保留 `pub use state::RoutingStrategy;` + `pub struct Router` 原地 → 不变 |
| LlmError / Provider / ProviderConfig / Result | STABLE | `llmrust::providers::*` | 不变（providers/mod.rs 契约不动） | 不变 |
| RetryProvider | STABLE | `llmrust::providers::retry::RetryProvider` | 不变 | 不变 |
| ModelPricing | STABLE | `llmrust::pricing::ModelPricing` | 不变 | 不变 |

### 4.2 prelude 再导出符号（必须全部保持）

| 符号 | 当前 prelude 路径 | 未来路径 | 兼容方案 |
|---|---|---|---|
| ChatRequest/ChatResponse/Content/ContentPart/Embedding/EmbeddingRequest/EmbeddingResponse/EmbeddingUsage/FunctionCall/FunctionDef/ImageUrl/LogProbs/Message/ResponseFormat/Role/StreamChunk/TokenLogProb/Tool/ToolCall/ToolChoice/TopLogProb/Usage | `llmrust::prelude::*` | 同上（域化后） | prelude.rs 再导出目标改为 `crate::types`（保持），`crate::types` 内部再导出域模块 → **prelude 零改动** |
| Router/RoutingStrategy | `llmrust::prelude::*` | 同上 | 同上 |
| RetryProvider/LlmError/LmrsClient/Provider/ProviderConfig/Result | `llmrust::prelude::*` | 不变 | 不变 |
| proxy（feature 门控） | `llmrust::prelude::proxy` | 不变 | 不变 |

### 4.3 Provider 结构体路径（非 root 再导出，module path 必须保持）

| 符号 | 当前路径 | 未来路径 | 兼容方案 |
|---|---|---|---|
| OpenAiCompatibleProvider | `llmrust::providers::compat::OpenAiCompatibleProvider` | `llmrust::providers::compat::OpenAiCompatibleProvider`（struct 原地，dto/map/stream 为私有子模块） | `compat.rs` 收口保留 `pub struct` + 子模块 `pub(crate) mod` → **公开路径不变** |
| GoogleProvider | `llmrust::providers::google::GoogleProvider` | 同上 | 同上 |
| AnthropicProvider | `llmrust::providers::anthropic::AnthropicProvider` | 同上 | 同上 |

**API 路径影响表覆盖声明**：上表覆盖 `src/lib.rs` root 全部 `pub use`（§4.1 19 符号 + 4 契约/门面）与 `src/prelude.rs` 全部再导出（§4.2）与三个 provider 结构体公开路径（§4.3）。`api-inventory.json` 中 STABLE/STABLE-ADDITIVE 分类逐一引用，冻结期零路径变更。

## 5. 迁移顺序（每步 ≤400 行、独立回滚点、golden fixture 前置）

### 5.0 公开 API freeze 锚点迁移策略（单列）

- `response_freeze` / `api_freeze` / `provider_contract_freeze` / `contract_tests` 为公开 API 守恒锚：**必须先整体保留在 `types.rs`/`providers/mod.rs` 原路径**（锚点文件零搬移），域化时仅在其后追加 `pub use` 兼容层，**锚点文件本身的断言/测试不得先行外迁**；
- 测试外迁（C1）**不得外迁** types.rs 与 providers/mod.rs 的 freeze 锚测试；冻结锚测试在 C10–C14 域化完成、兼容层验证通过后再按 §1.6 分组外迁；
- 每一步提交前运行 `cargo test --all-features`（含全部 freeze 锚），绿才可继续。

| 步 | 动作 | 预估 diff | 回滚点 | golden fixture 前置 |
|---|---|---|---|---|
| 1 | **测试外迁**（compat 694 / google 520 / anthropic 532 迁至 `tests/providers/<name>/`，router 507 迁至 `tests/router/`，types 207 迁至 `tests/types/`；冻结锚测试除外） | 测试迁移 | 提交前快照 | 全部现有 wire 守恒锚 + 各 provider 契约测试 |
| 2 | `compat/dto.rs` 抽离（wire DTO 208 行） | ≤210 | 步骤 1 后 | DTO serde / wire 测试 |
| 3 | `compat/map.rs` + `compat/stream.rs` 抽离 + compat 入口收口（map 162 + stream 135 = **297 行** + 收口） | ≤300 | 步骤 2 后 | 请求转换 / SSE 累积测试 |
| 4 | `google/dto.rs` 抽离（wire DTO 278 行） | ≤280 | 步骤 3 后 | Gemini DTO serde 测试 |
| 5 | `google/map.rs` 抽离（请求/响应映射 **353 行**） | ≤360 | 步骤 4 后 | 请求构造 / logprobs / finish_reason 映射测试 |
| 6 | `google/stream.rs` 抽离 + google 入口收口（153 行 + 收口） | ≤160 | 步骤 5 后 | SSE 状态机（thinking/tool 重组）测试 |
| 7 | `anthropic/dto.rs` 抽离（wire DTO 267 行） | ≤270 | 步骤 6 后 | Anthropic DTO serde 测试 |
| 8 | `anthropic/map.rs` 抽离（请求/响应映射 217 行） | ≤220 | 步骤 7 后 | 请求构造 / thinking / usage 映射测试 |
| 9 | `anthropic/stream.rs` 抽离 + anthropic 入口收口（257 行 + 收口） | ≤260 | 步骤 8 后 | SSE 状态机（thinking 块/工具重组/截流）测试 |
| 10 | `types/domain.rs` 抽离（L0 域 328 行 + 兼容 re-export） | ≤340 | 步骤 9 后 | api_freeze / response_freeze 全量锚 |
| 11 | `types/stream.rs` 抽离（流域 256 行 + 兼容 re-export） | ≤260 | 步骤 10 后 | api_freeze / response_freeze / provider_contract_freeze 全量锚 |
| 12 | `types/request.rs` 抽离（请求域 333 行 + 兼容 re-export）+ types 收口 | ≤340 | 步骤 11 后 | api_freeze / contract_tests 全量锚 |
| 13 | `router/state.rs` 抽离（路由状态 31+123+97 = **251 行**）+ router 收口；per-group counter 结构就位列为依赖 RTR-001 的后续项（不在本卡实施行为修复） | ≤260（基数 251） | 步骤 12 后 | router 契约测试（现状单计数器语义守恒） |
| 14 | 全局收口核验：lib.rs/prelude 再导出逐字节比对 api-inventory / freeze 锚；`resolve` 计数器现状语义守恒（RTR-001 修复前） | ≤50 | 步骤 13 后 | 全量 wire 守恒锚 + freeze 锚 |

**步骤↔卡 1:1 与模块归属自检**（ARC-001 MUST-2 同类）：compat wire DTO→C2；compat map/stream/入口→C3；google wire DTO→C4；google map→C5；google stream/入口→C6；anthropic wire DTO→C7；anthropic map→C8；anthropic stream/入口→C9；types domain→C10；types stream→C11；types request/入口→C12；router state/入口→C13；公开面收口→C14。**每个生产模块恰好归属一张卡**。目标模块三处一致（§1 映射 / §3 模块树 / §5 步骤）——ARC-001 MUST-1 同类自检通过。

## 6. 未来任务卡清单（14 张，≥5；每热点至少一张，含测试外迁方向）

| 卡 | 范围 | DoD | 依赖 | 守恒锚 | API 风险 |
|---|---|---|---|---|---|
| **C1 测试外迁** | compat/google/anthropic/router/types 测试迁至 `tests/<对应>/`（按主题分文件）；热点基线口径改计**生产行数**；冻结锚测试不迁 | 生产文件行数降为纯生产；architecture_guard 改读生产行数；全部测试迁移后全绿 | 无（先行） | 全部现有测试 + freeze 锚 | 无 |
| **C2 compat-DTO** | `compat/dto.rs` 抽离（28–262 区间 wire DTO，208 行） | compat dto 段归零；serde/wire 测试全绿 | C1 | DTO serde / wire 测试 | 低（`pub(crate)` 子模块，公开路径不变） |
| **C3 compat-转换流** | `compat/map.rs` + `compat/stream.rs` 抽离 + compat 入口收口 | 段归零；转换/SSE 测试全绿 | C2 | 请求转换 / SSE 累积测试 | 低 |
| **C4 google-DTO** | `google/dto.rs` 抽离（wire DTO 278 行） | google dto 段归零；serde 测试全绿 | C1 | Gemini DTO serde 测试 | 低 |
| **C5 google-映射** | `google/map.rs` 抽离（映射 **353 行**） | map 段归零；请求构造/logprobs/finish_reason 测试全绿 | C4 | 请求映射测试 | 低 |
| **C6 google-流** | `google/stream.rs` 抽离 + google 入口收口 | stream 段归零；SSE 状态机测试全绿 | C5 | SSE 状态机测试 | 低 |
| **C7 anthropic-DTO** | `anthropic/dto.rs` 抽离（wire DTO 267 行） | anthropic dto 段归零；serde 测试全绿 | C1 | Anthropic DTO serde 测试 | 低 |
| **C8 anthropic-映射** | `anthropic/map.rs` 抽离（映射 217 行） | map 段归零；thinking/usage 映射测试全绿 | C7 | 请求映射测试 | 低 |
| **C9 anthropic-流** | `anthropic/stream.rs` 抽离 + anthropic 入口收口 | stream 段归零；SSE 状态机（thinking/工具重组/截流）测试全绿 | C8 | SSE 状态机测试 | 低 |
| **C10 types-domain** | `types/domain.rs` 抽离（域类型 328 行）+ `types.rs` `pub use` 兼容层 | domain 段归零；api_freeze/response_freeze 全绿；root/prelude 再导出逐字节不变 | C1 | api_freeze / response_freeze | **高**（L0 公开 API 面） |
| **C11 types-stream** | `types/stream.rs` 抽离（流域 256 行）+ 兼容层 | stream 段归零；三个 freeze 锚全绿；再导出不变 | C10 | api_freeze / response_freeze / provider_contract_freeze | **高** |
| **C12 types-request** | `types/request.rs` 抽离（请求域 333 行）+ types 收口 | request 段归零；freeze 锚 + contract_tests 全绿；再导出不变 | C11 | api_freeze / contract_tests | **高** |
| **C13 router-state** | `router/state.rs` 抽离（路由状态 **251 行**）+ router 收口；**per-group counter 结构就位列为依赖 RTR-001 的后续项**（本卡不实施行为修复，保持单计数器现状语义） | state 段归零；router 契约测试全绿（现状语义守恒） | C1（+RTR-001 行为修复为后续独立项） | router 契约测试 | 中（RoutingStrategy 公开 enum 原地） |
| **C14 公开面收口** | lib.rs/prelude 再导出逐字节比对 api-inventory / freeze 锚；域化后兼容层终验 | 再导出零漂移；freeze 锚全绿 | C10–C13 | api-inventory / 全部 freeze 锚 | **高**（收口核验） |

全部 14 卡 0.1.4+ 候选，是否实施由 Owner 后续选择。

## 7. 风险清单

| 风险 | 等级 | 缓解 |
|---|---|---|
| types.rs 域化破坏 L0 公开 API（root/prelude 再导出、STABLE 形状） | 高 | freeze 锚先行（§5.0：锚点文件零搬移 + 兼容层后置验证）；API 路径影响表逐符号比对 |
| provider 拆分改变公开 module path（`providers::compat::OpenAiCompatibleProvider` 等） | 中 | §4.3：struct 原地收口 + 子模块 `pub(crate)`，公开路径不变 |
| 流解析状态机（ToolCall/GeminiTool/AnthropicTool Accumulator）抽离破坏 thinking/工具重组/截流语义 | 高 | golden fixture 前置（各 SSE 状态机测试先整体迁移）；每步独立回滚 |
| Router 单计数器跨组干扰：结构抽离误夹带行为修复或误改现状语义 | 中 | C13 边界：per-group counter 结构就位依赖 RTR-001，本卡保持单计数器现状；router 契约测试守恒 |
| wire DTO 被复制进 types（§4.3 禁止边） | 中 | 禁止边写入每张卡 DoD；评审逐字节核验 |
| 步骤超过 400 行 / 步骤与卡非 1:1 / 目标模块三处不一致（ARC-001 首轮 2 MUST 同类） | 低 | §5 自检表：14 步↔14 卡严格 1:1；§1/§3/§5 三处一致 |
| 测试外迁后热点基线口径变更（生产行数）暴露新热点 | 中 | 口径改计生产行数后重跑 architecture_guard |

## 8. 结论

五个热点文件（compat.rs 1455、google.rs 1524、anthropic.rs 1483、types.rs 1132、router.rs 851，合计 6445 行）可通过上述 14 步迁移收敛为按职责细分的子模块树（每个 provider 的 `dto/map/stream` + types 的 `domain/stream/request` + router 的 `state`），**无万能层、每模块单一职责、wire DTO 不进入 types、L0 公开 API 路径逐字节兼容**。测试外迁为首步（缓解热点 + 为后续生产拆分铺路）。未来任务卡 14 张（每热点至少一张，含测试外迁方向），步骤与卡严格 1:1、每个生产模块恰好归属一张卡，全部 0.1.4+ 候选，是否实施由 Owner 后续选择。RTR-001 行为修复（Router per-group counter）单列，不混入本推演。
