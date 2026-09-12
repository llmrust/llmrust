# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.3] - 2026-08-03

### Added

- Publishing channel (REL-003A): the tag-only release pipeline now carries a
  real-upload `publish` job using crates.io **Trusted Publishing (OIDC)**
  (`rust-lang/crates-io-auth-action`, pinned SHA) — a short-lived publishing
  identity instead of a stored long-lived API token. The pre-flight four gates
  and the fmt/clippy gate must both pass before any upload; crates.io visibility
  is verified by a bounded poll. The channel is armed but **not** fired by this
  change — the first real upload happens on the `v0.1.3` tag (REL-003).
- Provider smoke matrix (E2E-001): a low-budget end-to-end harness
  (`examples/e2e_smoke.rs`, env-gated by `LLMRUST_E2E=1`) plus a manual/weekly
  GitHub Actions workflow (`.github/workflows/e2e-smoke.yml`) that calls each of
  the 7 providers at its cheapest tier to surface upstream protocol drift that
  local fixtures cannot cover. See `docs/E2E-SMOKE.md` for operator setup.
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
- OpenAI-compatible reasoning is now explicitly isolated (REA-003): only the verified
  OpenAI provider opts into reasoning — `ChatRequest.thinking` with
  `Enabled{budget_tokens: None}` maps to `reasoning_effort: "medium"` on the streaming
  path, while non-stream `chat` with thinking enabled fails with `LlmError::Unsupported`
  before any network call. `Enabled{budget_tokens: Some(_)}` is rejected for OpenAI (no
  budget equivalent); DeepSeek, Moonshot and OpenRouter never send reasoning fields and
  fail with `Unsupported` when thinking is enabled (zero network). Streamed reasoning
  deltas accept both the `reasoning` and `reasoning_content` field names into
  `StreamChunk.thinking`, and usage translation now maps `prompt_tokens_details.cached_tokens`
  → `cache_read_tokens` and `reasoning_tokens` → `reasoning_tokens`. (REA-003, Refs #133)
- Gemini reasoning is wired on the streaming path (REA-004G): `ChatRequest.thinking`
  maps to the native `thinkingConfig` (`thinkingBudget` is optional upstream, so a
  missing budget is omitted losslessly; `includeThoughts` is always requested when
  enabled), while non-stream `chat` with thinking enabled fails with
  `LlmError::Unsupported` before any network call. Streamed `thought` parts surface via
  `StreamChunk.thinking`, the terminal chunk marks `thinking_done = true` at most once,
  and `usageMetadata.thoughtsTokenCount` maps to `Usage.reasoning_tokens`. (REA-004G,
  Refs #136)
- Ollama reasoning is now rejected instead of silently ignored (REA-004O): the Ollama
  wire offers only `options.think` (bool/level) with no lossless mapping for
  `ThinkingConfig.budget_tokens`, so both `chat()` and `stream()` with thinking enabled
  fail with `LlmError::Unsupported` before any network call (zero network); `Disabled`
  and unset reasoning pass through unchanged. (REA-004O, Refs #141)
- `stream_collect` / `stream_collect_full` no longer silently drop reasoning: any chunk
  carrying a non-empty `thinking` delta or `thinking_done == true` now fails with
  `LlmError::Unsupported` and points the caller to consume the raw `stream()` instead.
  Non-reasoning streams keep their exact prior behavior (text concatenation, terminal
  `usage` / `tool_calls` / `finish_reason`). (STR-003, Refs #144)
- The proxy no longer sends `Access-Control-Allow-Origin: *` by default (security hardening,
  SPCC §7.1): the unauthenticated and authenticated routers send **no CORS allow-origin header**
  unless the caller explicitly wraps the `Router` with a restrictive `CorsLayer`. The
  `proxy_server` example now binds `127.0.0.1:3000` by default (previously `0.0.0.0:3000`,
  which failed to start without a token) and honors `LLMRUST_PROXY_ADDR` to override the listen
  address; a non-loopback address still requires `LLMRUST_PROXY_KEY`. (PRX-001, Refs #150)
- Proxy authentication hardening (PRX-002): `GET /health` no longer requires authentication
  under `router_with_auth` (SPCC §7.2; previously the `check_bearer` layer wrapped every route
  including `/health`); the hand-rolled XOR constant-time comparison was replaced with the
  reviewed `subtle::ConstantTimeEq` (SPCC §7.1, optional dep behind the `proxy` feature); an
  empty or whitespace-only `LLMRUST_PROXY_KEY` now refuses to start (`router_with_auth` panics
  at construction, `serve()` returns an error) instead of silently enabling a degenerate auth
  mode, and a valid key is trimmed before use. (PRX-002, Refs #154)
- The OpenAI-compatible proxy no longer silently drops reasoning (PRX-003): a request carrying
  `reasoning_effort` / `reasoning` / `thinking` is rejected with `400 invalid_request_error`
  before any upstream dispatch (SPCC §6.1/§7.3); an upstream stream chunk carrying a non-empty
  `thinking` delta or `thinking_done == true` now emits exactly one `stream_error` event then
  terminates with `[DONE]`, instead of being silently discarded (SPCC §6.3). All other stream
  behavior (role on first delta, single `[DONE]`, usage-only with `include_usage`, finish once)
  is unchanged. (PRX-003, Refs #157)
- The Anthropic-compatible proxy no longer drops reasoning or truncates streams silently
  (PRX-004): non-empty `thinking` deltas surface as Anthropic thinking blocks
  (`content_block_start` type `thinking` + `thinking_delta`, closed on `thinking_done`;
  `signature_delta` is a declared lossy path); fragmented tool calls with the same id are
  reassembled into one `tool_use` block (`input_json_delta` to the same index); a stream that
  ends without a terminal now emits an `event: error` (`api_error`) instead of a fabricated
  `end_turn`; and a request carrying a `thinking` key is rejected with `400 invalid_request_error`
  before any upstream dispatch. All other Anthropic stream behavior is unchanged. (PRX-004,
  Refs #160)
- The proxy now enforces a configurable request-body limit and normalizes upstream errors
  (PRX-005): both protocol endpoints reject bodies over `LLMRUST_PROXY_MAX_BODY_BYTES` (default
  2 MiB, axum-ecosystem default) with a protocol-shaped `413` JSON error before any upstream
  dispatch, including chunked bodies. Upstream errors map to protocol-shaped bodies:
  `Parse` → `502 api_error`, `Http` → `502` "upstream connection failed", non-JSON upstream →
  fixed wording; error messages are truncated to ≤200 chars and never echo request-body content.
  New `SECURITY.md` deployment section (bind / auth / CORS / body-limit with vision-large-payload
  guidance / reverse-proxy TLS). (PRX-005, Refs #163)
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

- **FIX-R2 (FND-R2)**: the OpenAI-compatible streaming path now marks
  `StreamChunk.thinking_done = Some(true)` on the terminal chunk that surfaced
  a thinking increment (`reasoning` / `reasoning_content`), satisfying
  CONTRACTS.md Provider stream contract §7. Previously the terminal chunk
  carried the `thinking` delta but never emitted the end-of-thinking marker, a
  substantive deviation from the contract (disclosed as a known limitation in
  the v0.1.3 release notes; this change ships in the 0.1.4 batch and will be
  declared resolved there). Includes a failure-first regression test
  (`stream_reasoning_terminal_chunk_marks_thinking_done`). Other providers
  (Anthropic/Gemini) already emit `thinking_done`; this aligns the OpenAI path.
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
- Round-robin rotation is now isolated **per group** (RTR-001): the `Router` previously shared a
  single `AtomicUsize` counter across all groups, so traffic to one group shifted another group's
  next round-robin start. The counter is now a per-group `Mutex<HashMap<String, usize>>`, lazily
  created on first use; single-group behavior, `Ordered` strategy, unknown-group literal
  forwarding, and cooldown/failover semantics are all unchanged. (RTR-001, Refs #174)

### Security

- 0.1.3 is a **governance + API-freeze + incident-remediation** release (see
  `docs/COMPATIBILITY-0.1.3.md` for the full account). It hardens the proxy's default
  security posture: no permissive CORS allow-origin by default, loopback-bound example,
  constant-time token comparison, `/health` auth exemption, request-body limits, and
  protocol-shaped error normalization (PRX-001…PRX-005). Supply-chain and secret-scanning
  gates (cargo-deny, gitleaks, RustSec) run on every PR. No credentials are logged or
  stored anywhere in this crate.

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

## [0.1.4] - UNRELEASED (in progress)

> **本段由各任务卡逐条追加，`REL-004` 定稿时补上日期并上移为首段。**
> 置于文件末尾是**故意的**：`tests/agent_docs_validation.rs` 要求第一个 `## [` 标题
> 必须是 `## [0.1.3] - 2026-08-03`，且不得出现"未发布"段（方括号 Unreleased 形式）。

### Fixed

- **解析失败不再静默（`ERR-001`）**：工具调用参数若不是合法 JSON，llmrust 仍**降级为空对象发出**
  （**成功路径行为零变化**），但**现在会留下 `tracing::warn` 痕迹**——只含工具名与错误位置，
  **不含参数原文 / prompt / 凭证**。涉及 Anthropic 与 Gemini 的**请求构建**，以及 Anthropic
  **代理的响应转换**（此处原先静默改写的是**上游返回**的工具参数，调用方无从察觉）。
  另：HTTP 客户端构建失败时的回落**现在写明丢失了哪些配置**
  （`connect_timeout` / `timeout` / `pool_max_idle_per_host` / `tcp_keepalive` / `no_proxy` / 自定义头），
  并按卡片要求附上**"降级而非传播错误"的书面理由**（签名连锁代价 + 降级后客户端仍可用）。

### Fixed

- **`Retry-After` 的 HTTP-date 形态：年份越界不再 panic（`H-2`）**：`year` 此前只解析不校验，
  巨年份（如 `i64::MAX`）会让 `days_from_civil` 的 `era * 146_097` **i64 溢出**——debug 直接 panic
  （实测 `attempt to multiply with overflow`），release 回绕。现加 **4 位年边界**（`1000..=9999`，
  依 RFC 9110 `year = 4DIGIT`）⇒ 越界返回"不可解析"（回落本地退避），**不把畸形日期降级成"立刻重试"**；
  另以**饱和算术**兜底。合法 4 位年行为不变。
- **代理不再静默丢弃 `cache`（`H-3` 编号待改，见 PR）**：`CAP-005` 给 `ChatRequest` 加了 `cache` 字段，但代理 DTO
  `ProxyChatRequest` **没有**它，而该路径**未启用 `deny_unknown_fields`** ⇒ 客户端向代理发
  `"cache": {"retention": …}` 会被 serde **静默忽略**：**响应 200 成功、断点一个没设**
  （§6.3 禁止的"降级到调用方无从察觉"；对"省钱"目标尤其危险——看着生效、实则零收益）。
  **现在：响亮拒绝**——代理在**派发前**检出该键并返回 **400**，错误体指明"用库内 API 设置
  `ChatRequest.cache`"，**零上游派发**。
  **为什么不直接让代理支持它**：给公开 DTO 加字段是 `constructible_struct_adds_field`，
  CI 的 `API-002 semver 门`判为**破坏性变更**（实测 `field ProxyChatRequest.cache` ⇒
  `semver requires new major version`）——**代理 DTO 并不豁免于该门**。
  故本版采用与本文件 `reasoning` 键**完全一致**的既有做法（检出即 400）。
  **行为变更（wire 面）**：此前发 `cache` 得 200（且无效），现在得 400（并说明去向）；
  **不发的客户端行为不变**，其它未知键**仍照常容忍**（未引入 `deny_unknown_fields`）。

### Changed

- **Proxy 认证：校验与存储对 trim 现在一致（`ERR-005` / `F-D2`）**：此前 `router_with_auth` **校验**用
  `token.trim().is_empty()`，**存储**却保留原始串；而请求侧比对的是 `provided.trim()`。
  于是**配置里带前后空白的 token 会让任何客户端都 401**（`Bearer secret` 与 `Bearer   secret  ` 同样失败），
  排障时表现为"token 明明没错却连不上"——也正是 `FIX-001` 要求先行排除的干扰项。
  处置选择 **规范化（trim）而非拒绝**，理由：① 比对侧本就在 trim，"比 trim 后的形态"是代码既有意图，
  trim 存储正是让两侧一致（本卡目标）；② 拒绝会把现有带空白的部署从"能跑"变成**启动即 panic**，破坏更大；
  ③ **未降低认证强度**——除配置密钥自身的 trim 形态外，没有任何原本被拒的 token 变成可用
  （RFC 6750 的 `b64token` 本不允许空格，带空白属畸形配置而非合法密钥）；
  ④ **常数时间比对未改动**。规范化实际发生时**留 `tracing::warn` 痕迹**（**不打印 token 值本身**）。
- **消费上游 `Retry-After`（`ERR-004`）**：`RetryProvider` 现在**优先采用上游明示的等待窗口**
  （0.1.3 期从不读取该头，全仓 `retry.?after` 零命中），退避日志新增 `delay_source`
  （`"upstream"` / `"local"`）使**窗口来源可观测**。支持 RFC 9110 的**两种格式**
  （`delay-seconds` 与 `HTTP-date`）；**等待窗口上限 60 秒**（防恶意/异常超长值钉死客户端）；
  畸形/缺失值回落本地退避（不猜、不 panic）；`0` 秒提示尊重但下限 1ms（不忙等）。
  **承载方式为纯附加**：走**带默认实现的 trait 方法** `Provider::last_retry_after()`，
  **不改 `LlmError`**（该枚举与其 `Api` 变体都不是 `#[non_exhaustive]`，加字段/加变体对下游
  都是破坏性变更，卡内明禁"破坏错误类型形状"）。
  **`should_retry` 对 429 的既有设计未被推翻**（同一部署不重试限流目标）；**failover 仍不等待**
  （换到另一个未被限流的部署才是正确响应），其日志记录 `waits_for_upstream = false` 与理由。
  **覆盖面（已记录，非静默）**：捕获接在**共享的 OpenAI 兼容路径**上，覆盖
  `openai` / `deepseek` / `moonshot` / `openrouter`；Anthropic / Gemini / Ollama 的错误构造路径
  独立，**尚未提供**提示，行为与 0.1.3 完全一致。
- **Gemini `FINISH_REASON_UNSPECIFIED` 不再当成"正常结束"（`ERR-003`）**：该值此前被映射为
  `FinishReason::Stop`——把**缺失信息**当成了"模型正常结束"的**断言**，调用方会据此以为生成完整。
  现在它走 `FinishReason::Other("FINISH_REASON_UNSPECIFIED")` **逃生口**，语义原样保留；
  流终止判定（`done`）**行为不变**（`Other(..)` 仍是 `Some(..)`）；
  **`FinishReason` 变体集合未变**（新增变体对下游是 breaking，§5.3 禁止）。
  **同族缺口（本版未修、已记录）**：`ollama.rs` 在上游缺 `done_reason` 时用
  `.unwrap_or(FinishReason::Stop)`——同一类"缺失即视为正常结束"，经**字段缺失**而非显式
  `UNSPECIFIED` 抵达；因本卡禁止改动其他 Provider 的映射，故仅登记为后续发现。
- **错误分类保真与错误体上界（`ERR-002`）**：`Router` 的 failover 日志不再把 `error_kind`
  **硬编码为 `api_error`**，改为反映真实分类（`authentication_error` / `rate_limit_error` /
  `invalid_request_error` / `api_error` / `connection_error` / `stream_error` / `parse_error` /
  `unknown_provider` / `unsupported`）——0.1.3 期"限流 / 连接失败 / 上游 5xx / provider 未注册"
  在日志里**全是一个样**，排障无从区分。
- **代理错误体的 ≤200 字符上界现在对全部路径生效**（`FND-R3`）：此前 `UnknownProvider`
  （载荷是**调用方可控**的 `provider/model` 串）与 `Unsupported` 两条分支**未截断**，
  属无界反射、且与注释宣称的"唯一机械规则"不符。截断改为**结构性**（移出 `match`，只做一次），
  新增分支**无法遗漏**。（注释与实现不符时的处置选择：**改实现**，理由已写入代码注释。）
- **能力检查上移到统一入口（`CAP-003`）**：`LmrsClient` 在派发前做一次**能力裁决**，
  依据 `Provider::capabilities()` 的声明决定 **放行 / 告警 / 拒绝**。
  公开行为变更（**唯一一处**）：
  - 对**声明该能力为 `unsupported`** 的 Provider 发送相应诉求时，现在返回
    `LlmError::Unsupported`，而**不再是静默丢弃**。当前受影响的具体情形：
    **Ollama + `tools`**（非流式 → `feature = "tool_calling"`；流式 → `"tool_calling_stream"`）、
    **Ollama + 图像输入**（`feature = "image_input"`）。
  - **已支持路径行为零变化**：裁决表为空时不产生任何副作用；
    `n > 1` 仍是"放行 + 一次性告警"（`CAP-003` 未改此语义）。
- **告警去重口径**：由 `(provider, n)` 改为 `(provider, capability-face)`，
  且只在入口判一次——`RetryProvider` 重入**不再重复告警**。

### Added

- **`Provider::capabilities()` 与能力声明载体（`CAP-002`）**：新增 `Capabilities`
  （`#[non_exhaustive]`）、`Capability`、`CapabilityLevel`（SPCC §6.2 五级）、
  `EvidenceKind`、`Verified`。`Provider::capabilities()` **带默认实现**，下游自定义
  Provider 不实现亦不破坏；默认声明保守（全部视为 `unsupported`）并一次性告警。
- **缓存断点发送能力（`CAP-005`）**：`CachePolicy` / `CacheRetention` 与
  `ChatRequest.cache`；按库内约定在 `system` 末块、最后一个工具定义、最后一条消息末块
  打断点；`MAX_CACHE_BREAKPOINTS = 4` 由**编译期断言**保证。
- **缓存价列（`CAP-006`）**：`CachePricing`（读价 / 写价 5m / 写价 1h）与
  `ModelPricing::estimate_cost_with_cache`；未配置档**回落 prompt 价，绝不静默免费**。
- **官方服务商目录（`CAP-006`）**：`AccessPath` / `ModelSpec` 两维模型（8 模型 / 15 条官方路径）。
- **每模型能力/价格表（`CAP-007`）**：`llmrust.models.json` 为唯一事实源，
  `docs/CAPABILITIES.md` 中对应区块**由表生成**（人工改动即 CI 红）。

