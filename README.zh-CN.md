# llmrust

![llmrust](images/llmrust-banner.png)

[![Crates.io](https://img.shields.io/crates/v/llmrust)](https://crates.io/crates/llmrust)
[![Documentation](https://docs.rs/llmrust/badge.svg)](https://docs.rs/llmrust)
[![License](https://img.shields.io/crates/l/llmrust)](https://github.com/llmrust/llmrust)
[![CI](https://github.com/llmrust/llmrust/actions/workflows/ci.yml/badge.svg)](https://github.com/llmrust/llmrust/actions)

[English](README.md) | **中文版**

---

> 通过统一的 Rust 接口调用多个 LLM API。

一个高性能、类型安全的 Rust 库，通过统一接口调用多个 LLM 提供商。灵感来自 Python 的 [LiteLLM](https://github.com/BerriAI/litellm)，但基于 Rust 的性能和安全特性构建。

## 特性

- **统一 API** — 同一接口支持 OpenAI、Anthropic、DeepSeek、Google Gemini、Ollama 等
- **流式支持** — 所有提供商都支持异步流式响应
- **类型安全** — 编译期保证，无运行时错误
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
| *(无)* | ✅ | LLM 客户端 — 全部提供商 + 流式；工具调用与 JSON 模式支持 OpenAI 兼容提供商 |
| `proxy` | ❌ | 内置 OpenAI 兼容 HTTP 代理服务器（会引入 `axum`）|

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

| 服务商 | 模型 | 流式 | 工具调用 | 状态 |
|---|---|---|---|---|
| OpenAI | gpt-4o, gpt-4o-mini, o1-preview | ✅ | ✅ | 稳定 |
| DeepSeek | deepseek-chat, deepseek-coder | ✅ | ✅ | 稳定 |
| Moonshot / Kimi | moonshot-v1-8k, kimi-latest | ✅ | ✅ | 稳定 |
| OpenRouter | 通过 OpenRouter 访问任意模型 | ✅ | ✅ | 稳定 |
| Anthropic | claude-3.5-sonnet, claude-3-opus | ✅ | 🚧 计划中 (0.2) | 稳定（对话）|
| Google Gemini | gemini-2.0-flash, gemini-1.5-pro | ✅ | 🚧 计划中 (0.2) | 稳定（对话）|
| Ollama | llama3.2, qwen2.5 等本地模型 | ✅ | ➖ | 稳定（对话）|

> **功能支持说明**
>
> - **工具调用 / Function calling** 与 **JSON 模式** 目前通过 OpenAI 兼容服务商（OpenAI、DeepSeek、Moonshot、OpenRouter）提供。Anthropic 与 Gemini 的原生工具调用正在开发中，计划于 0.2 版本补齐。
> - 除 `temperature` / `max_tokens` / `top_p` 之外的**采样参数**（如 `stop`、`seed`、`presence_penalty`、`frequency_penalty`、`logprobs`、`n`、`response_format`）目前会发送给 OpenAI 兼容服务商；Anthropic、Gemini、Ollama 当前仅支持核心采样参数。

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

> JSON 模式以及下面的扩展采样参数目前在 OpenAI 兼容服务商（OpenAI、DeepSeek、Moonshot、OpenRouter）上生效。

```rust
use llmrust::ChatRequest;

let request = ChatRequest::new("gpt-4o", "以 JSON 格式列出 3 个城市")
    .with_json_mode()
    .with_seed(42)
    .with_temperature(0.2);
```

### HTTP 代理服务器

运行一个本地 OpenAI 兼容的 API 网关（需要 `features = ["proxy"]`）：

```bash
export LLMRUST_OPENAI_KEY="sk-..."
export LLMRUST_DEEPSEEK_KEY="sk-..."
# 可选：启用 Bearer Token 认证
export LLMRUST_PROXY_KEY="some-shared-secret"
cargo run --example proxy_server --features proxy
```

```bash
curl http://localhost:3000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer some-shared-secret" \
  -d '{
    "model": "deepseek/deepseek-chat",
    "messages": [{"role": "user", "content": "你好！"}]
  }'
```

> **安全提示：** 若不设置 `LLMRUST_PROXY_KEY`，代理没有任何认证。仅在 localhost 或反向代理之后运行。

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

| 特性 | Python LiteLLM | llmrust |
|---|---|---|
| 启动时间 | ~数百毫秒（解释器） | 编译后二进制，接近瞬间 |
| 内存 | 依赖 Python 运行时 | 单一静态二进制 |
| 并发 | asyncio | tokio（原生） |
| 部署 | Python + venv | 单一二进制 |
| 类型安全 | 运行时 | 编译期 |
| 服务商 | 100+ | 7（持续增加） |

## 路线图

- [x] 核心服务商（OpenAI、Anthropic、DeepSeek）
- [x] 流式支持
- [x] Google Gemini、Ollama、Moonshot、OpenRouter
- [x] HTTP 代理服务器
- [x] 重试逻辑
- [x] 工具调用 / Function calling（OpenAI 兼容；Anthropic 与 Gemini 进行中）
- [x] JSON 模式 & 采样参数（OpenAI 兼容服务商）
- [ ] Embeddings API
- [ ] 批量 API
- [ ] 速率限制
- [ ] 更多服务商（Cohere、Mistral、Groq 等）

## 许可证

双许可，任选其一：

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

## 贡献

欢迎贡献！请提交 Issue 或 PR。

```bash
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```
