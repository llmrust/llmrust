# Project Map

A structural guide to the llmrust codebase. Read this first if you're new to the project (human or agent).

## Directory layout

```
llmrust/
├── Cargo.toml              # crate metadata, feature flags (default = [], proxy)
├── src/
│   ├── lib.rs              # LmrsClient: unified client, env auto-detect, model routing
│   ├── prelude.rs          # Convenience glob imports
│   ├── types.rs            # Shared types: Message, ChatRequest, ChatResponse, Tool, etc.
│   ├── router.rs           # Multi-deployment routing with failover + per-group round-robin (RTR-001)
│   ├── pricing.rs          # ModelPricing + Usage::estimated_cost (token cost estimation)
│   ├── providers/          # Provider implementations
│   │   ├── mod.rs          # Provider trait, ProviderConfig, LlmError
│   │   ├── compat.rs       # OpenAiCompatibleProvider (shared OpenAI-protocol logic)
│   │   ├── openai.rs       # OpenAIProvider (wraps compat)
│   │   ├── deepseek.rs     # DeepSeekProvider (wraps compat)
│   │   ├── moonshot.rs     # MoonshotProvider (wraps compat)
│   │   ├── openrouter.rs   # OpenRouterProvider (wraps compat, adds referer headers)
│   │   ├── anthropic.rs    # Native Anthropic Messages API
│   │   ├── google.rs       # Native Gemini generateContent API
│   │   ├── ollama.rs       # Native Ollama /api/chat + /api/embed
│   │   ├── retry.rs        # RetryProvider: exponential backoff decorator
│   │   ├── stream_util.rs  # Line-streaming utility for SSE/NDJSON
│   │   └── http.rs         # Shared reqwest client builder
│   └── proxy/              # HTTP proxy server (feature = "proxy")
│       ├── mod.rs          # Axum server, OpenAI /v1/chat/completions + /v1/embeddings
│       └── anthropic_proxy.rs  # Anthropic /v1/messages handler + format conversion
├── examples/               # Runnable examples
│   └── README.md           # Example index
├── tests/                  # Integration tests
├── docs/                   # Documentation
│   ├── PROJECT_MAP.md      # This file
│   ├── CAPABILITIES.md     # Per-provider feature matrix
│   ├── CONTRACTS.md        # Semantic contracts
│   └── architecture/       # Decomposition plans — 0.1.4+ candidate plans, NOT yet implemented
│       ├── PROXY-DECOMPOSITION.md  # Proxy hot-spot split plan (future work, not applied)
│       └── CORE-DECOMPOSITION.md   # Core hot-spot split plan (future work, not applied)
├── AGENT_MANIFESTO.md      # Project philosophy
├── AGENTS.md               # Instructions for AI coding agents
├── CONTRIBUTING.md         # Contribution guide
├── llmrust.capabilities.json  # Machine-readable capability metadata
├── SECURITY.md             # Security guidance
└── RELEASE_CHECKLIST.md    # Release process
```

## Request flow

```
User code
  │
  ▼
LmrsClient::chat("openai/gpt-4o", "Hello")
  │
  ├─ parse_model("openai/gpt-4o") → ("openai", "gpt-4o")
  │
  ├─ get_provider("openai") → Arc<dyn Provider>
  │
  └─ provider.chat(&ChatRequest { model: "gpt-4o", messages: [...], ... })
       │
       ├─ OpenAiCompatibleProvider → POST /v1/chat/completions (SSE for stream)
       ├─ AnthropicProvider       → POST /v1/messages (SSE, different format)
       ├─ GoogleProvider          → POST /v1/models/{model}:generateContent
       └─ OllamaProvider          → POST /api/chat (NDJSON)

Embeddings flow:

LmrsClient::embed("openai/text-embedding-3-small", "hello")
  │
  ├─ parse_model("openai/text-embedding-3-small") → ("openai", "text-embedding-3-small")
  ├─ get_provider("openai") → Arc<dyn Provider>
  └─ provider.embed(&EmbeddingRequest { model: "text-embedding-3-small", ... })
       │
       ├─ OpenAiCompatibleProvider → POST /v1/embeddings
       └─ OllamaProvider          → POST /api/embed
```

## Proxy request flow

```
Client SDK (OpenAI format)          Client SDK (Anthropic format)
  │                                   │
  │ POST /v1/chat/completions         │ POST /v1/messages
  │ POST /v1/embeddings               │
  ▼                                   ▼
proxy::mod.rs                     proxy::anthropic_proxy.rs
  │                                   │
  ├─ deserialize OpenAI JSON          ├─ deserialize Anthropic JSON
  ├─ convert to ChatRequest           ├─ convert to ChatRequest
  │  / EmbeddingRequest               │
  ├─ parse provider from model        ├─ parse provider from model
  │                                   │
  └──────────────┬───────────────────-┘
                 ▼
         provider.chat() / provider.stream() / provider.embed()
                 │
                 ▼
         format response back to protocol-specific JSON/SSE
```

## Key architectural decisions

1. **`default = []`**: No dependencies unless you ask. The proxy feature pulls in `axum`; the base client is pure `reqwest` + `tokio`.
2. **Provider trait is minimal**: Required `chat()` and `stream()`, plus optional default `embed()`. Every provider implements chat/stream; providers that support embeddings override `embed()`; others rely on the default `Unsupported`.
3. **OpenAI-compatible providers share code**: `OpenAiCompatibleProvider` in `compat.rs` handles OpenAI, DeepSeek, Moonshot, and OpenRouter. Each is a thin wrapper setting the base URL.
4. **Anthropic and Gemini are native**: They don't go through OpenAI-compatible shims. They implement the provider's actual wire format.
5. **Stream errors are surfaced**: Malformed stream data returns `LlmError::Parse`, not silently skipped.
6. **`n` policy**: Direct calls warn on `n > 1`. The proxy rejects `n != 1`. This prevents billing surprises.
7. **Logging is sanitized**: API keys, prompts, responses, tool args, embedding vectors, embedding input text, and full URLs are never logged.

## Where to add things

| What to add | Where to put it |
|-------------|-----------------|
| New provider (OpenAI-protocol) | `src/providers/<name>.rs` (wrap `OpenAiCompatibleProvider`) |
| New provider (custom protocol) | `src/providers/<name>.rs` (implement `Provider` directly) |
| New shared type | `src/types.rs` |
| New LmrsClient method | `src/lib.rs` |
| New embedding type (shared) | `src/types.rs` |
| New proxy route/feature | `src/proxy/mod.rs` or `src/proxy/anthropic_proxy.rs` |
| New example | `examples/<name>.rs` + add to `examples/README.md` |
| New test (unit) | Inline `#[cfg(test)] mod tests` in the relevant source file |
| New test (integration) | `tests/integration_tests.rs` |
