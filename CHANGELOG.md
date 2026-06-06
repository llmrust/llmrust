# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
- `ChatRequest::from_messages` / `ChatRequest::with_messages` constructors for building a request from a prepared message list.
- `CHANGELOG.md`.

### Changed

- Anthropic and Google Gemini HTTP clients now use explicit request (120s) and connect (30s) timeouts, matching the OpenAI-compatible client, so a stalled connection can no longer hang a call indefinitely. The Ollama client enforces only a connection timeout, since local generation can legitimately be long-running.
- Google Gemini now passes the API key via the `x-goog-api-key` header instead of the URL query string, avoiding key leakage in request logs.
- README (English and 中文) provider/feature matrix now reflects actual per-provider capabilities: tool calling is supported on OpenAI-compatible, Anthropic, and Gemini providers (non-streaming); JSON mode and the extended sampling parameters remain OpenAI-compatible only.

### Fixed

- Anthropic responses containing non-text content blocks (e.g. `tool_use`) no longer fail to deserialize; text blocks are concatenated and other blocks are skipped.
- Google Gemini responses whose parts carry a `functionCall` (and no `text`) no longer fail to deserialize; part `text` is now optional.
- Ollama streaming now reassembles network chunks through the shared line reader, fixing dropped tokens and corrupted multi-byte UTF-8 (CJK / emoji) when a JSON line or character spans a chunk boundary.
- Ollama token-usage totals use saturating addition to avoid a potential debug-build overflow panic.

### Removed

- Stray `test.txt` from the repository root.

### Known limitations

- Tool calls are surfaced from the non-streaming `chat` path only. The streaming
  path (`stream`) emits text deltas and `finish_reason`, but does not yet
  reconstruct tool calls from streamed chunks for any provider.
