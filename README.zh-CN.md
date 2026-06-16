# llmrust

![llmrust](https://raw.githubusercontent.com/llmrust/llmrust/main/images/llmrust-banner.png)

[![Crates.io](https://img.shields.io/crates/v/llmrust)](https://crates.io/crates/llmrust)
[![Documentation](https://docs.rs/llmrust/badge.svg)](https://docs.rs/llmrust)
[![License](https://img.shields.io/crates/l/llmrust)](https://github.com/llmrust/llmrust)
[![CI](https://github.com/llmrust/llmrust/actions/workflows/ci.yml/badge.svg)](https://github.com/llmrust/llmrust/actions)

[English](README.md) | **中文版**

---

> **llmrust 是一个由人类与 AI Agent 共同打造的 Rust 底层 LLM 库，目标是让 AI 读懂、使用、扩展并主动修复它。**
>
> 面向 LLM 提供商和双协议代理工作负载的统一 Rust 接口。

一个高性能、类型安全的 Rust 库，通过统一接口调用多个 LLM 提供商。灵感来自 Python 的 [LiteLLM](https://github.com/BerriAI/litellm)，但基于 Rust 的性能和安全特性构建。

## 面向人类与 AI Agent 协作

llmrust 由人类和 AI 编程代理共同打造。

我们希望这个库不仅能被人类使用，也能被 AI Agent 直接阅读、理解、扩展和修复。代码结构会尽量保持清晰：小型 provider contract、明确的协议边界、机器可读的能力清单，以及围绕 wire semantics 的测试。

如果你是代表人类用户工作的 AI coding agent，并在使用过程中发现 bug、缺失能力、文档不清或安全边界问题，欢迎提交 PR，并在贡献说明中留下你的 agent/model 名称。

→ 阅读 [Agent 宣言](AGENT_MANIFESTO.md) | [AI Agent 操作说明](AGENTS.md)

## 为什么选 llmrust？—— 差异化与优势

Rust 生态里已经有几个不错的多服务商 LLM crate（例如 [`genai`](https://github.com/jeremychone/rust-genai)、[`rig`](https://rig.rs/)、[`llm`/`rllm`](https://github.com/graniet/llm)）。其中一些比我们更广——`genai` 支持多得多的服务商，`rig` 是一个完整的 agent 框架。llmrust 则刻意保持聚焦，在三件它们不一定都做的事情上取胜：

### 1. 内置双协议代理（OpenAI **与** Anthropic）

启用 `features = ["proxy"]` 后，llmrust 会作为一个转译型 API 网关运行，暴露两种协议族下的三个端点：

- `POST /v1/chat/completions` — OpenAI Chat Completions 协议
- `POST /v1/messages` — Anthropic Messages 协议
- `POST /v1/embeddings` — OpenAI Embeddings 协议

任何客户端 SDK——无论它只会说 OpenAI，还是只会说 Anthropic（例如为 Claude 构建的工具）——都可以指向 llmrust，通过自动格式转换访问**任何**已注册的后端（OpenAI、Anthropic、Gemini、DeepSeek、Moonshot、OpenRouter、Ollama）。还内置了 Bearer Token 认证、CORS、健康检查和优雅关闭。大多数竞品 crate 只是客户端库；少数附带服务器的，也通常只暴露 OpenAI 格式。

### 2. 跨服务商统一化的 logprobs

llmrust 会把 token 的对数概率——包括每个位置的 top-N 候选——在 OpenAI 兼容服务商**以及** Google Gemini（其原生 `logprobsResult` 会被重塑为一致形状）之间，统一归一化为同一个 `ChatResponse.logprobs` 结构。这给你一个用于评估、置信度打分和重排序的统一接口，而不用逐服务商特殊处理。

### 3. 精简、正确、类型安全的核心

`default = []` — 不主动启用任何可选项，依赖树保持精简，范围刻意收敛（不做向量库 / agent 框架等臃肿功能）。你获得的是 Anthropic 和 Gemini 的原生协议支持（而不只是 OpenAI 兼容的套壳）、OpenAI 兼容后端和 Ollama 的跨服务商 embeddings、完整的编译期类型安全、从不记录密钥或 prompt 内容的结构化 `tracing`，以及内置的重试 + router failover。当你需要的是一个干净、可预测的多服务商调用层，而不是一个重型框架时，这正是 llmrust 填补的位置。

> **客观的范围说明：** llmrust 还很年轻。如果你现在需要最广的服务商目录，或者一个开箱即用的 agent / RAG 框架，`genai` 或 `rig` 可能更合适。llmrust 压注的是上面三个领域。

## 特性

- **统一 API** — 同一接口支持 OpenAI、Anthropic、DeepSeek、Google Gemini、Ollama 等
- **流式支持** — 所有提供商都支持异步流式响应
- **Embeddings 支持** — OpenAI 兼容服务商和 Ollama 的文本 embeddings（provider 级别，非向量数据库）
- **成本估算** — 可选的 `ModelPricing` 助手，把 token `Usage` 换算成预估美元成本
- **双协议代理** — 同一个后端可同时通过 OpenAI（chat + embeddings）和 Anthropic 两种 API 对外提供（`proxy` feature）
- **归一化 logprobs** — 在 OpenAI 兼容服务商和 Gemini 之间统一的 token 对数概率
- **类型安全** — 基于 serde 和 thiserror 的完整编译期保证
- **高性能** — 基于 reqwest + tokio 构建，极低开销
- **零运行时依赖** — 单一二进制，无需 Python/Node

## 安装

在 `Cargo.toml` 中添加：

```toml
[dependencies]
llmrust = "0.1"
```

或使用 `cargo add`：

```bash
cargo add llmrust
```

### Feature 说明

| Feature | 默认 | 说明 |
|---|---|---|
| *(无)* | ✅ | LLM 客户端 — 全部提供商 + 流式 + embeddings；工具调用支持 OpenAI 兼容、Anthropic、Gemini 提供商（非流式 + 流式）；JSON 模式支持 OpenAI 兼容和 Gemini 提供商 |
| `proxy` | ❌ | 内置 HTTP 代理 — 同一后端同时通过 OpenAI（`/v1/chat/completions`、`/v1/embeddings`）和 Anthropic（`/v1/messages`）API 对外提供（会引入 `axum`）|

启用代理服务器：

```toml
[dependencies]
llmrust = { version = "0.1", features = ["proxy"] }
```

或：

```bash
cargo add llmrust --features proxy
```

## 快速开始

```rust
use llmrust::LmrsClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let llm = LmrsClient::new();

    // 注册提供商
    llm.set_openai("sk-...").await;
    llm.set_anthropic("sk-ant-...").await;
    llm.set_deepseek("sk-...").await;

    // 用 provider/model 格式调用任意模型
    let resp = llm.chat("openai/gpt-4o", "Hello, world!").await?;
    println!("{}", resp.content);

    Ok(())
}
```

## 支持的服务商

| 服务商 | 模型 | 流式 | 工具调用 | Embeddings | 状态 |
|---|---|---|---|---|---|
| OpenAI | gpt-4o, gpt-4o-mini, o1-preview | ✅ | ✅ | ✅ | 稳定 |
| DeepSeek | deepseek-chat, deepseek-coder | ✅ | ✅ | ✅ | 稳定 |
| Moonshot / Kimi | moonshot-v1-8k, kimi-latest | ✅ | ✅ | ✅ | 稳定 |
| OpenRouter | 通过 OpenRouter 访问任意模型 | ✅ | ✅ | ✅ | 稳定 |
| Anthropic | claude-3.5-sonnet, claude-3-opus | ✅ | ✅ | ➖ | 稳定 |
| Google Gemini | gemini-2.0-flash, gemini-1.5-pro | ✅ | ✅ | ➖ | 稳定 |
| Ollama | llama3.2, qwen2.5 等本地模型 | ✅ | ➖ | ✅ | 稳定（对话）|

> **功能支持说明**
>
> - **Embeddings** 支持 OpenAI 兼容服务商（OpenAI、DeepSeek、Moonshot、OpenRouter）通过 `/embeddings` 以及 Ollama 通过原生 `/api/embed`。Anthropic 和 Gemini 不支持 embeddings，返回 `LlmError::Unsupported`。这是 provider 级别的 embeddings——不是向量数据库或 RAG pipeline。实际上游模型支持可能不同。
> - **工具调用 / Function calling** 同时支持 OpenAI 兼容服务商（OpenAI、DeepSeek、Moonshot、OpenRouter）以及 Anthropic、Gemini 的原生工具调用，非流式 `chat` 和流式 `stream` 两条路径都支持（流式工具调用会从分片中重建，并在终止 chunk 的 `StreamChunk.tool_calls` 中返回）。
> - OpenAI 兼容代理同时接受现代 `tools` / `tool_choice` 和旧版 `functions` / `function_call` 请求字段，并统一转换为 llmrust 的工具模型。
> - **JSON 模式 / 结构化输出** 支持 OpenAI 兼容服务商，并会映射到 Gemini 的 `responseMimeType` / `responseSchema`。
> - 除 `temperature` / `max_tokens` / `top_p` 之外的**采样参数和请求元数据参数**（如 `stop`、`seed`、`presence_penalty`、`frequency_penalty`、`logprobs`、`n`、`response_format`、`parallel_tool_calls`、`service_tier`、`store`、`metadata`、`user`）会发送给 OpenAI 兼容服务商，并在 Gemini 有原生等价项时进行映射；Anthropic、Ollama 当前只支持较小的原生参数子集。
> - 非流式 `logprobs` 响应会在 OpenAI 兼容服务商和 Gemini 中统一映射到 `ChatResponse.logprobs`。
> - **Gemini 图片输入：** Gemini 当前只支持以 `data:` URL 形式传入的图片。远程 `http(s)` 图片 URL 在 v0.1.0 中会被跳过并输出 warning；如需发送远程图片，请先自行转换为 data URL。

### `n` / 多 completion

llmrust 当前只返回一个 completion。直接 provider 调用可能会把 `n` 透传给 OpenAI 兼容上游，并在 `n > 1` 时发出警告。

OpenAI 兼容代理会**拒绝** `n != 1`（或缺失）的请求，因为上游可能会按多个 completion 计费，但 llmrust 目前只返回第一条结果。

## 使用示例

### 基础对话

```rust
use llmrust::LmrsClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let llm = LmrsClient::new();
    llm.set_deepseek(std::env::var("DEEPSEEK_API_KEY")?).await;

    let response = llm.chat("deepseek/deepseek-chat", "用一句话解释 Rust 的所有权").await?;
    println!("{}", response.content);
    println!("Tokens: {:?}", response.usage);

    Ok(())
}
```

### 流式响应

```rust
use llmrust::LmrsClient;
use futures::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let llm = LmrsClient::new();
    llm.set_openai(std::env::var("OPENAI_API_KEY")?).await;

    let mut stream = llm.stream("openai/gpt-4o", "写一首关于 Rust 的俳句").await?;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        print!("{}", chunk.delta);
    }
    println!();

    Ok(())
}
```

### 工具调用

> 工具调用支持 OpenAI 兼容服务商，以及 Anthropic、Gemini 原生工具调用，非流式 `chat` 和流式 `stream` 两条路径都支持。先在请求上提供工具定义，再把返回的 `tool_calls` 结果作为 `tool` 消息回填到下一轮对话。

```rust
use llmrust::{ChatRequest, Message, Tool, ToolChoice};
use serde_json::json;

let tools = vec![Tool::function(
    "get_weather",
    Some("获取某城市的当前天气".to_string()),
    json!({
        "type": "object",
        "properties": { "city": { "type": "string" } },
        "required": ["city"]
    }),
)];

let request = ChatRequest::from_messages(
    "claude-3-5-sonnet-20241022",
    vec![Message::user("旧金山现在天气怎么样？")],
)
.with_tools(tools)
.with_tool_choice(ToolChoice::auto());

let response = llm.chat_with("anthropic/claude-3-5-sonnet-20241022", request).await?;
if let Some(calls) = &response.tool_calls {
    for call in calls {
        println!("调用 {} -> {}", call.function.name, call.function.arguments);
    }
}
```

### 高级配置

```rust
use llmrust::{LmrsClient, ChatRequest};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let llm = LmrsClient::new();
    llm.set_anthropic(std::env::var("ANTHROPIC_API_KEY")?).await;

    let request = ChatRequest::new("claude-3-5-sonnet-20241022", "人生的意义是什么？")
        .with_system("你是一个哲学助手。")
        .with_temperature(0.7)
        .with_max_tokens(1000);

    let response = llm.chat_with("anthropic/claude-3-5-sonnet-20241022", request).await?;
    println!("{}", response.content);

    Ok(())
}
```

### JSON 模式 & 采样参数

> JSON 模式以及下面的扩展采样参数会在 OpenAI 兼容服务商（OpenAI、DeepSeek、Moonshot、OpenRouter）上生效，并在 Gemini 有原生等价项时进行映射。

```rust
use llmrust::ChatRequest;

let request = ChatRequest::new("gpt-4o", "以 JSON 格式列出 3 个城市")
    .with_json_mode()
    .with_seed(42)
    .with_temperature(0.2);
```

### 成本估算

用 `ModelPricing` 把 token 用量换算成预估美元成本。价格以每 1,000 token 的美元计价，prompt（输入）与 completion（输出）分别计费：

```rust
use llmrust::{ModelPricing, Usage};

// prompt $0.0025 / 1K，completion $0.01 / 1K
let pricing = ModelPricing::new(0.0025, 0.01);

let usage = Usage { prompt_tokens: 1_000, completion_tokens: 500, total_tokens: 1_500 };
let cost = usage.estimated_cost(&pricing); // 0.0075
println!("预估成本: ${cost:.6}");
```

把它和 `ChatResponse` 返回的 `usage` 搭配，即可为真实请求估算成本。该估算只反映你提供的价格，不考虑服务商折扣或缓存 token 费率。

### HTTP 代理服务器

运行一个本地 API 网关，暴露 OpenAI 兼容 chat + embeddings 和 Anthropic 协议（需要 `features = ["proxy"]`）：

```bash
export LLMRUST_OPENAI_KEY="sk-..."
export LLMRUST_DEEPSEEK_KEY="sk-..."
# 可选：启用 Bearer Token 认证
export LLMRUST_PROXY_KEY="some-shared-secret"
cargo run --example proxy_server --features proxy
```

用 **OpenAI** Chat Completions API 调用：

```bash
curl http://localhost:3000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer some-shared-secret" \
  -d '{
    "model": "deepseek/deepseek-chat",
    "messages": [{"role": "user", "content": "你好！"}]
  }'
```

同一个服务器还会暴露 **Anthropic** Messages API，所以只会说 Anthropic 的客户端（比如为 Claude 构建的工具）无需修改即可使用——即使后端是 OpenAI：

```bash
curl http://localhost:3000/v1/messages \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer some-shared-secret" \
  -d '{
    "model": "openai/gpt-4o",
    "messages": [{"role": "user", "content": "你好！"}],
    "max_tokens": 256
  }'
```

> **安全提示：** 若不设置 `LLMRUST_PROXY_KEY`，代理没有任何认证。仅在 localhost 或反向代理之后运行。设置了 `LLMRUST_PROXY_KEY` 后，每个请求必须带 `Authorization: Bearer <key>` 头，token 使用恒定时间比较。
>
> 代理遵循 OpenAI chat completions 请求约定，包括 `stop` 可为字符串或数组，并会对格式错误的请求返回 JSON 错误体。流式错误会以 error event 返回给客户端，不会静默伪装成成功 completion。
> 流式响应使用 OpenAI 风格 SSE chunk，包括只在首个 delta 中发送一次 `assistant` role。设置 `stream_options.include_usage = true` 时，仅包含 usage 的 chunk 会返回空 `choices`。

### Proxy model 名称

proxy 通过 `model` 字段路由请求。请使用和 client API 一致的 provider 前缀模型名，例如：

- `openai/gpt-4o`
- `anthropic/claude-3-5-sonnet-latest`
- `gemini/gemini-1.5-pro`
- `ollama/llama3.2`

前缀用于选择 provider，后面的模型名会在 llmrust 完成路由解析后发送给对应 provider。

## 日志

`llmrust` 会通过 [`tracing`](https://docs.rs/tracing) 输出结构化事件，覆盖 provider 注册、请求生命周期（包括便捷 `chat` / `stream` API）、proxy 请求、重试、router failover 以及上游 API 错误。作为依赖库，它不会安装全局 subscriber；日志如何收集由你的应用决定。

```toml
[dependencies]
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
```

```rust
tracing_subscriber::fmt()
    .with_env_filter("llmrust=debug")
    .init();
```

llmrust 的 tracing 日志不会记录 API key、prompt 内容、模型响应文本、请求体、工具参数、图片数据或完整 URL。需要定位问题时，只记录数量或长度，例如 `message_count`、`tool_count`、`data_len`、`url_len`。`provider`、`model`、HTTP `status`、重试 `attempt`、router `group` 等运行元数据始终会被记录。

## 服务商配置

### OpenAI

```rust
llm.set_openai("sk-...").await;
// 自定义 base URL（Azure、本地代理等）
llm.set_openai_compatible("sk-...", "https://your-proxy.com/v1").await;
```

### Anthropic

```rust
llm.set_anthropic("sk-ant-...").await;
```

### DeepSeek

```rust
llm.set_deepseek("sk-...").await;
```

### Google Gemini

```rust
llm.set_google("AIza...").await;
```

### Ollama（本地模型）

```rust
llm.set_ollama(None).await;  // 默认：http://localhost:11434
llm.set_ollama(Some("http://your-server:11434")).await;
```

### Moonshot / Kimi

```rust
llm.set_moonshot("sk-...").await;
```

### OpenRouter

```rust
llm.set_openrouter("sk-or-...").await;
```

## 错误处理

```rust
use llmrust::{LmrsClient, LlmError};

match llm.chat("openai/gpt-4o", "你好").await {
    Ok(response) => println!("{}", response.content),
    Err(LlmError::Api { status, message }) => {
        eprintln!("API 错误 {}: {}", status, message);
    }
    Err(LlmError::UnknownProvider(name)) => {
        eprintln!("未注册的服务商: '{}'", name);
    }
    Err(e) => eprintln!("错误: {}", e),
}
```

## 性能

尚未发布正式基准测试。该库在 `reqwest` 之上仅增加薄薄一层异步封装，每个请求的开销最多几百微秒。

## 对比

### 与 Python LiteLLM 对比

| 特性 | Python LiteLLM | llmrust |
|---|---|---|
| 启动时间 | ~数百毫秒（解释器） | 编译后二进制，接近瞬间 |
| 内存 | 依赖 Python 运行时 | 单一静态二进制 |
| 并发 | asyncio | tokio（原生） |
| 部署 | Python + venv | 单一二进制 |
| 类型安全 | 运行时 | 编译期 |
| 服务商 | 100+ | 7（持续增加） |

### 与其他 Rust crate 对比

| | llmrust | genai | rig | llm / rllm |
|---|---|---|---|---|
| 定位 | 精简统一客户端 + 代理 | 广覆盖统一客户端 | Agent 框架 | 客户端 + 额外能力（TTS/STT/视觉） |
| 服务商数量 | 7（持续增加） | 25+ | 若干 | 多 |
| 内置代理服务器 | ✅ OpenAI **+** Anthropic | ➖ | ➖ | 仅 OpenAI REST |
| 跨服务商归一化 logprobs | ✅（含 Gemini） | ➖ | ➖ | ➖ |
| 默认依赖 | 极少（`default = []`） | 中等 | 重（框架） | 中等+ |
| 额外范围 | 无 | 仅客户端 | agents / RAG | embeddings / 视觉 / 音频 |

> 其他 crate 的服务商数量和功能集变化很快——请把这当作定位示意，而非基准测试。选择最适合你需求的工具。

## 路线图

- [x] 核心服务商（OpenAI、Anthropic、DeepSeek）
- [x] 流式支持
- [x] Google Gemini、Ollama、Moonshot、OpenRouter
- [x] HTTP 代理服务器（OpenAI + Anthropic 协议）
- [x] 重试逻辑
- [x] 工具调用 / Function calling（OpenAI 兼容、Anthropic、Gemini；非流式）
- [x] JSON 模式 & 采样参数（OpenAI 兼容服务商）
- [x] 流式工具调用（从流式分片中重建工具调用）
- [x] Embeddings API（OpenAI 兼容 + Ollama，proxy `/v1/embeddings`）
- [ ] 批量 API
- [ ] 速率限制
- [ ] 更多服务商（Cohere、Mistral、Groq 等）

## 许可证

双许可，任选其一：

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

## 贡献

欢迎贡献 —— 无论你是人类、AI coding agent，还是人机协作团队。

完整指南请参阅 [`CONTRIBUTING.md`](CONTRIBUTING.md)；如果你是 AI agent，请先阅读 [`AGENTS.md`](AGENTS.md)。

```bash
cargo build --all-targets --all-features
cargo test
cargo test --all-features
cargo clippy --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
cargo fmt --check
```
