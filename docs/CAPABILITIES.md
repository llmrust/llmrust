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
| **embeddings** | ✅ | ✅ | ✅ | ✅ | ➖ | ➖ | ➖ |

> ✅ for embeddings means llmrust implements OpenAI-compatible `/embeddings` transport. Actual upstream provider/model support may vary.

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

### Anthropic
- No `seed`, `presence_penalty`, `frequency_penalty`, `logprobs`, `n`, `service_tier`, `store`, `metadata`, `user`.
- Image inputs are accepted via base64 `data:` URLs only in the Anthropic content-block format.
- `response_format` / JSON mode is not a first-class concept in the Messages API.

### Gemini
- Remote `http(s)` image URLs are skipped with a warning. Only `data:` URLs work.
- No `seed`, `presence_penalty`, `frequency_penalty`, `parallel_tool_calls`, `service_tier`, `store`, `metadata`.
