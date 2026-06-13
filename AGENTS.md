# Instructions for AI Coding Agents

You are working on **llmrust**, a Rust LLM library designed for human-agent collaboration.
The codebase intentionally prioritizes clarity so that AI agents can read it as documentation.

## Read first (in order)

1. `AGENT_MANIFESTO.md` — project philosophy and agent-contribution principles
2. `docs/PROJECT_MAP.md` — architecture map and module boundaries
3. `docs/CAPABILITIES.md` — per-provider feature matrix
4. `docs/CONTRACTS.md` — semantic contracts every provider must honor
5. `src/types.rs` — the shared type system (ChatRequest, ChatResponse, EmbeddingRequest, StreamChunk, Tool, etc.)
6. `src/providers/mod.rs` — the `Provider` trait and error types
7. The relevant provider file under `src/providers/` for the provider you intend to change
8. `examples/README.md` — which example demonstrates what

## Core rules

These rules are non-negotiable. Violating them will likely cause a PR to be rejected.

### API surface

- Model names use the `provider/model` format (e.g. `openai/gpt-4o`). Never change the separator.
- The default feature set (`default = []`) must stay lightweight. Do not add dependencies to default features unless strictly required.
- `ChatRequest` is `#[non_exhaustive]`. Add new fields but never remove or rename existing ones.
- `FinishReason` variants are cross-provider. `EmbeddingRequest` fields are cross-provider; provider-specific knobs belong in `extra`. New variants or fields must have a clear, documented meaning.

### Providers

- Every provider must implement both `chat` and `stream` from the `Provider` trait.
- `embed()` has a default Unsupported implementation. If the upstream supports embeddings, override `embed()`; otherwise rely on the default and document it.
- Streaming must surface parse errors and upstream errors instead of silently dropping malformed data.
- When adding a new provider:
  1. Create `src/providers/<name>.rs`
  2. Implement `Provider` for it (chat + stream required; embed optional)
  3. Add a `set_<name>()` method to `LmrsClient` in `src/lib.rs`
  4. Add it to `LmrsClient::from_env()` with appropriate env-var detection
  5. Update `docs/CAPABILITIES.md` (embed row if supported)
  6. Update `llmrust.capabilities.json`
  7. Update README provider table
  8. Add tests covering chat, stream, embed (if supported, or Unsupported contract if not), tool calls (if supported), and error handling

### Proxy

- The proxy exposes: OpenAI `/v1/chat/completions` + `/v1/embeddings`, Anthropic `/v1/messages`.
- Do not change proxy wire semantics without corresponding tests in `src/proxy/mod.rs`.
- The proxy accepts missing `n` or `n = 1`, and rejects `n = 0` or `n > 1`.
- `/v1/embeddings` accepts string or string-array input; rejects base64 encoding_format.
- Proxy authentication uses constant-time token comparison.
- Stream errors must be surfaced to clients as error SSE events, not silently converted to successful completions.
- `LlmError::Unsupported` maps to 400 on OpenAI-compatible endpoints (not 502).

### Logging

- **Never log**: API keys, prompt content, response text, request bodies, tool arguments, image data, embedding vectors, embedding input text, full URLs.
- When location information is useful, use metadata fields like `message_count`, `tool_count`, `data_len`, `input_count`, `url_len`, `error_kind`.
- llmrust emits `tracing` events but does not install a global subscriber.

### Sensitive data

- API keys must never appear in debug output, error messages, or logs. `ProviderConfig` debug output masks keys.
- Image data must not be logged at any level.

## Pull request expectations

Every PR must include:

- **Summary**: what was changed and why
- **Behavior changed**: which external behavior differs (or "none" if purely internal)
- **Tests added**: what tests cover the change
- **Commands run**: at minimum `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`, `cargo test --all-features`
- **AI agent contribution** (see PR template checkbox):
  - Whether AI assistance was used
  - Agent / model name if applicable

## Common pitfalls

- Do not use `unwrap()` on network or parse results; use `?` or proper error handling.
- Do not change the `Provider` trait signature without updating all provider implementations, including the shared `OpenAiCompatibleProvider` and `RetryProvider` (which must delegate `embed()`).
- Do not assume that all providers support `temperature`, `top_p`, or `max_tokens`. Check `docs/CAPABILITIES.md`.
- Do not add heavy dependencies (e.g. `serde_yaml`, large framework crates) without discussion.
- Do not make remote image fetching implicit.
- The Ollama provider intentionally has no overall request timeout (local generation can be long-running).
- Google Gemini passes its API key via the `x-goog-api-key` header — not in the URL.
- Ollama embeddings use `/api/embed`, not `/api/embeddings`. Ollama does not support the `user` field.
