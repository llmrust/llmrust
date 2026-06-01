# llmrust

> Call multiple LLM APIs with one unified Rust interface.

[![Crates.io](https://img.shields.io/crates/v/llmrust)](https://crates.io/crates/llmrust)
[![Documentation](https://docs.rs/llmrust/badge.svg)](https://docs.rs/llmrust)
[![License](https://img.shields.io/crates/l/llmrust)](https://github.com/llmrust/llmrust)
[![CI](https://github.com/llmrust/llmrust/actions/workflows/ci.yml/badge.svg)](https://github.com/llmrust/llmrust/actions)

A high-performance, type-safe Rust library for calling multiple LLM providers through a unified interface. Inspired by Python's [LiteLLM](https://github.com/BerriAI/litellm), but built for Rust's performance and safety guarantees.

## Features

- **Unified API**: One interface for OpenAI, Anthropic, DeepSeek, Google Gemini, Ollama, and more
- **Streaming support**: First-class async streaming for all providers
- **Type-safe**: Full compile-time guarantees with serde and thiserror
- **High performance**: Built on reqwest + tokio, minimal overhead
- **Zero runtime dependencies**: Single binary, no Python/Node required

## Quick Start

```bash
cargo add llmrust
```

```rust
use llmrust::LiteLLM;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let llm = LiteLLM::new();
    
    // Register providers
    llm.set_openai("sk-...").await;
    llm.set_anthropic("sk-ant-...").await;
    llm.set_deepseek("sk-...").await;
    
    // Call any model with provider/model format
    let resp = llm.chat("openai/gpt-4o", "Hello, world!").await?;
    println!("{}", resp.content);
    
    Ok(())
}
```

## Supported Providers

| Provider | Models | Streaming | Status |
|----------|--------|-----------|--------|
| OpenAI | gpt-4o, gpt-4o-mini, o1-preview | ✅ | Stable |
| Anthropic | claude-3.5-sonnet, claude-3-opus | ✅ | Stable |
| DeepSeek | deepseek-chat, deepseek-coder | ✅ | Stable |
| Google Gemini | gemini-2.0-flash, gemini-1.5-pro | ✅ | Stable |
| Ollama | llama3.2, qwen2.5, any local model | ✅ | Stable |
| Moonshot/Kimi | moonshot-v1-8k, kimi-latest | ✅ | Stable |
| OpenRouter | any model via OpenRouter | ✅ | Stable |

## Usage Examples

### Basic Chat

```rust
use llmrust::LiteLLM;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let llm = LiteLLM::new();
    llm.set_deepseek(std::env::var("DEEPSEEK_API_KEY")?).await;
    
    let response = llm.chat("deepseek/deepseek-chat", "Explain Rust ownership in one paragraph.").await?;
    println!("{}", response.content);
    println!("Tokens: {:?}", response.usage);
    
    Ok(())
}
```

### Streaming

```rust
use llmrust::LiteLLM;
use futures::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let llm = LiteLLM::new();
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

### Advanced Configuration

```rust
use llmrust::{LiteLLM, ChatRequest, Message};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let llm = LiteLLM::new();
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

### HTTP Proxy Server

Run a local OpenAI-compatible API gateway:

```bash
export LLMRUST_OPENAI_KEY="sk-..."
export LLMRUST_DEEPSEEK_KEY="sk-..."
# Optional: enable bearer-token auth. If set, clients must send
# `Authorization: Bearer <token>` on every request.
export LLMRUST_PROXY_KEY="some-shared-secret"
cargo run --example proxy_server
```

Then use any OpenAI client:

```bash
curl http://localhost:3000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer some-shared-secret" \
  -d '{
    "model": "deepseek/deepseek-chat",
    "messages": [{"role": "user", "content": "Hello!"}]
  }'
```

**Security note:** Without `LLMRUST_PROXY_KEY` set, the proxy has no
authentication and will happily burn through whatever API keys you
registered. Only run it on `localhost`, behind a reverse proxy, or with
`LLMRUST_PROXY_KEY` set to a strong shared secret.

## Provider Configuration

### OpenAI

```rust
llm.set_openai("sk-...").await;
// Or with a custom base URL (for Azure, local proxies, or any
// OpenAI-compatible API). The base URL must include the `/v1` (or
// equivalent) path prefix; `/chat/completions` is appended automatically.
llm.set_openai_compatible("sk-...", "https://your-proxy.com/v1").await;
// Azure OpenAI example:
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

### Ollama (Local Models)

```rust
llm.set_ollama(None).await;  // Default: http://localhost:11434
// Or with custom URL
llm.set_ollama(Some("http://your-server:11434")).await;
```

### Moonshot/Kimi

```rust
llm.set_moonshot("sk-...").await;
```

### OpenRouter

```rust
llm.set_openrouter("sk-or-...").await;
```

## Error Handling

```rust
use llmrust::{LiteLLM, LlmError};

match llm.chat("openai/gpt-4o", "Hello").await {
    Ok(response) => println!("{}", response.content),
    Err(LlmError::Api { status, message }) => {
        eprintln!("API error {}: {}", status, message);
    }
    Err(LlmError::UnknownProvider(name)) => {
        eprintln!("Provider '{}' not registered", name);
    }
    Err(e) => eprintln!("Error: {}", e),
}
```

## Performance

We have not yet published formal benchmarks. The library adds a thin async
layer on top of `reqwest` (no JSON re-parsing, no extra buffering), so
overhead versus calling the HTTP API directly should be a few hundred
microseconds per request at most. Real numbers will be added once a
`cargo bench` suite lands.

## Comparison with Python LiteLLM

| Feature | Python LiteLLM | llmrust |
|---------|----------------|---------|
| Startup time | a few hundred ms (interpreter) | compiled binary, near-instant |
| Memory usage | depends on Python runtime | single static binary, no runtime |
| Concurrency | asyncio | tokio (native) |
| Deployment | Python + venv | Single binary |
| Type safety | Runtime | Compile-time |
| Providers | 100+ | 7 (growing) |

## Roadmap

- [x] Core providers (OpenAI, Anthropic, DeepSeek)
- [x] Streaming support
- [x] Google Gemini, Ollama, Moonshot, OpenRouter
- [x] HTTP proxy server
- [ ] Tool-use / Function calling
- [ ] Embeddings API
- [ ] Batch API
- [ ] Rate limiting and retry logic
- [ ] More providers (Cohere, Mistral, Groq, etc.)

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Contributing

Contributions welcome! Please open an issue or PR.

```bash
# Development
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```
