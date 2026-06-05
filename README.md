# llmrust

> Call multiple LLM APIs with one unified Rust interface.
> 通过统一的 Rust 接口调用多个 LLM API。

[![Crates.io](https://img.shields.io/crates/v/llmrust)](https://crates.io/crates/llmrust)
[![Documentation](https://docs.rs/llmrust/badge.svg)](https://docs.rs/llmrust)
[![License](https://img.shields.io/crates/l/llmrust)](https://github.com/llmrust/llmrust)
[![CI](https://github.com/llmrust/llmrust/actions/workflows/ci.yml/badge.svg)](https://github.com/llmrust/llmrust/actions)

**English** | [中文](#chinese)

---

<a name="chinese"></a>

**English** | [中文](#chinese)

---

## 📖 Introduction / 简介

**English:**

A high-performance, type-safe Rust library for calling multiple LLM providers through a unified interface. Inspired by Python's [LiteLLM](https://github.com/BerriAI/litellm), but built for Rust's performance and safety guarantees.

**中文:**

一个高性能、类型安全的 Rust 库，通过统一接口调用多个 LLM 提供商。灵感来自 Python 的 [LiteLLM](https://github.com/BerriAI/litellm)，但基于 Rust 的性能和安全特性构建。

## ✨ Features / 特性

**English:**

- **Unified API** — One interface for OpenAI, Anthropic, DeepSeek, Google Gemini, Ollama, and more
- **Streaming support** — First-class async streaming for all providers
- **Type-safe** — Full compile-time guarantees with serde and thiserror
- **High performance** — Built on reqwest + tokio, minimal overhead
- **Zero runtime dependencies** — Single binary, no Python/Node required

**中文:**

- **统一 API** — 同一接口支持 OpenAI、Anthropic、DeepSeek、Google Gemini、Ollama 等
- **流式支持** — 所有提供商的一等异步流式响应
- **类型安全** — 通过 serde 和 thiserror 提供编译期保证
- **高性能** — 基于 reqwest + tokio 构建，极低开销
- **零运行时依赖** — 单一二进制，无需 Python/Node

## 🚀 Quick Start / 快速开始

```bash
cargo add llmrust
```

```rust
use llmrust::LmrsClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let llm = LmrsClient::new();

    // Register providers / 注册提供商
    llm.set_openai("sk-...").await;
    llm.set_anthropic("sk-ant-...").await;
    llm.set_deepseek("sk-...").await;

    // Call any model with provider/model format
    // 使用 provider/model 格式调用任意模型
    let resp = llm.chat("openai/gpt-4o", "Hello, world!").await?;
    println!("{}", resp.content);

    Ok(())
}
```

## 🔌 Supported Providers / 支持的服务商

| Provider / 提供商 | Models / 模型 | Streaming / 流式 | Status / 状态 |
|---|---|---|---|
| OpenAI | gpt-4o, gpt-4o-mini, o1-preview | ✅ | Stable |
| Anthropic | claude-3.5-sonnet, claude-3-opus | ✅ | Stable |
| DeepSeek | deepseek-chat, deepseek-coder | ✅ | Stable |
| Google Gemini | gemini-2.0-flash, gemini-1.5-pro | ✅ | Stable |
| Ollama | llama3.2, qwen2.5, any local model | ✅ | Stable |
| Moonshot / Kimi | moonshot-v1-8k, kimi-latest | ✅ | Stable |
| OpenRouter | any model via OpenRouter | ✅ | Stable |

## 📝 Usage Examples / 使用示例

### Basic Chat / 基础对话

```rust
use llmrust::LmrsClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let llm = LmrsClient::new();
    llm.set_deepseek(std::env::var("DEEPSEEK_API_KEY")?).await;

    let response = llm.chat("deepseek/deepseek-chat", "Explain Rust ownership in one paragraph.").await?;
    println!("{}", response.content);
    println!("Tokens: {:?}", response.usage);

    Ok(())
}
```

### Streaming / 流式响应

```rust
use llmrust::LmrsClient;
use futures::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let llm = LmrsClient::new();
    llm.set_openai(std::env::var("OPENAI_API_KEY")?).await;

    let mut stream = llm.stream("openai/gpt-4o", "Write a haiku about Rust.").await?;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        print!("{}", chunk.delta);
    }
    println!();

    Ok(())
}
```

### Advanced Configuration / 高级配置

```rust
use llmrust::{LmrsClient, ChatRequest, Message};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let llm = LmrsClient::new();
    llm.set_anthropic(std::env::var("ANTHROPIC_API_KEY")?).await;

    let request = ChatRequest::new("claude-3-5-sonnet-20241022", "What is the meaning of life?")
        .with_system("You are a philosophical assistant.")
        .with_temperature(0.7)
        .with_max_tokens(1000);

    let response = llm.chat_with("anthropic/claude-3-5-sonnet-20241022", request).await?;
    println!("{}", response.content);

    Ok(())
}
```

### HTTP Proxy Server / HTTP 代理服务器

Run a local OpenAI-compatible API gateway / 运行一个本地 OpenAI 兼容的 API 网关：

```bash
export LLMRUST_OPENAI_KEY="sk-..."
export LLMRUST_DEEPSEEK_KEY="sk-..."
# Optional: enable bearer-token auth. If set, clients must send
# `Authorization: Bearer <token>` on every request.
# 可选：启用 Bearer Token 认证。设置后客户端每次请求必须
# 携带 `Authorization: Bearer <token>` 头。
export LLMRUST_PROXY_KEY="some-shared-secret"
cargo run --example proxy_server
```

Then use any OpenAI client / 然后使用任意 OpenAI 客户端：

```bash
curl http://localhost:3000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer some-shared-secret" \
  -d '{
    "model": "deepseek/deepseek-chat",
    "messages": [{"role": "user", "content": "Hello!"}]
  }'
```

**Security note / 安全提示:** Without `LLMRUST_PROXY_KEY` set, the proxy has no authentication and will happily burn through whatever API keys you registered. Only run it on `localhost`, behind a reverse proxy, or with `LLMRUST_PROXY_KEY` set to a strong shared secret.

若不设置 `LLMRUST_PROXY_KEY`，代理无任何认证，会消耗你注册的所有 API Key。仅在 `localhost`、反向代理之后、或设置了强 `LLMRUST_PROXY_KEY` 时运行。

## ⚙️ Provider Configuration / 服务商配置

### OpenAI

```rust
llm.set_openai("sk-...").await;
// Or with a custom base URL (for Azure, local proxies, or any
// OpenAI-compatible API). The base URL must include the `/v1` (or
// equivalent) path prefix; `/chat/completions` is appended automatically.
// 或使用自定义 base URL（Azure、本地代理等），需包含 `/v1` 路径前缀
llm.set_openai_compatible("sk-...", "https://your-proxy.com/v1").await;
// Azure OpenAI example / 示例:
llm.set_openai_compatible("sk-...", "https://YOUR_RESOURCE.openai.azure.com/openai/deployments/YOUR_DEPLOYMENT").await;
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

### Ollama (Local Models / 本地模型)

```rust
llm.set_ollama(None).await;  // Default: http://localhost:11434
// Or with custom URL / 或自定义地址
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

## 🔒 Error Handling / 错误处理

```rust
use llmrust::{LmrsClient, LlmError};

match llm.chat("openai/gpt-4o", "Hello").await {
    Ok(response) => println!("{}", response.content),
    Err(LlmError::Api { status, message }) => {
        eprintln!("API error {}: {}", status, message);
    }
    Err(LlmError::UnknownProvider(name)) => {
        eprintln!("Provider '{}' not registered / 未注册", name);
    }
    Err(e) => eprintln!("Error / 错误: {}", e),
}
```

## 📊 Performance / 性能

**English:**

We have not yet published formal benchmarks. The library adds a thin async layer on top of `reqwest` (no JSON re-parsing, no extra buffering), so overhead versus calling the HTTP API directly should be a few hundred microseconds per request at most. Real numbers will be added once a `cargo bench` suite lands.

**中文:**

我们尚未发布正式基准测试。该库在 `reqwest` 之上仅增加了一层薄薄的异步封装（无 JSON 重新解析、无额外缓冲），因此与直接调用 HTTP API 相比，每个请求的开销最多几百微秒。`cargo bench` 基准测试上线后会补充具体数据。

## 🔄 Comparison / 对比

| Feature / 特性 | Python LiteLLM | llmrust |
|---|---|---|
| Startup time / 启动时间 | ~hundreds ms (interpreter) | compiled binary, near-instant |
| Memory usage / 内存占用 | depends on Python runtime | single static binary |
| Concurrency / 并发 | asyncio | tokio (native) |
| Deployment / 部署 | Python + venv | Single binary |
| Type safety / 类型安全 | Runtime | Compile-time |
| Providers / 服务商 | 100+ | 7 (growing) |

## 🗺️ Roadmap / 路线图

- [x] Core providers (OpenAI, Anthropic, DeepSeek) / 核心服务商
- [x] Streaming support / 流式支持
- [x] Google Gemini, Ollama, Moonshot, OpenRouter
- [x] HTTP proxy server / HTTP 代理服务器
- [x] Retry logic / 重试逻辑
- [ ] Tool-use / Function calling / 工具调用
- [ ] Embeddings API / 嵌入 API
- [ ] Batch API / 批量 API
- [ ] Rate limiting / 速率限制
- [ ] More providers (Cohere, Mistral, Groq, etc.) / 更多服务商

## 📄 License / 许可证

Licensed under either of / 双许可：

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option / 任选其一。

## 🤝 Contributing / 贡献

Contributions welcome! Please open an issue or PR / 欢迎贡献！请提交 Issue 或 PR。

```bash
# Development / 开发
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```
