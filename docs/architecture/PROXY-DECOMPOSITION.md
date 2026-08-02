# Proxy 热点拆分推演（ARC-001）

> **状态**：推演文档（M4 开门卡，纯设计，零生产代码改动）
> **日期**：2026-08-03
> **依据**：SPCC §11.7 ARC-001 卡（初始状态/目标/允许范围/禁止范围/执行步骤/DoD/回证）
> **目标文件**：`src/proxy/mod.rs`（3327 行）、`src/proxy/anthropic_proxy.rs`（1884 行）
> **产出**：行区间映射、依赖图、目标模块树、迁移顺序、未来任务卡清单（≥5，含测试外迁卡）、风险清单

## 0. 背景与约束

- M3 封板（PRX-001..005 + STATE 关账 5 笔），热点膨胀为结构性根因：**测试与生产同文件**（见 M3 相位复盘 §16）。
- 本卡**只产出推演文档**，`src/**`、`tests/**`、`tests/hotspot_ledger.json` 逐字节零 diff。
- 目标模块树**禁止** `common`/`shared`/`utils` 万能层；每模块单一职责（SPCC §4.4）。
- 迁移顺序：每步 ≤400 行人工 diff、独立回滚点、golden fixture 前置说明。
- 未来任务卡是否实施归 Owner 后续选择（0.1.4+ 候选），本卡不实施。

## 1. 现状行区间映射（mod.rs，3327 行）

| 行区间 | 职责 | 行数 | 目标模块 |
|---|---|---|---|
| 1–46 | imports | 46 | — |
| 47–56 | `mod anthropic_proxy` 声明 | 10 | — |
| 57–76 | `generate_id` / `unix_timestamp`（通用辅助） | 20 | `proxy::util` |
| 77–368 | OpenAI wire DTO（ProxyChatRequest / ProxyStreamOptions / ProxyFunctionCallChoice / ProxyStop / ProxyMessage / ProxyChatResponse / ProxyChoice / ProxyResponseMessage / ProxyUsage / ProxyStreamChunk / ProxyStreamChoice / ProxyDelta / ProxyEmbeddingRequest / ProxyEmbeddingInput / ProxyEmbeddingResponse / ProxyEmbedding / ProxyEmbeddingUsage / ProxyError / ProxyErrorDetail） | 292 | `proxy::openai::dto` |
| 369–408 | AppState + 相关 | 40 | `proxy::app` |
| 409–470 | `router` / `router_with_auth`（路由装配） | 62 | `proxy::router` |
| 471–482 | `map_body_limit_response`（413 协议形状中间件） | 12 | `proxy::router` |
| 483–515 | `check_bearer`（认证中间件） | 33 | `proxy::auth` |
| 516–523 | `subtle_constant_time_eq` | 8 | `proxy::auth` |
| 524–532 | `health_check` | 9 | `proxy::handlers` |
| 533–576 | `shutdown_signal` | 44 | `proxy::server` |
| 577–590 | `is_loopback_addr` | 14 | `proxy::config` |
| 591–641 | `serve`（绑定/启动/认证决策） | 51 | `proxy::server` |
| 642–710 | `handle_chat_completions`（OpenAI 入口：body limit → reasoning 边界拒绝 → 分发） | 69 | `proxy::handlers` |
| 711–759 | `handle_non_stream`（OpenAI 非流） | 49 | `proxy::handlers` |
| 760–841 | `handle_embeddings`（OpenAI embeddings 入口） | 82 | `proxy::handlers` |
| 842–1005 | `build_openai_sse_response`（OpenAI SSE 唯一终止 + reasoning 增量守卫） | 164 | `proxy::openai::sse` |
| 1006–1045 | `handle_stream`（OpenAI 流分发） | 40 | `proxy::handlers` |
| 1046–1127 | `convert_request`（OpenAI 请求转换） | 82 | `proxy::openai::convert` |
| 1128–1138 | `split_model`（provider/model 解析） | 11 | `proxy::util` |
| 1139–1175 | `proxy_error_from_llm_error`（错误归一） | 37 | `proxy::error` |
| 1176–1184 | `api_error_type` | 9 | `proxy::error` |
| 1185–1196 | `json_rejection_response` | 12 | `proxy::error` |
| 1197–1203 | `invalid_json_response` | 7 | `proxy::error` |
| 1204–1213 | `proxy_max_body_bytes`（env 读取） | 10 | `proxy::config` |
| 1214–1228 | `body_too_large_response` | 15 | `proxy::error` |
| 1229–1232 | `error_response` | 4 | `proxy::error` |
| 1233–1245 | `error_response_with_type` | 13 | `proxy::error` |
| 1246–3327 | 测试段（见 §1.2 测试分组） | 2082 | 测试外迁卡 |

### 1.1 mod.rs 测试段分组

| 行区间 | 测试主题 | 目标 |
|---|---|---|
| 1246–1391 | 测试模块头部 + 辅助（build_request 等） | `proxy/tests` 测试基建 |
| 1392–1819 | 请求转换 / 响应构造 / 工具转换测试 | `proxy/tests/openai` |
| 1820–1923 | Auth middleware tests | `proxy/tests/auth` |
| 1924–2224 | serve() integration tests | `proxy/tests/server` |
| 2225–2372 | OpenAI proxy stream error tests | `proxy/tests/stream` |
| 2373–2702 | proxy embeddings tests | `proxy/tests/embeddings` |
| 2703–2802 | PRX-001 CORS / listen-address | `proxy/tests/security` |
| 2803–2995 | PRX-002 auth / constant-time / health | `proxy/tests/security` |
| 2996–3162 | PRX-003 reasoning rejection + delta guard | `proxy/tests/security` |
| 3163–3327 | PRX-005 body limit + error normalization | `proxy/tests/security` |

## 2. 现状行区间映射（anthropic_proxy.rs，1884 行）

| 行区间 | 职责 | 行数 | 目标模块 |
|---|---|---|---|
| 1–28 | imports | 28 | — |
| 29–202 | Anthropic wire DTO（AnthropicRequest / AnthropicSystemContent / AnthropicSystemBlock / AnthropicMessage / AnthropicMessageContent / AnthropicContentBlock / AnthropicToolResultContent / AnthropicToolResultBlock / AnthropicImageSource / AnthropicToolDef / AnthropicToolChoiceDef / AnthropicResponse / AnthropicResponseBlock / AnthropicUsageOut / AnthropicErrorBody / AnthropicErrorDetail） | 174 | `proxy::anthropic::dto` |
| 203–373 | `convert_request`（Anthropic 请求转换） | 171 | `proxy::anthropic::convert` |
| 374–414 | `build_response`（Anthropic 非流响应） | 41 | `proxy::anthropic::convert` |
| 415–426 | `normalize_stop_reason` | 12 | `proxy::anthropic::convert` |
| 427–820 | `AnthropicStreamState`（SSE 状态机：thinking 块 / 工具片段重组 / 截流错误化） | 394 | `proxy::anthropic::sse` |
| 821–899 | `build_stream_response` | 79 | `proxy::anthropic::sse` |
| 900–952 | `handle_messages`（Anthropic 入口：body limit → thinking 拒绝 → 分发） | 53 | `proxy::handlers` |
| 953–966 | `handle_non_stream` | 14 | `proxy::handlers` |
| 967–991 | `handle_stream` | 25 | `proxy::handlers` |
| 992–1004 | `anthropic_error` | 13 | `proxy::error` |
| 1005–1008 | `invalid_json_body_response` | 4 | `proxy::error` |
| 1009–1044 | `anthropic_error_from_llm_error` | 36 | `proxy::error` |
| 1045–1884 | 测试段（见 §2.1） | 840 | 测试外迁卡 |

### 2.1 anthropic_proxy.rs 测试段分组

| 行区间 | 测试主题 | 目标 |
|---|---|---|
| 1045–1442 | 请求转换 / 非流响应 / SSE 基础测试 | `proxy/tests/anthropic` |
| 1443–1682 | block index tests | `proxy/tests/anthropic` |
| 1683–1884 | PRX-004 thinking mapping + tool reassembly + truncation | `proxy/tests/security` |

## 3. 内部依赖图（现状）

```
LmrsClient (src/lib.rs)
    │ 注入
    ▼
AppState (proxy::app) ──► Arc<LmrsClient>
    │
    ├── router / router_with_auth ──► map_body_limit_response
    │                                   │
    │                                   ▼
    │                              check_bearer (auth)
    │                                   │
    │                                   ▼
    ├── handle_chat_completions ──► convert_request ──► ChatRequest (types)
    │        │                        │
    │        │                        └──► split_model / generate_id / unix_timestamp (util)
    │        ▼
    │   handle_non_stream / handle_stream ──► build_openai_sse_response (SSE)
    │        │
    │        ▼
    │   proxy_error_from_llm_error / api_error_type (error)
    │
    ├── handle_embeddings ──► ProxyEmbeddingRequest / ProxyEmbeddingResponse (dto)
    │
    └── anthropic_proxy::handle_messages
              │
              ├── convert_request ──► ChatRequest (types)
              ├── build_stream_response ──► AnthropicStreamState (SSE)
              └── anthropic_error_from_llm_error (error)
```

**Feature 门控**：整个 `proxy` 模块由 `features = ["proxy"]` 门控（Cargo.toml）；`anthropic_proxy` 是 `mod.rs` 的子模块。

## 4. 目标模块树与允许边

```
src/proxy/
├── mod.rs               （门控入口 + 精简重导出）
├── app.rs               （AppState）
├── router.rs            （router / router_with_auth / map_body_limit_response 413 中间件）
├── auth.rs              （check_bearer / subtle_constant_time_eq）
├── config.rs            （is_loopback_addr / proxy_max_body_bytes / env 读取）
├── server.rs            （serve / shutdown_signal）
├── util.rs              （generate_id / unix_timestamp / split_model）
├── error.rs             （proxy_error_from_llm_error / anthropic_error* / error_response* / api_error_type / body_too_large_response / invalid_json*）
├── handlers.rs          （handle_chat_completions / handle_non_stream / handle_stream / handle_embeddings / health_check / handle_messages 分发）
├── openai/
│   ├── dto.rs           （全部 OpenAI wire DTO）
│   ├── convert.rs       （convert_request）
│   └── sse.rs           （build_openai_sse_response）
└── anthropic/
    ├── dto.rs           （全部 Anthropic wire DTO）
    ├── convert.rs       （convert_request / build_response / normalize_stop_reason）
    └── sse.rs           （AnthropicStreamState / build_stream_response）
```

**允许边**（每模块仅依赖）：
- `dto` → `types`（无 proxy 内部依赖）
- `convert` → `dto` + `types` + `util`
- `sse` → `dto` + `types` + `error`
- `handlers` → `dto` + `convert` + `sse` + `error` + `auth` + `config`
- `router` → `handlers` + `auth` + `config`（map_body_limit_response 413 中间件与路由装配同职责，随 router 走）
- `server` → `router` + `config` + `auth`
- `app` → `types`
- `error` → `types` + `util`
- `config` → 仅 std/env

**禁止边**：任何模块 → `common`/`shared`/`utils` 万能层（不存在）；`dto` → `handlers`；`sse` → `handlers`；`convert` → `sse`。

**`util.rs` 单一职责声明（SHOULD-1 处置）**：`util.rs` 仅收 `generate_id` / `unix_timestamp` / `split_model` 三个无状态纯辅助函数，**禁止再向 util 追加任何新职责**。消费者 ≥2：OpenAI 路径（`handle_chat_completions` / `build_openai_sse_response`）与 Anthropic 路径（`handle_messages` / `build_stream_response`）均复用 `generate_id` / `unix_timestamp` 装配请求与 SSE 元数据；`split_model` 供两协议入口 handler 做 provider/model 分发决策（依据 §3 依赖图）。若未来增长，按职责拆分为 `ids.rs`（id/time）与 `model.rs`（split_model）——实施时以引用核验为准。

## 5. 迁移顺序（每步 ≤400 行、独立回滚点、golden fixture 前置）

| 步 | 动作 | 预估 diff | 回滚点 | golden fixture 前置 |
|---|---|---|---|---|
| 1 | **测试外迁**（第一步优先，缓解热点） | 测试迁移 | 提交前快照 | PRX-001..005 全部 wire 守恒锚（现有测试） |
| 2 | `util` + `config` 抽离（无依赖辅助） | ≤200 | 步骤 1 后 | 无（纯函数） |
| 3 | `error` 抽离（错误归一，含两协议形状） | ≤300 | 步骤 2 后 | 413/502 形状矩阵测试 |
| 4 | `openai/dto` + `anthropic/dto` 抽离 | ≤350 | 步骤 3 后 | DTO serde 测试 |
| 5 | `openai/convert` + `anthropic/convert` 抽离 | ≤350 | 步骤 4 后 | 请求转换测试 |
| 6 | `openai/sse` + `anthropic/sse` 抽离 | ≤400 | 步骤 5 后 | SSE golden 序列测试 |
| 7 | `auth` + `router` + `server` + `app`（AppState）抽离 | ≤250 | 步骤 6 后 | serve/auth/health 集成测试 |
| 8 | `handlers` 抽离（两协议入口 + 健康检查，341 行） | ≤341 | 步骤 7 后 | 全量 wire 守恒锚 |
| 9 | `mod.rs` 收口为精简入口 | ≤100 | 步骤 8 后 | 全量测试 |

## 6. 未来任务卡清单（≥5，含测试外迁卡）

| 卡 | 范围 | DoD | 依赖 | 守恒锚 |
|---|---|---|---|---|
| **T-外迁（测试外迁卡）** | 将 mod.rs 2082 行测试 + anthropic_proxy.rs 840 行测试迁至 `tests/proxy/` 子模块（按主题分文件）；热点基线口径改计**生产行数** | 生产文件行数降为纯生产；architecture_guard 改读生产行数；全部测试迁移后全绿 | 无（先行） | 全部现有测试 |
| **T-辅助拆分** | `util.rs`（generate_id / unix_timestamp / split_model）+ `config.rs`（is_loopback_addr / proxy_max_body_bytes / env 读取）抽离 | 两文件段归零；纯函数测试全绿 | 无（无依赖） | 无（纯函数） |
| **T-错误归一** | `error.rs` 抽离（两协议错误形状统一） | error 段归零；413/502 形状矩阵全绿 | T-DTO | 错误形状矩阵测试 |
| **T-DTO 拆分** | `openai/dto.rs` + `anthropic/dto.rs` 从两文件抽离 | 两文件 dto 段归零；serde 测试全绿；wire 守恒 | 无 | DTO serde / wire 测试 |
| **T-转换拆分** | `openai/convert.rs` + `anthropic/convert.rs` 抽离 | convert 段归零；转换测试全绿 | T-DTO | 请求转换测试 |
| **T-SSE 拆分** | `openai/sse.rs` + `anthropic/sse.rs`（含 AnthropicStreamState 394 行）抽离 | SSE 段归零；SSE golden 全绿 | T-DTO + T-转换 | SSE golden 序列测试 |
| **T-路由/服务器/认证** | `router.rs` + `server.rs` + `auth.rs` + `app.rs`（AppState）抽离 | 四文件段归零；serve/auth 集成测试全绿 | T-SSE + T-错误 | serve/auth/health 集成测试 |
| **T-处理器拆分** | `handlers.rs` 抽离（两协议入口 handle_chat_completions / handle_messages / handle_embeddings + handle_stream* + health_check） | handlers 段归零；入口分发测试全绿 | T-转换 + T-SSE + T-错误 + T-辅助 | 全量 wire 守恒锚 |
| **T-入口收口** | `mod.rs` 收口为门控入口 + 重导出 | 两文件仅剩入口；全量测试绿；热点台账降至最小 | 全部 | 全量 wire 守恒锚 |

## 7. 风险清单

| 风险 | 等级 | 缓解 |
|---|---|---|
| SSE 状态机（AnthropicStreamState 394 行）抽离时破坏 thinking 块/工具重组/截流语义 | 高 | golden fixture 前置（PRX-004 测试先整体迁移）；每步独立回滚 |
| 错误归一抽离时两协议形状不对称回归（P2-4 已修） | 中 | 413/502 形状矩阵测试锁定 |
| 测试外迁后热点基线口径变更（生产行数）可能暴露新热点 | 中 | 口径改计生产行数后重跑 architecture_guard |
| 公开 path/re-export 变更影响下游（`proxy::router` 等 pub API） | 中 | 迁移步骤保留 pub re-export 兼容（§11.7 ARC-002 同类风险） |
| 迁移顺序中某步超过 400 行 | 低 | 每步拆小、独立提交、回滚点明确 |

## 8. 结论

两热点文件（mod.rs 3327 行、anthropic_proxy.rs 1884 行）可通过上述 9 步迁移收敛为精简入口 + 按职责细分的子模块树（openai/、anthropic/、auth、app、config、router、server、util、error、handlers），**无万能层、每模块单一职责**。测试外迁为首步（缓解热点 + 为后续生产拆分铺路）。未来任务卡 9 张（含测试外迁卡），步骤与卡严格 1:1、每个生产模块恰好归属一张卡，全部 0.1.4+ 候选，是否实施由 Owner 后续选择。
