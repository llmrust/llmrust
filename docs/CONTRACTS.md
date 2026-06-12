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
4. **Stream collection**: `stream_collect` returns concatenated text. `stream_collect_full` returns a full `ChatResponse` with `usage`, `tool_calls`, and `finish_reason`.
5. **Retry wrapping**: `with_retry(max_retries)` wraps all registered providers in `RetryProvider`. Retry logic applies exponential backoff and only retries on transient errors (5xx, network errors).
