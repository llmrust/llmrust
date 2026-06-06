# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `ChatRequest::from_messages` / `ChatRequest::with_messages` constructors for building a request from a prepared message list.
- `CHANGELOG.md`.

### Changed

- Anthropic and Google Gemini HTTP clients now use explicit request (120s) and connect (30s) timeouts, matching the OpenAI-compatible client, so a stalled connection can no longer hang a call indefinitely. The Ollama client enforces only a connection timeout, since local generation can legitimately be long-running.
- Google Gemini now passes the API key via the `x-goog-api-key` header instead of the URL query string, avoiding key leakage in request logs.
- README (English and 中文) provider/feature matrix now reflects actual per-provider capabilities: tool calling and JSON mode are currently available on the OpenAI-compatible providers only.

### Fixed

- Anthropic responses containing non-text content blocks (e.g. `tool_use`) no longer fail to deserialize; text blocks are concatenated and other blocks are skipped.
- Ollama streaming now reassembles network chunks through the shared line reader, fixing dropped tokens and corrupted multi-byte UTF-8 (CJK / emoji) when a JSON line or character spans a chunk boundary.
- Ollama token-usage totals use saturating addition to avoid a potential debug-build overflow panic.

### Removed

- Stray `test.txt` from the repository root.
