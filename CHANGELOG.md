# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
