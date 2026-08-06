# Capabilities Matrix

Which features each provider supports, and how they map across providers.

> **Note on capabilities**: "Support" means llmrust maps the field or protocol. Actual upstream model support may vary by model. For example, llmrust sends `response_format` to DeepSeek and Moonshot, but some of their models may ignore or reject it. A ✅ in this matrix does not guarantee every model under that provider supports the feature.

## Provider support matrix

| Capability | OpenAI | DeepSeek | Moonshot | OpenRouter | Anthropic | Gemini | Ollama |
|------------|--------|----------|----------|------------|-----------|--------|--------|
| **chat** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **stream** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **tool calling (chat)** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ➖ |
| **tool calling (stream)** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ➖ |
| **JSON mode** | ✅ | ✅ | ✅ | ✅ | ➖ (n/a) | ✅ | ➖ |
| **JSON schema** | ✅ | ➖ | ➖ | ➖ | ➖ (n/a) | ✅ | ➖ |
| **logprobs** | ✅ | ➖ | ➖ | ➖ | ➖ (n/a) | ✅ | ➖ |
| **image input** | ✅ | ➖ | ➖ | ✅ | ✅ | ✅ (data: URL only) | ➖ |
| **system message** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **multi-turn** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **custom base URL** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **embeddings** | ✅ | ✅ | ✅ | ✅ | ➖ | ➖ | ✅ |
| **reasoning (request + stream + usage)** | ✅ | ➖ | ➖ | ➖ | ✅ | ✅ | ➖ |

> ✅ for embeddings means llmrust implements an embeddings transport for that provider. OpenAI, DeepSeek, Moonshot, and OpenRouter use OpenAI-compatible `/embeddings`; Ollama uses native `/api/embed`. Actual upstream/local model support may vary.

> **Reasoning semantics**: ✅ for reasoning means llmrust maps the reasoning contract on the **streaming** path (request field + `StreamChunk.thinking` deltas + `thinking_done` + usage mapping), verified by local fixtures (2026-08-02). Non-stream `chat` and the `stream_collect*` aggregates **fail with `LlmError::Unsupported`** for these providers (reasoning cannot be carried losslessly in `ChatResponse` / aggregated text); `➖` providers reject reasoning before any network call. Real upstream E2E verification belongs to E2E-001 (SPCC §8.2, REA-001 §3).

## Sampling parameter support

| Parameter | OpenAI-compat | Anthropic | Gemini | Ollama |
|-----------|---------------|-----------|--------|--------|
| `temperature` | ✅ | ✅ | ✅ | ✅ |
| `max_tokens` | ✅ | ✅ | ✅ | ✅ |
| `top_p` | ✅ | ✅ | ✅ | ➖ |
| `stop` | ✅ (array) | ✅ (sequences) | ✅ | ➖ |
| `seed` | ✅ | ➖ | ➖ | ➖ |
| `presence_penalty` | ✅ | ➖ | ➖ | ➖ |
| `frequency_penalty` | ✅ | ➖ | ➖ | ➖ |
| `logprobs` | ✅ | ➖ | ✅ | ➖ |
| `top_logprobs` | ✅ | ➖ | ✅ | ➖ |
| `response_format` | ✅ | ✅ | ✅ | ➖ |
| `parallel_tool_calls` | ✅ | ➖ | ➖ | ➖ |
| `service_tier` | ✅ | ➖ | ➖ | ➖ |
| `store` | ✅ | ➖ | ➖ | ➖ |
| `metadata` | ✅ | ➖ | ➖ | ➖ |
| `user` | ✅ | ➖ | ➖ | ➖ |

## Thinking / reasoning control

`ChatRequest.thinking` (type `ThinkingConfig`) and `ChatRequest::with_thinking` were introduced
in **0.1.2** and formally **adopted as the 0.1.3 freeze baseline** (adjudication **D7**).

Request-side reasoning is mapped per provider (REA-002 / REA-003 / REA-004G / REA-004O):

- **Anthropic** (`implemented`): `Enabled` → `thinking: {type: "enabled", budget_tokens}` on
  the wire; `Disabled` → omitted. Streamed `thinking_delta` / `signature_delta` /
  `redacted_thinking` surface via `StreamChunk.thinking`; cache/reasoning usage is translated.
- **OpenAI** (`implemented`): `Enabled{budget_tokens: None}` → `reasoning_effort: "medium"` on
  the streaming path; `budget_tokens: Some(_)` is rejected (no OpenAI equivalent). Streamed
  `reasoning` / `reasoning_content` map to `StreamChunk.thinking`; `reasoning_tokens` usage is
  translated.
- **Gemini** (`implemented`): `Enabled` → `thinkingConfig` on the wire (`thinkingBudget` is
  optional upstream and omitted losslessly; `includeThoughts` is always requested when enabled).
  Streamed `thought` parts surface via `StreamChunk.thinking`; `usageMetadata.thoughtsTokenCount`
  maps to `Usage.reasoning_tokens`.
- **DeepSeek / Moonshot / OpenRouter** (`unsupported`): no verified official reasoning/cache
  fields; setting thinking fails with `LlmError::Unsupported` before any network call.
- **Ollama** (`unsupported`): the wire offers only `options.think` (bool/level) with no lossless
  mapping for `ThinkingConfig.budget_tokens`; setting thinking fails with `LlmError::Unsupported`
  before any network call.

Non-stream `chat` and the `stream_collect` / `stream_collect_full` aggregates fail with
`LlmError::Unsupported` whenever reasoning is present — `ChatResponse` and the aggregate return
values cannot carry reasoning losslessly (REA-001 §1.4, STR-003); callers must consume the raw
`stream()` for reasoning streams. Capability status is verified at local-fixture level
(2026-08-02); real upstream E2E verification belongs to E2E-001.

## Error normalization

llmrust normalizes errors into `LlmError`:

| Source | `LlmError` variant |
|--------|-------------------|
| Network failure, TLS error, timeout | `LlmError::Http(reqwest::Error)` |
| Upstream 4xx/5xx with JSON body | `LlmError::Api { status, message }` |
| Malformed stream data | `LlmError::Stream(String)` |
| JSON parse failure | `LlmError::Parse(String)` |
| Unregistered provider name | `LlmError::UnknownProvider(String)` |
| Unsupported provider feature | `LlmError::Unsupported { feature, message }` |

### Proxy error mapping

The proxy maps `LlmError` to HTTP status codes and OpenAI-style error bodies:

| `LlmError` | HTTP status | Error type |
|------------|-------------|------------|
| `UnknownProvider` | 404 | `invalid_request_error` |
| `Parse` (invalid JSON) | 400 | `invalid_request_error` |
| `Api { status: 401 }` | 401 | `authentication_error` |
| `Api { status: 429 }` | 429 | `rate_limit_error` |
| Other `Api` / `Http` | 502 | `api_error` |
| `Stream` error | 502 | `api_error` |

### Responses API proxy endpoint

The proxy exposes `POST /v1/responses` (OpenAI Responses API wire protocol) in addition to
`/v1/chat/completions` and `/v1/messages`. Responses-native clients (e.g. Codex CLI) can reach any
registered backend through automatic conversion:

- **Request conversion**: `input` (string or array of items) and `instructions` map onto llmrust
  messages (`instructions` → leading system message); `input_text` / `input_image` content parts map
  onto text/image content; Responses-shaped tools (flat `name`/`description`/`parameters`) are
  normalized to llmrust's nested tool shape; `tool_choice` and `reasoning.effort` are mapped.
- **Non-streaming response**: a Responses object (`{object: "response", status, model, output,
  usage}`) with message and `function_call` output items.
- **Streaming response**: SSE event sequence `response.created` → `response.output_item.added` →
  `response.content_part.added` → `response.output_text.delta`* →
  `response.output_item.done` → `response.completed` → `data: [DONE]`. Tool calls surface as
  `function_call` output items plus `response.function_call_arguments.delta` events. Delta payloads
  carry `item_id` / `output_index` / `content_index` for client-side assembly.
- **Auth / body limits**: same bearer-token auth and 2 MiB body limit as the other proxy endpoints.
- **Errors**: upstream failures map to a `{type: "error", error: {message, type}}` body (400 for
  invalid requests / unknown providers, upstream HTTP status for API errors, 502 otherwise).

## Streaming behavior

For all providers, streaming follows these rules:

1. Stream emits incremental `StreamChunk` items with `delta` text.
2. The terminal chunk has `done: true` and carries `finish_reason`, `usage`, and `tool_calls` (if applicable).
3. Parse errors in the stream are surfaced as `Err(LlmError::Parse(...))` chunks — never silently dropped.
4. Upstream API errors mid-stream are surfaced as `Err(LlmError::Api{...})` or `Err(LlmError::Stream(...))`.

### Provider-specific stream formats

| Provider | Wire format | SSE event structure |
|----------|-------------|---------------------|
| OpenAI-compat | SSE (`text/event-stream`) | `data: {"id":"...","object":"chat.completion.chunk","choices":[...]}` + `data: [DONE]` |
| Anthropic | SSE | `event: content_block_start/delta/ping`, `event: message_start/delta/stop` |
| Gemini | SSE (Gemini format) | `data: {"candidates":[...]}` with `text` deltas and `functionCall` parts |
| Ollama | NDJSON | `{"model":"...","message":{"content":"..."},"done":false}` per line |

## Things each provider does NOT support

This section exists to help AI agents avoid wasting time on impossible features.

### Ollama
- No tool calling (Ollama models vary widely in function-calling support; not worth the abstraction overhead).
- No JSON mode (same reason).
- No `top_p`, `stop`, `seed`, penalties, `n`, or request metadata fields.
- Image content is flattened to text (local models may not support vision).
- No reasoning: setting `ChatRequest.thinking` to a non-`Disabled` value fails with
  `LlmError::Unsupported` before any network call (no lossless wire mapping for
  `ThinkingConfig.budget_tokens`; REA-004O).

### Anthropic
- No `seed`, `presence_penalty`, `frequency_penalty`, `logprobs`, `n`, `service_tier`, `store`, `metadata`, `user`.
- Image inputs are accepted via base64 `data:` URLs only in the Anthropic content-block format.
- `response_format` / JSON mode is not a first-class concept in the Messages API.

### Gemini
- Remote `http(s)` image URLs are skipped with a warning. Only `data:` URLs work.
- No `seed`, `presence_penalty`, `frequency_penalty`, `parallel_tool_calls`, `service_tier`, `store`, `metadata`.
