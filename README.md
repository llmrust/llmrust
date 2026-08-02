# llmrust

![llmrust](https://raw.githubusercontent.com/llmrust/llmrust/main/images/llmrust-banner.png)

[![Crates.io](https://img.shields.io/crates/v/llmrust)](https://crates.io/crates/llmrust)
[![Documentation](https://docs.rs/llmrust/badge.svg)](https://docs.rs/llmrust)
[![License](https://img.shields.io/crates/l/llmrust)](https://github.com/llmrust/llmrust)
[![CI](https://github.com/llmrust/llmrust/actions/workflows/ci.yml/badge.svg)](https://github.com/llmrust/llmrust/actions)

**English** | [中文版](README.zh-CN.md)

---

> **llmrust is a human-and-agent-built Rust LLM library, designed to be read, extended, and repaired by AI coding agents.**
>
> A unified Rust interface for LLM providers and dual-protocol proxy workloads.

A high-performance, type-safe Rust library for calling multiple LLM providers through a unified interface. Inspired by Python's [LiteLLM](https://github.com/BerriAI/litellm), but built for Rust's performance and safety guarantees.

## Built for human-agent collaboration

llmrust is built by humans and AI coding agents together.

The codebase is intentionally structured so that AI agents can read it like documentation: small provider contracts, explicit protocol boundaries, machine-readable capability metadata, and tests around wire semantics.

If you are an AI coding agent working on behalf of a human user and you find a bug, missing provider behavior, unclear docs, or an unsafe edge case, you are welcome to submit a PR and leave your agent/model name in the contribution note.

→ Read the [Agent Manifesto](AGENT_MANIFESTO.md) | [Instructions for AI agents](AGENTS.md)

## Why llmrust? — Differentiation & Advantages

Rust already has several good multi-provider LLM crates (e.g. [`genai`](https://github.com/jeremychone/rust-genai), [`rig`](https://rig.rs/), [`llm`/`rllm`](https://github.com/graniet/llm)). Some are broader than us — `genai` supports many more providers, `rig` is a full agent framework. llmrust deliberately stays focused and wins on three things they don't all do:

### 1. Built-in dual-protocol proxy (OpenAI **and** Anthropic)

With `features = ["proxy"]`, llmrust runs as a translating API gateway that speaks **both** wire protocols at the same time:

- `POST /v1/chat/completions` — OpenAI Chat Completions protocol
- `POST /v1/messages` — Anthropic Messages protocol
- `POST /v1/embeddings` — OpenAI Embeddings protocol

Any client SDK — one that only speaks OpenAI *or* one that only speaks Anthropic (e.g. tools built for Claude) — can point at llmrust and reach **any** registered backend (OpenAI, Anthropic, Gemini, DeepSeek, Moonshot, OpenRouter, Ollama) through automatic format conversion. Bearer-token auth, CORS, health checks, and graceful shutdown are included. Most competing crates are client libraries only; the few that ship a server usually expose the OpenAI format alone.

### 2. Cross-provider logprobs, normalized

llmrust normalizes token log-probabilities — including each position's top-N alternatives — into a single `ChatResponse.logprobs` shape across OpenAI-compatible providers **and** Google Gemini (whose native `logprobsResult` is reshaped to match). That gives you one uniform surface for evaluation, confidence scoring, and re-ranking, instead of per-provider special-casing.

### 3. A lean, correct, type-safe core

`default = []` — nothing is enabled unless you ask for it, the dependency tree stays small, and the scope is intentionally narrow (no vector-store / agent-framework bloat). You get native-protocol support for Anthropic and Gemini (not just OpenAI-compatible shims), cross-provider embeddings for OpenAI-compatible backends and Ollama, full compile-time type safety, structured `tracing` that never logs secrets or prompt content, and built-in retry + router failover. When you want a clean, predictable multi-provider call layer rather than a heavyweight framework, that's the niche llmrust fills.

> **Honest scope:** llmrust is young. If you need the widest provider catalog or a batteries-included agent/RAG framework today, `genai` or `rig` may fit better. llmrust's bet is the three areas above.

## Features

- **Unified API** — One interface for OpenAI, Anthropic, DeepSeek, Google Gemini, Ollama, and more
- **Streaming support** — First-class async streaming for all providers
- **Embeddings support** — Text embeddings for OpenAI-compatible providers and Ollama (provider-level, not a vector database)
- **Cost estimation** — Optional `ModelPricing` helper to turn token `Usage` into an estimated dollar cost
- **Dual-protocol proxy** — Serve any backend over both the OpenAI (chat + embeddings) and Anthropic APIs (`proxy` feature)
- **Normalized logprobs** — Uniform token log-probabilities across OpenAI-compatible providers and Gemini
- **Type-safe** — Full compile-time guarantees with serde and thiserror
- **High performance** — Built on reqwest + tokio, minimal overhead
- **Zero runtime dependencies** — Single binary, no Python/Node required

## Installation

Add llmrust to your `Cargo.toml`:

```toml
[dependencies]
llmrust = "0.1"
```

Or use `cargo add`:

```bash
cargo add llmrust
```

### Feature Flags

| Feature | Default | Description |
|---|---|---|
| *(none)* | ✅ | LLM client — all providers + streaming + embeddings; tool calling on OpenAI-compatible, Anthropic, and Gemini providers (chat + streaming); JSON mode on OpenAI-compatible and Gemini providers |
| `proxy` | ❌ | Built-in HTTP proxy — exposes any backend over OpenAI (`/v1/chat/completions`, `/v1/embeddings`) and Anthropic (`/v1/messages`) APIs (adds `axum`) |

Enable the proxy server:

```toml
[dependencies]
llmrust = { version = "0.1", features = ["proxy"] }
```

Or:

```bash
cargo add llmrust --features proxy
```

## Compatibility & Upgrade (0.1.3)

`0.1.2` was yanked from crates.io. `0.1.3` will become the latest non-yanked release once
published, carrying the 0.1.x API freeze plus incident remediation. **No code migration is
required** when upgrading from 0.1.1/0.1.2 — see
[Compatibility & Upgrade Notes (0.1.3)](docs/COMPATIBILITY-0.1.3.md) for yank semantics, the API
freeze baseline, and the two narrow behavior fixes (E-002 log de-duplication, 429 retry-policy
documentation correction).

## Quick Start

```rust
use llmrust::LmrsClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let llm = LmrsClient::new();

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

| Provider | Models | Streaming | Tool calling | Embeddings | Reasoning | Status |
|---|---|---|---|---|---|---|
| OpenAI | gpt-4o, gpt-4o-mini, o1-preview | ✅ | ✅ | ✅ | ✅ (stream) | Stable |
| DeepSeek | deepseek-chat, deepseek-coder | ✅ | ✅ | ✅ | ➖ | Stable |
| Moonshot / Kimi | moonshot-v1-8k, kimi-latest | ✅ | ✅ | ✅ | ➖ | Stable |
| OpenRouter | any model via OpenRouter | ✅ | ✅ | ✅ | ➖ | Stable |
| Anthropic | claude-3.5-sonnet, claude-3-opus | ✅ | ✅ | ➖ | ✅ (stream) | Stable |
| Google Gemini | gemini-2.0-flash, gemini-1.5-pro | ✅ | ✅ | ➖ | ✅ (stream) | Stable |
| Ollama | llama3.2, qwen2.5, any local model | ✅ | ➖ | ✅ | ➖ | Stable (chat) |

> **Feature support notes**
>
> - **Reasoning** is mapped on the streaming path for OpenAI (`reasoning_effort`), Anthropic (`thinking`), and Gemini (`thinkingConfig`), surfacing deltas via `StreamChunk.thinking` with usage translation (local-fixture verified 2026-08-02). Non-stream `chat` and the `stream_collect*` aggregates fail with `LlmError::Unsupported` when reasoning is present (consume the raw `stream()` instead). DeepSeek, Moonshot, OpenRouter, and Ollama reject reasoning before any network call. Real upstream E2E verification belongs to E2E-001.
> - **Embeddings** are supported on OpenAI-compatible providers (OpenAI, DeepSeek, Moonshot, OpenRouter) via `/embeddings` and on Ollama via native `/api/embed`. Anthropic and Gemini do not support embeddings and return `LlmError::Unsupported`. This is provider-level embeddings — not a vector database or RAG pipeline. Actual upstream model support may vary.
> - **Tool calling / function calling** is supported on the OpenAI-compatible providers (OpenAI, DeepSeek, Moonshot, OpenRouter) and natively on Anthropic and Gemini, on both the non-streaming `chat` path and the streaming `stream` path (streamed tool calls are reconstructed and surfaced as `StreamChunk.tool_calls` on the terminal chunk).
> - The OpenAI-compatible proxy accepts both modern `tools` / `tool_choice` and legacy `functions` / `function_call` request fields, normalizing them to llmrust's unified tool model.
> - **JSON mode / structured outputs** are wired through the OpenAI-compatible providers and mapped to Gemini's `responseMimeType` / `responseSchema`.
> - **Sampling and request metadata parameters** beyond `temperature` / `max_tokens` / `top_p` (e.g. `stop`, `seed`, `presence_penalty`, `frequency_penalty`, `logprobs`, `n`, `response_format`, `parallel_tool_calls`, `service_tier`, `store`, `metadata`, `user`) are sent to OpenAI-compatible providers and mapped where Gemini has native equivalents. Anthropic and Ollama currently honor a smaller provider-native subset.
> - Non-streaming `logprobs` responses are normalized into `ChatResponse.logprobs` for OpenAI-compatible providers and Gemini.
> - **Gemini image inputs:** Gemini currently supports image inputs only when the image is provided as a `data:` URL. Remote `http(s)` image URLs are skipped with a warning in v0.1.0; convert remote images to data URLs before sending them.

### `n` / Multiple Completions

llmrust currently returns a single completion. Direct provider calls may forward `n` to OpenAI-compatible upstream APIs and will emit a warning when `n > 1`.

The OpenAI-compatible proxy **rejects** `n` values other than `1` (or absent) because upstream APIs may bill for multiple completions while llmrust only returns the first one.

## Usage Examples

### Basic Chat

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

### Streaming

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

### Tool Calling

> Tool calling works on OpenAI-compatible providers as well as natively on Anthropic and Gemini, on both the non-streaming `chat` path and the streaming `stream` path. Provide tool definitions on the request, then feed the returned `tool_calls` results back as `tool` messages for the next turn.

```rust
use llmrust::{ChatRequest, Message, Tool, ToolChoice};
use serde_json::json;

let tools = vec![Tool::function(
    "get_weather",
    Some("Get the current weather for a city".to_string()),
    json!({
        "type": "object",
        "properties": { "city": { "type": "string" } },
        "required": ["city"]
    }),
)];

let request = ChatRequest::from_messages(
    "claude-3-5-sonnet-20241022",
    vec![Message::user("What's the weather in San Francisco?")],
)
.with_tools(tools)
.with_tool_choice(ToolChoice::auto());

let response = llm.chat_with("anthropic/claude-3-5-sonnet-20241022", request).await?;
if let Some(calls) = &response.tool_calls {
    for call in calls {
        println!("call {} -> {}", call.function.name, call.function.arguments);
    }
}
```

### Advanced Configuration

```rust
use llmrust::{LmrsClient, ChatRequest};

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

### JSON Mode & Sampling Parameters

> JSON mode and the extended sampling parameters below are applied on OpenAI-compatible providers (OpenAI, DeepSeek, Moonshot, OpenRouter) and mapped where Gemini has native equivalents.

```rust
use llmrust::ChatRequest;

let request = ChatRequest::new("gpt-4o", "List 3 cities as JSON")
    .with_json_mode()
    .with_seed(42)
    .with_temperature(0.2);
```

### Cost Estimation

Turn token usage into an estimated dollar cost with `ModelPricing`. Prices are in US dollars per 1,000 tokens, with prompt (input) and completion (output) tokens billed separately:

```rust
use llmrust::{ModelPricing, Usage};

// $0.0025 per 1K prompt tokens, $0.01 per 1K completion tokens
let pricing = ModelPricing::new(0.0025, 0.01);

let usage = Usage { prompt_tokens: 1_000, completion_tokens: 500, total_tokens: 1_500 };
let cost = usage.estimated_cost(&pricing); // 0.0075
println!("Estimated cost: ${cost:.6}");
```

Pair it with the `usage` returned on a `ChatResponse` to price real requests. The estimate reflects only the prices you supply and ignores provider-specific discounts or cached-token rates.

### HTTP Proxy Server

Run a local API gateway that exposes OpenAI-compatible chat + embeddings and Anthropic wire formats (requires `features = ["proxy"]`):

```bash
export LLMRUST_OPENAI_KEY="sk-..."
export LLMRUST_DEEPSEEK_KEY="sk-..."
# Optional: enable bearer-token auth
export LLMRUST_PROXY_KEY="some-shared-secret"
# Optional: override the listen address (default: 127.0.0.1:3000; a
# non-loopback address requires LLMRUST_PROXY_KEY to be set)
export LLMRUST_PROXY_ADDR="127.0.0.1:3000"
cargo run --example proxy_server --features proxy
```

Call it with the **OpenAI** Chat Completions API:

```bash
curl http://localhost:3000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer some-shared-secret" \
  -d '{
    "model": "deepseek/deepseek-chat",
    "messages": [{"role": "user", "content": "Hello!"}]
  }'
```

The same server also exposes the **Anthropic** Messages API, so Anthropic-only clients (such as tools built for Claude) work unchanged — even when the backend is OpenAI:

```bash
curl http://localhost:3000/v1/messages \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer some-shared-secret" \
  -d '{
    "model": "openai/gpt-4o",
    "messages": [{"role": "user", "content": "Hello!"}],
    "max_tokens": 256
  }'
```

> **Security note:** Without `LLMRUST_PROXY_KEY` set, the proxy has no authentication and binds only to the loopback interface by default. Run it on localhost or behind a reverse proxy. With `LLMRUST_PROXY_KEY` set, every request must include an `Authorization: Bearer <key>` header; the token is compared in constant time.
>
> **CORS:** The proxy sends **no CORS allow-origin header by default** (SPCC §7.1). To allow browser access from specific origins, wrap the `Router` with a restrictive `CorsLayer` (see `docs/CAPABILITIES.md` / the `router()` docs). `Access-Control-Allow-Origin: *` is only permitted on the authenticated router with explicit risk acceptance — never the default.
>
> The proxy follows OpenAI chat-completions request conventions, including `stop` as either a string or an array, and returns JSON error bodies for malformed requests. Stream errors are surfaced to clients as error events rather than being silently converted to successful completions.
> Streaming responses use OpenAI-style SSE chunks, including a single initial `assistant` role delta. When `stream_options.include_usage` is `true`, usage-only chunks use empty `choices`.

### Proxy model names

The proxy routes requests by the `model` string. Use the same provider-prefixed model names as the client API, for example:

- `openai/gpt-4o`
- `anthropic/claude-3-5-sonnet-latest`
- `gemini/gemini-1.5-pro`
- `ollama/llama3.2`

The prefix selects the provider. The remaining model name is sent to the provider after llmrust resolves the route.

## Logging

`llmrust` emits structured [`tracing`](https://docs.rs/tracing) events for provider registration, request lifecycle (including the convenience `chat` / `stream` APIs), proxy requests, retries, router failover, and upstream API errors. As a library, it does not install a global subscriber; your application decides how logs are collected.

```toml
[dependencies]
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
```

```rust
tracing_subscriber::fmt()
    .with_env_filter("llmrust=debug")
    .init();
```

llmrust tracing events do not include API keys, prompt content, response text, request bodies, tool arguments, image data, or full URLs. When helpful, logs use counts and lengths such as `message_count`, `tool_count`, `data_len`, or `url_len`. Operational metadata like `provider`, `model`, HTTP `status`, retry `attempt`, and router `group` are always recorded.

## Provider Configuration

### OpenAI

```rust
llm.set_openai("sk-...").await;
// Custom base URL (Azure, local proxy, etc.)
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

### Ollama (Local Models)

```rust
llm.set_ollama(None).await;  // Default: http://localhost:11434
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

## Error Handling

```rust
use llmrust::{LmrsClient, LlmError};

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

We have not yet published formal benchmarks. The library adds a thin async layer on top of `reqwest`, so overhead versus calling the HTTP API directly should be a few hundred microseconds per request at most.

## Comparison

### vs. Python LiteLLM

| Feature | Python LiteLLM | llmrust |
|---|---|---|
| Startup time | ~hundreds ms (interpreter) | compiled binary, near-instant |
| Memory | Python runtime dependent | single static binary |
| Concurrency | asyncio | tokio (native) |
| Deployment | Python + venv | Single binary |
| Type safety | Runtime | Compile-time |
| Providers | 100+ | 7 (growing) |

### vs. other Rust crates

| | llmrust | genai | rig | llm / rllm |
|---|---|---|---|---|
| Focus | Lean unified client + proxy | Broad unified client | Agent framework | Client + extras (TTS/STT/vision) |
| Providers | 7 (growing) | 25+ | several | many |
| Built-in proxy server | ✅ OpenAI **+** Anthropic | ➖ | ➖ | OpenAI REST only |
| Normalized cross-provider logprobs | ✅ (incl. Gemini) | ➖ | ➖ | ➖ |
| Default dependencies | minimal (`default = []`) | moderate | heavy (framework) | moderate+ |
| Extra scope | embeddings | client only | agents / RAG | embeddings / vision / audio |

> Provider counts and feature sets for other crates move fast — treat this as a positioning sketch, not a benchmark. Pick the tool that matches your needs.

## Roadmap

- [x] Core providers (OpenAI, Anthropic, DeepSeek)
- [x] Streaming support
- [x] Google Gemini, Ollama, Moonshot, OpenRouter
- [x] HTTP proxy server (OpenAI + Anthropic protocols)
- [x] Retry logic
- [x] Tool-use / Function calling (OpenAI-compatible, Anthropic, Gemini; non-streaming)
- [x] JSON mode & sampling parameters (OpenAI-compatible providers)
- [x] Streaming tool calls (reconstruct tool calls from streamed chunks)
- [x] Embeddings API (OpenAI-compatible + Ollama, proxy `/v1/embeddings`)
- [ ] Batch API
- [ ] Rate limiting
- [ ] More providers (Cohere, Mistral, Groq, etc.)

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Contributing

Contributions welcome — from humans, AI coding agents, and human-agent teams.

Please read [`CONTRIBUTING.md`](CONTRIBUTING.md) for the full guide, and [`AGENTS.md`](AGENTS.md) if you are an AI agent.

```bash
cargo build --all-targets --all-features
cargo test
cargo test --all-features
cargo clippy --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
cargo fmt --check
```
