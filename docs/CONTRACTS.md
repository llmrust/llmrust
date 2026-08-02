# Semantic Contracts

This document defines the behavioral contracts that every provider, the proxy, and the client must honor. These are not type-level guarantees — they're about runtime semantics. Tests should verify these.

> These contracts are covered by local contract tests in `tests/contract_tests.rs`. Changes to provider, client, proxy, or stream behavior should update both this document and the relevant tests.

## Provider contract

Every implementation of `Provider` must satisfy:

### `chat(&self, req: &ChatRequest) -> Result<ChatResponse>`

1. **Model forwarding**: The `req.model` string is passed to the upstream API as-is. The client has already stripped the provider prefix.
2. **Content extraction**: The response must set `ChatResponse.content` to the full text of the first/primary completion. For providers that return structured content blocks (Anthropic, Gemini), concatenate all text blocks.
3. **Finish reason**: Normalize provider-specific stop reasons to `FinishReason`. Special cases:
   - Anthropic `"tool_use"` → `FinishReason::ToolCalls`
   - Anthropic `"end_turn"` → `FinishReason::EndTurn`
   - Anthropic `"max_tokens"` → `FinishReason::MaxTokens`
   - Anthropic `"stop_sequence"` → `FinishReason::StopSequence`
4. **Tool calls**: If the response contains tool calls, populate `ChatResponse.tool_calls`. Each `ToolCall` must have `id`, `function.name`, and `function.arguments` (JSON string).
5. **Usage**: Populate `ChatResponse.usage` with prompt tokens, completion tokens, and total when the upstream returns them.
6. **Error propagation**: Upstream API errors (4xx, 5xx) must become `LlmError::Api { status, message }`. Network errors become `LlmError::Http`. Never return `Ok(...)` with partial/empty content on error.
7. **Logprobs** (if supported): Populate `ChatResponse.logprobs` with the normalized structure.

### `stream(&self, req: &ChatRequest) -> Result<BoxStream<'static, Result<StreamChunk>>>`

1. **Stream establishment**: The stream must be established (HTTP connection + initial response) before returning `Ok(stream)`. If the upstream rejects the request, return `Err(...)`.
2. **Incremental deltas**: Each `StreamChunk` must carry `delta` text. The final chunk must have `done: true`.
3. **Terminal metadata**: The final chunk must carry `finish_reason`, `usage` (if available), and `tool_calls` (if applicable).
4. **Error in stream**: If the stream encounters a parse error mid-stream, yield `Err(LlmError::Parse(...))`. If the upstream returns an error mid-stream, yield `Err(LlmError::Api{...})` or `Err(LlmError::Stream(...))`.
5. **No silent drops**: Never silently skip malformed data. Never emit `Ok(chunk)` with empty delta and `done: false` as a workaround for parse failures.
6. **Tool call reconstruction** (if supported): Accumulate streamed tool call fragments and emit the complete `tool_calls` on the terminal chunk.
7. **Reasoning deltas**: Providers that map reasoning on the streaming path must surface increments via `StreamChunk.thinking` (additive, appended per chunk) and mark the end with `StreamChunk.thinking_done = Some(true)` at most once on the terminal chunk. Reasoning must never be mixed into `StreamChunk.delta` (REA-001 §1.3). Providers without a lossless wire mapping must reject `Enabled` reasoning before any network call (REA-002/003/004G/004O).

## Proxy contract

### OpenAI `/v1/chat/completions`

1. **Model routing**: Parse `model` as `provider/model`. Return 400 if format is invalid.
2. **n policy**: Accept missing `n` or `n = 1`. Reject `n = 0` or `n > 1` with a 400 error.
3. **Message validation**: Return 400 if `messages` is empty or contains invalid roles.
4. **Legacy function support**: Accept `functions`/`function_call` and normalize to `tools`/`tool_choice`.
5. **Non-streaming response**: Return OpenAI-shaped JSON: `{"id":"chatcmpl-...","object":"chat.completion","choices":[...],"usage":{...}}`.
6. **Streaming response**: Return SSE events: `data: {"id":"...","object":"chat.completion.chunk","choices":[...]}` per chunk, followed by `data: [DONE]`.
7. **Role emission**: Emit `"role":"assistant"` only on the first delta chunk.
8. **Usage chunks**: When `stream_options.include_usage` is true, usage-only chunks use empty `choices: []`.
9. **Error bodies**: Return OpenAI-style JSON errors: `{"error":{"message":"...","type":"...","code":null}}`.
10. **Stream errors**: Emit error as an SSE event with `"error"` in the JSON body, then send `[DONE]`.

### Anthropic `/v1/messages`

1. **Content block format**: Non-streaming responses return `content: [{type: "text", text: "..."}]` blocks.
2. **Stream events**: Return Anthropic SSE events: `message_start`, `content_block_start`, `content_block_delta`, `content_block_stop`, `message_delta`, `message_stop`.
3. **Stop reason**: Map `FinishReason` to Anthropic stop reasons.
4. **Tool use blocks**: Return `content: [{type: "tool_use", ...}]` blocks for tool calls.

### Authentication

1. **No key set**: All requests pass through (no auth).
2. **Key set via `LLMRUST_PROXY_KEY`**: Every request must include `Authorization: Bearer <key>`. Missing/malformed/wrong → 401.
3. **Token comparison**: Use constant-time comparison, not standard string equality.

## Client contract (`LmrsClient`)

1. **Model format**: All `chat`/`stream` methods require `provider/model` format. Parse errors return `LlmError::Parse`.
2. **Provider resolution**: `model` is the model name *after* the `/`. The client sets `req.model` before calling the provider.
3. **Convenience methods**: `chat(model, prompt)` and `stream(model, prompt)` construct a `ChatRequest` with a single user message and delegate to `chat_with`/`stream_with`.
4. **Stream collection**: `stream_collect` returns concatenated text. `stream_collect_full` returns a full `ChatResponse` with `usage`, `tool_calls`, and `finish_reason`. Reasoning streams are not aggregatable: if any chunk carries a non-empty `thinking` delta or `thinking_done == true`, both collectors fail with `LlmError::Unsupported` (feature `reasoning`) and direct the caller to consume the raw `stream()` instead (STR-003).
5. **Retry wrapping**: `with_retry(max_retries)` wraps all registered providers in `RetryProvider`. Retry logic applies exponential backoff and only retries on transient errors (5xx, network errors).

## Router contract

1. **Failover scope**: Router failover only on `should_failover` errors (HTTP, Stream, 5xx API, 429, UnknownProvider). Non-failoverable errors (4xx, Parse) return immediately.
2. **Stream failover**: Failover applies only to the initial stream connection. Mid-stream errors propagate to the caller.
3. **Cooldown is opt-in**: Default Router behavior is unchanged. Calling `Router::with_cooldown(duration)` enables passive cooldown.
4. **Cooldown marks**: Only failoverable errors mark a deployment as cooling. A successful `chat` or `stream` connection clears the deployment's cooldown immediately.
5. **Cooldown expiry**: Deployments exit cooldown automatically after `duration`. The next routing attempt may retry them.
6. **Cooldown filtering**: When cooldown is enabled, routing prefers deployments not in cooldown. Deployments in cooldown are deprioritized but not permanently excluded.
7. **Fail-open on all-cooling**: If all deployments in a group are in cooldown, the Router fails open: all deployments are attempted in their original order rather than returning an error.
8. **No background health check**: Cooldown is purely passive. No pings, probes, or background tasks. Deployment state is updated only as a side effect of routing.

## Embeddings contract

1. **Model routing**: Embeddings use the same `provider/model` format. `LmrsClient::embed` / `embed_batch` / `embed_with` all parse the prefix and strip it before calling the provider.
2. **Unknown provider**: Returns `LlmError::UnknownProvider(name)`.
3. **Unsupported provider**: Providers that do not override `embed()` return `LlmError::Unsupported { feature: "embeddings", ... }`. Do not return `Api 501`.
4. **Input order**: `data[].index` must reflect the original `input` order. llmrust does not reorder.
5. **Vector dimensions**: Defined by the upstream provider/model. llmrust does not normalize or pad vectors.
6. **Implementation status**: The embeddings API (shared types, the `Provider::embed` method, and client routing) plus the OpenAI-compatible and Ollama provider implementations all ship as of v0.1.1. A new provider adds embeddings support by overriding `Provider::embed`.
7. **OpenAI-compatible transport**: OpenAI, DeepSeek, Moonshot, and OpenRouter wrappers implement `/embeddings`. Requests must not log input text or embedding vectors. Non-2xx upstream errors map to `LlmError::Api`. Response `model` falls back to request `model` when the upstream omits it.
8. **Unsupported providers**: Anthropic and Google Gemini do not implement embeddings and continue to return `LlmError::Unsupported` via the default `Provider::embed`.
9. **Proxy endpoint**: `POST /v1/embeddings` accepts string or string-array input, float encoding only. Base64 and token arrays return 400 `invalid_request_error`. Provider/model routing works identically to chat proxy. Unsupported providers map to 400 (not 502).
10. **Ollama embeddings**: Uses native `POST /api/embed`. Does not send `user`. `prompt_eval_count` maps to `EmbeddingUsage`. Actual local model support depends on installed model.
