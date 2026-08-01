# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- API freeze gate (Track ① of API-002): a `cargo-semver-checks` CI job comparing the
  public API against the `0.1.2` crates.io baseline. Proxy DTOs are exempt from this
  gate — they are classified `UNSTABLE` and feature-gated, so they are not compiled into
  the default-feature build that the gate checks.
- Response-compatibility regression tests freezing the wire shapes of `Usage`,
  `ChatResponse`, and `StreamChunk`, pinning the `None` vs `Some(0)` distinction for the
  `Option<u64>` token counters (`cache_read_tokens`, `cache_write_tokens`,
  `reasoning_tokens`), and round-tripping unknown `finish_reason` values through
  `FinishReason::Other` (the §5.1 wire escape hatch).
- `api_freeze` integration test that consumes `docs/api-inventory.json` and asserts the
  API-001 classification boundaries: `FinishReason`/`ChatResponse` variants and fields are
  frozen (D1/D2), `ThinkingConfig` is `STABLE` but not root-reexported (D3),
  `STABLE-ADDITIVE` symbols require `#[non_exhaustive]`, and the proxy module is
  `UNSTABLE`. Editing the classification to permit a change fails the gate (fail-closed).
- Pricing regression test asserting `Usage::estimated_cost` does not double-count
  cache/reasoning tokens.

### Changed

- Bumped crate version `0.1.1` → `0.1.3` to align with the 0.1.3 development line and to
  enable the semver baseline comparison (current version must be greater than the `0.1.2`
  baseline). `llmrust.capabilities.json` version synced accordingly.
- Anthropic extended thinking is now wired end-to-end on the streaming path (REA-002):
  `ChatRequest.thinking` maps to the native `thinking: {type: "enabled", budget_tokens}`
  parameter; non-stream `chat` with thinking enabled fails with `LlmError::Unsupported`
  before any network call (`ChatResponse` cannot carry reasoning in 0.1.3);
  `Enabled{budget_tokens: None}` is also rejected before the network (the Anthropic API
  requires `budget_tokens`). Streamed `thinking_delta` text is surfaced via
  `StreamChunk.thinking`; `signature_delta` and `redacted_thinking` blocks mark the end of
  the thinking phase via `StreamChunk.thinking_done` (at most once). Usage translation now
  covers prompt-cache and reasoning tokens: `cache_creation_input_tokens` →
  `cache_write_tokens`, `cache_read_input_tokens` → `cache_read_tokens`, and
  `output_tokens_details.thinking_tokens` → `reasoning_tokens`, with `message_start` usage
  merged into the terminal chunk. (REA-002, Refs #130)
- `ThinkingConfig` (enum) and `ChatRequest.thinking` / `ChatRequest::with_thinking` — introduced
  in 0.1.2 and **formally adopted as the 0.1.3 freeze baseline** (adjudication **D7**). This is
  a request-side contract only: no provider implements thinking/reasoning at this time (tracked
  as **E-003**), and this status is documented in `AGENTS.md` and `docs/CAPABILITIES.md`, not
  implied as implemented.
- `LmrsClient::stream()` now enforces the SPCC §6.5/§6.6 single-terminal contract at the
  public boundary through a shared collapse layer (`unify_terminal`, `pub(crate)`): exactly one
  `done = true` chunk is emitted carrying the final `finish_reason` / `usage` / `tool_calls` /
  `thinking_done`; late metadata (e.g. a usage-only chunk arriving after the finish chunk) is
  captured; a missing `done` is synthesized; and an `Err` is never followed by a success
  terminal. The public `StreamChunk` shape is unchanged (API-freeze safe). (STR-001, Refs #116)
- Anthropic streaming now honors the STR-001 single-terminal contract on the provider side.
  Malformed or truncated SSE `data` lines surface as `LlmError::Parse` (previously silently
  dropped), a stream-level `error` event surfaces as `LlmError::Stream` (previously silently
  ignored), and `message_delta` usage is translated into `StreamChunk.usage` (previously
  dropped). Unknown / future event types (`message_stop`, `ping`, `comment`, …) remain ignored,
  and only the event *type* is logged — never event content. Terminal handling (exactly one
  `done = true`, an `Err` never followed by a success terminal) is still guaranteed by the shared
  `unify_terminal` layer. (STR-002A, Refs #119)
- Gemini streaming now honors the STR-001 single-terminal contract on the provider side, mirroring
  the Anthropic fix (STR-002A). Malformed or truncated SSE `data` lines surface as `LlmError::Parse`
  (previously silently dropped), and an in-stream `{"error":{...}}` envelope surfaces as
  `LlmError::Stream` (previously silently swallowed — `GeminiStreamEvent` tolerates unknown fields
  and has no `error` field, so the envelope deserialized to an empty event). The Gemini-native
  `GeminiErrorBody` is reused to detect the envelope (no Anthropic DTO copy). Terminal handling
  (exactly one `done = true`, an `Err` never followed by a success terminal) is still guaranteed by
  the shared `unify_terminal` layer. (STR-002G, Refs #124)

### Fixed

- Retry contract clarification (API-003): `RetryProvider` does **not** retry HTTP `429`
  (rate-limit) responses — only `HTTP 5xx`, network errors, and transient stream errors are
  retried. The previously published `llmrust.capabilities.json` incorrectly listed
  `"429 (rate limit)"` under `retry_on`; this is corrected to match the implementation
  (`should_retry` returns `false` for all 4xx, including `429`). Important distinction from
  routing: the **Router** *does* fail over on `429` (treats it as transient and switches
  deployment), but that is a separate mechanism from `RetryProvider`'s retry policy and is
  unchanged.
- `n > 1` advisory is now emitted once per `(provider, n)` for the process lifetime instead of
  being repeated on every `RetryProvider` retry attempt (E-002). Pure log-noise reduction; no
  functional change.

## [0.1.1] - 2026-06-16

### Added

- `ModelPricing` and `Usage::estimated_cost` for estimating request cost in US dollars from token usage, using per-1,000-token prompt and completion rates. Pure, additive utility with no new dependencies.
- Ollama embeddings provider implementation using native `/api/embed`.
- OpenAI-compatible proxy `/v1/embeddings` endpoint for float embeddings.
- OpenAI-compatible embeddings provider implementation for OpenAI, DeepSeek, Moonshot, and OpenRouter wrappers.
- Embeddings API foundation: `EmbeddingRequest`, `EmbeddingResponse`, `Embedding`, `EmbeddingUsage` types, `Provider::embed` default method, and `LmrsClient` embed routing methods.
- Embeddings quickstart example demonstrating `LmrsClient::embed` and `embed_batch`.
- Opt-in passive cooldown for Router deployments after failoverable errors, with fail-open on all-cooling groups.
- Local contract tests for client model routing, stream error propagation, stream metadata collection (`stream_collect_full`), and proxy `n` policy.
- CI validation for agent-facing docs, examples index, and capability metadata (`tests/agent_docs_validation.rs`).
- **Agent-native codebase**: repositioned llmrust as an AI-agent-friendly infrastructure library.
  - `AGENT_MANIFESTO.md` — project philosophy for human-agent collaboration.
  - `AGENTS.md` — operational instructions for AI coding agents.
  - `CONTRIBUTING.md` — full contribution guide (human + agent).
  - `docs/PROJECT_MAP.md` — architecture map and module boundaries.
  - `docs/CAPABILITIES.md` — per-provider feature matrix with explicit unsupported flags.
  - `docs/CONTRACTS.md` — semantic contracts for providers, proxy, and client.
  - `llmrust.capabilities.json` — machine-readable capability metadata.
  - `examples/README.md` — example index with quick-run instructions.
  - `.github/pull_request_template.md` — PR template with AI agent contribution section.

### Changed

- `EmbeddingRequest` is now marked `#[non_exhaustive]` (matching `ChatRequest`), so future optional fields can be added without a breaking change. Build it with `EmbeddingRequest::new` / `EmbeddingRequest::batch` and the builder methods rather than struct-literal syntax from outside the crate.
- Aligned docs, README, PROJECT_MAP, AGENTS.md, and capabilities.json with embeddings support for 0.1.1.
- Replaced Python agent-doc validation script with a Rust integration test to keep the repository toolchain Rust-native.
- Tightened agent-doc validation (`tests/agent_docs_validation.rs`): the capability metadata version is now checked against the crate version (`CARGO_PKG_VERSION`), and the proxy embeddings endpoint (`POST /v1/embeddings`) must be listed, so release-metadata drift is caught in CI.
- README (English and 中文) now leads with the human-agent collaboration narrative, with the original provider-unification message as secondary description.
- README contributing sections now point to `CONTRIBUTING.md` and `AGENTS.md`.

### Fixed

- Corrected the `version` field in `llmrust.capabilities.json`, which had been left at `0.1.0` while the crate had already moved to `0.1.1`.

## [0.1.0] - 2026-06-11

### Added

- Native tool calling for Anthropic Claude and Google Gemini (non-streaming `chat`):
  - Requests map llmrust tools / `tool_choice` to each provider's native shape
    (`input_schema` + `tool_choice` for Claude; `functionDeclarations` +
    `toolConfig` for Gemini).
  - Responses parse `tool_use` blocks (Claude) and `functionCall` parts
    (Gemini) into `ChatResponse.tool_calls`, with `finish_reason` normalized to
    `tool_calls`.
  - Multi-turn tool loops round-trip correctly: assistant tool calls and tool
    results are re-encoded as Claude `tool_use` / `tool_result` blocks and
    Gemini `functionCall` / `functionResponse` parts.
- Streaming tool calls: the `stream` path now reconstructs tool calls from
  streamed chunks across the OpenAI-compatible providers (OpenAI, DeepSeek,
  Moonshot, OpenRouter), Anthropic Claude, and Google Gemini, surfacing them as
  `StreamChunk.tool_calls` on the terminal chunk with `finish_reason`
  normalized to `tool_calls`. Claude reassembles `tool_use` blocks from
  `content_block_start` + `input_json_delta` fragments; Gemini collects
  streamed `functionCall` parts.
- `ChatRequest::from_messages` / `ChatRequest::with_messages` constructors for building a request from a prepared message list.
- `ChatRequest` builder support for OpenAI-compatible request metadata/control
  fields: `parallel_tool_calls`, `service_tier`, `store`, `metadata`, and
  `user`. The built-in OpenAI-compatible provider and proxy forward these
  fields when set.
- Logging documentation for the library's `tracing` events, including subscriber setup and the sensitive-data boundary.
- MSRV 1.86 and `cargo publish --dry-run` checks in CI.
- `SECURITY.md` with proxy deployment and logging guidance.
- `RELEASE_CHECKLIST.md` for v0.1.0 publishing.
- `CHANGELOG.md`.

### Changed

- Convenience `LmrsClient::chat` / `stream` calls now share the same structured
  request lifecycle tracing as `chat_with` / `stream_with`, and
  `ProviderConfig` debug output masks configured base URLs as well as API keys.
- Provider registration now emits consistent `tracing` debug events without logging API keys or raw base URLs.
- Anthropic and Google Gemini HTTP clients now use explicit request (120s) and connect (30s) timeouts, matching the OpenAI-compatible client, so a stalled connection can no longer hang a call indefinitely. The Ollama client enforces only a connection timeout, since local generation can legitimately be long-running.
- Google Gemini now passes the API key via the `x-goog-api-key` header instead of the URL query string, avoiding key leakage in request logs.
- README (English and 中文) provider/feature matrix now reflects actual per-provider capabilities: tool calling is supported on OpenAI-compatible, Anthropic, and Gemini providers (both non-streaming `chat` and streaming `stream`); JSON mode and extended sampling parameters are supported by OpenAI-compatible providers and mapped where Gemini has native equivalents.
- CI now builds, tests, lints, and checks rustdoc warnings with `--all-features`, so the optional proxy feature and public documentation are covered.
- Clarified proxy model routing, authentication, and stream error semantics in docs and examples.
- Fixed `router_with_auth` doc comment to match actual constant-time token comparison.

### Fixed

- The OpenAI-compatible proxy now accepts assistant tool-call messages with `content: null`, forwards advanced request fields such as `response_format`, `stop`, `seed`, penalties, and logprob options, and returns tool calls / real `finish_reason` values in non-streaming and streaming responses.
- The OpenAI-compatible proxy now accepts `stop` as either a string or an array,
  rejects empty `messages` locally, returns OpenAI-style JSON error bodies for
  malformed JSON, and maps upstream API failures to more accurate error types
  such as `rate_limit_error` and `authentication_error`.
- OpenAI-compatible proxy streaming chunks now emit the assistant `role` only
  on the first delta and return `choices: []` for usage-only chunks, matching
  OpenAI-style SSE stream conventions more closely.
- OpenAI-compatible proxy streaming now honors
  `stream_options.include_usage`: usage events are emitted only when requested.
- The OpenAI-compatible proxy now accepts legacy `functions` / `function_call`
  request fields and normalizes them to modern `tools` / `tool_choice`.
- OpenAI-compatible provider responses now parse non-streaming
  `choices[].logprobs` into `ChatResponse.logprobs` when reported.
- Public rustdoc no longer links to a private helper, so docs build cleanly when warnings are denied.
- Anthropic responses containing non-text content blocks (e.g. `tool_use`) no longer fail to deserialize; text blocks are concatenated and other blocks are skipped.
- Google Gemini responses whose parts carry a `functionCall` (and no `text`) no longer fail to deserialize; part `text` is now optional.
- Ollama streaming now reassembles network chunks through the shared line reader, fixing dropped tokens and corrupted multi-byte UTF-8 (CJK / emoji) when a JSON line or character spans a chunk boundary.
- Ollama token-usage totals use saturating addition to avoid a potential debug-build overflow panic.
- Fixed release README packaging so crates.io does not depend on excluded local image assets.
- Fixed proxy server example command to include the `proxy` feature.
- Logging no longer includes API keys, prompt content, response text, request bodies, tool arguments, image data, or full URLs. The `truncate_str` utility was removed.
- Anthropic stream block index uses a monotonic counter instead of separate text/tool indices.
- Anthropic proxy stream error events now terminate the stream correctly.
- OpenAI proxy stream errors are now emitted before `[DONE]` via an unfold state machine.
- Malformed provider stream data now returns `LlmError::Parse` instead of being silently skipped.
- The OpenAI-compatible proxy rejects requests where `n != 1` (missing `n` or `n=1` is accepted) to avoid silent upstream billing.

### Removed

- Stray `test.txt` from the repository root.
