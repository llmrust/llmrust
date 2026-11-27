//! Shared implementation for all OpenAI-compatible chat completion APIs.
//!
//! OpenAI, DeepSeek, Moonshot, and OpenRouter all expose the same
//! `/chat/completions` endpoint with an identical body format. This module
//! provides a single [`OpenAiCompatibleProvider`] struct that can be
//! configured for each provider via [`ProviderConfig`] and extra headers,
//! plus timeouts and connection pooling applied uniformly.

use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::providers::http::{self, build_http_client, REQUEST_TIMEOUT};
use crate::providers::sse;
use crate::providers::{LlmError, Provider, ProviderConfig, Result};
use crate::types::{
    ChatRequest, ChatResponse, Content, Embedding, EmbeddingRequest, EmbeddingResponse,
    EmbeddingUsage, FinishReason, FunctionCall, LogProbs, Message, ResponseFormat, StreamChunk,
    ThinkingConfig, Tool, ToolCall, ToolChoice, Usage,
};
use std::collections::HashMap;

// ── Shared request / response types ───────────────────────

#[derive(Serialize)]
struct CompChatRequest<'a> {
    model: &'a str,
    messages: &'a [CompMessage],
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f64>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<StreamOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<&'a [Tool]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'a ToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<&'a ResponseFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop: Option<&'a [String]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    n: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    presence_penalty: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frequency_penalty: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    logprobs: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_logprobs: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parallel_tool_calls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    service_tier: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    store: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<&'a serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<&'a str>,
    /// REA-003: reasoning effort for OpenAI reasoning models
    /// (`Enabled{budget_tokens: None}` → `"medium"`; a budget has no OpenAI
    /// equivalent and is rejected before the network, O-5).
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'a str>,
    /// Provider-specific extra fields flattened into the request body.
    #[serde(flatten)]
    extra: &'a HashMap<String, serde_json::Value>,
}

/// Asks OpenAI-compatible servers to emit a terminal chunk carrying token
/// usage when streaming (supported by OpenAI, DeepSeek, Moonshot, OpenRouter).
#[derive(Serialize)]
struct StreamOptions {
    include_usage: bool,
}

#[derive(Serialize, Deserialize)]
struct CompMessage {
    role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    content: Option<Content>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ToolCall>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

impl From<&Message> for CompMessage {
    fn from(msg: &Message) -> Self {
        let role = match msg.role {
            crate::types::Role::System => "system",
            crate::types::Role::User => "user",
            crate::types::Role::Assistant => "assistant",
            crate::types::Role::Tool => "tool",
        }
        .to_string();
        // OpenAI expects an assistant tool-call turn to carry `content: null`
        // (omitted here) rather than an empty string alongside `tool_calls`.
        let content = if msg.content.is_empty() && msg.tool_calls.is_some() {
            None
        } else {
            Some(msg.content.clone())
        };
        Self {
            role,
            content,
            tool_calls: msg.tool_calls.clone(),
            tool_call_id: msg.tool_call_id.clone(),
            name: msg.name.clone(),
        }
    }
}

#[derive(Deserialize)]
struct CompResponse {
    choices: Vec<CompChoice>,
    model: String,
    usage: Option<CompUsage>,
}

#[derive(Deserialize)]
struct CompChoice {
    message: CompMessage,
    #[serde(default)]
    finish_reason: Option<String>,
    #[serde(default)]
    logprobs: Option<LogProbs>,
}

#[derive(Deserialize)]
struct CompUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
    /// REA-003: reasoning tokens reported for o-series models.
    #[serde(default)]
    reasoning_tokens: Option<u64>,
    /// REA-003: prompt-cache hit tokens (`prompt_tokens_details.cached_tokens`).
    #[serde(default)]
    prompt_tokens_details: Option<CompPromptTokensDetails>,
}

#[derive(Deserialize)]
struct CompPromptTokensDetails {
    #[serde(default)]
    cached_tokens: Option<u64>,
}

/// AUD#3 (C-AUD-COMPAT): an in-band error payload delivered inside a
/// 200-status SSE frame — `data: {"error":{...},"choices":[]}`. Real
/// upstreams end a stream this way (observed on quota exhaustion: the HTTP
/// status stays 200 and the failure is only visible in the frame body), so
/// the frame has to surface as [`LlmError::Api`], never be dropped.
#[derive(Deserialize)]
struct CompStreamError {
    #[serde(default)]
    message: String,
    /// OpenAI-compatible servers send `code` either as a number (HTTP-ish
    /// status, e.g. `429`) or as a machine string; both are accepted, and
    /// only a numeric error-range code is used as the surfaced status.
    #[serde(default)]
    code: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct CompStreamChunk {
    #[serde(default)]
    choices: Vec<CompStreamChoice>,
    #[serde(default)]
    usage: Option<CompUsage>,
    /// AUD#3: in-band error field, named exactly as on the wire (`error`)
    /// and absent on normal frames. Checked before the choices arm, so a
    /// `{"error":{...},"choices":[]}` frame can never fall into the
    /// choice-less keepalive path (the old silent drop). Public so the
    /// field form is directly checkable by the card's machine probe.
    #[serde(default)]
    pub error: Option<CompStreamError>,
}

#[derive(Deserialize)]
struct CompStreamChoice {
    /// DELTA-NULL: OpenAI-compatible endpoints commonly send the terminal frame
    /// as `{"delta":null,"finish_reason":"stop",...}` (official OpenAI semantics,
    /// llama.cpp, some vLLM builds, self-hosted proxies), and some omit the field
    /// entirely. Both mean "no increment, only a finish_reason", so this is
    /// optional and consumers substitute an empty [`CompDelta`].
    #[serde(default)]
    delta: Option<CompDelta>,
    finish_reason: Option<String>,
}

#[derive(Deserialize, Default)]
struct CompDelta {
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<CompToolCallDelta>>,
    /// M2-16：reasoning 增量（OpenAI o-series `reasoning` 字段）。
    #[serde(default)]
    reasoning: Option<String>,
    /// REA-003：部分 OpenAI-compatible 端点（含 OpenAI 官方 o-series chat
    /// completions）使用 `reasoning_content` 字段名；两者都接受（容错解析，
    /// O-1 处理）。
    #[serde(default)]
    reasoning_content: Option<String>,
}

/// A streamed tool-call fragment. OpenAI-compatible servers stream tool calls
/// incrementally: the first fragment for a given `index` carries the call `id`
/// and function `name`, and later fragments append `arguments` text.
#[derive(Deserialize)]
struct CompToolCallDelta {
    #[serde(default)]
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<CompFunctionDelta>,
}

#[derive(Deserialize)]
struct CompFunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Deserialize)]
struct CompErrorBody {
    error: CompErrorDetail,
}

#[derive(Deserialize)]
struct CompErrorDetail {
    message: String,
}

// ── Embedding request / response types ────────────────────────────

#[derive(Serialize)]
struct CompEmbeddingRequest<'a> {
    model: &'a str,
    input: &'a [String],
    #[serde(skip_serializing_if = "Option::is_none")]
    dimensions: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<&'a str>,
    #[serde(flatten)]
    extra: &'a HashMap<String, serde_json::Value>,
}

#[derive(Deserialize)]
struct CompEmbeddingResponse {
    #[serde(default)]
    model: Option<String>,
    data: Vec<CompEmbedding>,
    #[serde(default)]
    usage: Option<CompEmbeddingUsage>,
}

#[derive(Deserialize)]
struct CompEmbedding {
    index: usize,
    embedding: Vec<f32>,
}

#[derive(Deserialize)]
struct CompEmbeddingUsage {
    prompt_tokens: u64,
    total_tokens: u64,
}

/// AUD#7 (C-AUD-COMPAT): hard upper bound on the tool-call `index` an
/// upstream frame may carry. Real OpenAI-compatible streams number tool
/// calls densely from 0 (a handful per turn, even with parallel tool
/// calls), so anything beyond this is a broken or hostile payload. The
/// bound exists because the accumulator is a `Vec` positioned by `index`:
/// with unbounded growth a single frame directly sized the allocation
/// (`index = 1000000` ordered a million builders up front; `usize::MAX`
/// even overflowed/panicked). Reject loudly instead of dying by OOM.
const MAX_TOOL_CALL_INDEX: usize = 1024;

/// Accumulates streamed tool-call fragments across SSE lines. OpenAI-style
/// streams deliver tool calls as a series of deltas keyed by `index`: the
/// first delta for an index carries `id` and `name`, later deltas append
/// `arguments` text. [`ToolCallAccumulator::ingest`] folds each delta in and
/// [`ToolCallAccumulator::take`] drains the completed calls on the terminal
/// chunk.
///
/// The accumulator doubles as the stream-state carrier for AUD#1: the
/// `saw_thinking` flag records whether any thinking increment has been
/// observed in this stream, so a later terminal frame can report the end of
/// the thinking stage without re-checking its own frame for a reasoning
/// delta (real reasoning streams finish on a bare
/// `{"delta":null,"finish_reason":"stop"}` frame).
#[derive(Default)]
struct ToolCallAccumulator {
    builders: Vec<ToolCallBuilder>,
    /// AUD#1 (C-AUD-COMPAT): sticky terminal-state flag — set when a frame
    /// carries a `reasoning`/`reasoning_content` increment, read when the
    /// terminal chunk computes `thinking_done`.
    saw_thinking: bool,
}

#[derive(Default)]
struct ToolCallBuilder {
    id: String,
    name: String,
    arguments: String,
}

impl ToolCallAccumulator {
    fn ingest(&mut self, deltas: &[CompToolCallDelta]) -> Result<()> {
        for delta in deltas {
            // AUD#7 (C-AUD-COMPAT): bound the index BEFORE any growth. The
            // accumulator is positioned by the upstream-supplied `index`,
            // so an unbounded grow turned one hostile frame into a direct
            // allocation order (panic/OOM at absurd indexes); reject it.
            if delta.index > MAX_TOOL_CALL_INDEX {
                return Err(LlmError::Parse(format!(
                    "streamed tool_call index {} exceeds the bounded accumulator limit {}",
                    delta.index, MAX_TOOL_CALL_INDEX
                )));
            }
            if self.builders.len() <= delta.index {
                // Reachable only for index <= MAX_TOOL_CALL_INDEX (guarded
                // above): the allocation is bounded by the constant.
                self.builders
                    .resize_with(delta.index + 1, ToolCallBuilder::default);
            }
            let builder = &mut self.builders[delta.index];
            if let Some(id) = delta.id.as_deref().filter(|s| !s.is_empty()) {
                builder.id = id.to_string();
            }
            if let Some(function) = &delta.function {
                if let Some(name) = function.name.as_deref().filter(|s| !s.is_empty()) {
                    builder.name = name.to_string();
                }
                if let Some(arguments) = &function.arguments {
                    builder.arguments.push_str(arguments);
                }
            }
        }
        Ok(())
    }

    fn take(&mut self) -> Option<Vec<ToolCall>> {
        if self.builders.is_empty() {
            return None;
        }
        let calls: Vec<ToolCall> = self
            .builders
            .drain(..)
            .filter(|b| !b.name.is_empty())
            .map(|b| ToolCall {
                id: b.id,
                call_type: "function".to_string(),
                function: FunctionCall {
                    name: b.name,
                    arguments: b.arguments,
                },
            })
            .collect();
        if calls.is_empty() {
            None
        } else {
            Some(calls)
        }
    }
}

/// Parse a single SSE line from an OpenAI-compatible stream into zero or more
/// [`StreamChunk`]s, threading a [`ToolCallAccumulator`] so streamed tool-call
/// fragments can be reassembled across lines. Lines are guaranteed complete by
/// [`line_stream`], so a failed JSON parse here indicates a genuinely
/// malformed payload rather than a chunk-boundary artifact.
///
/// A streamed response ends with a chunk carrying a `finish_reason`, optionally
/// followed by a choices-less chunk carrying only `usage` (when
/// `stream_options.include_usage` was requested), then a literal `[DONE]`.
/// Accumulated tool calls are drained onto the chunk that carries
/// `finish_reason`.
///
/// A frame carrying an in-band `error` object (an upstream failure delivered
/// inside a 200 stream, with `choices` empty or absent) surfaces as
/// `LlmError::Api` instead of being dropped as a keepalive (AUD#3).
fn parse_sse_line(tools: &mut ToolCallAccumulator, line: &str) -> Vec<Result<StreamChunk>> {
    let line = line.trim();
    let Some(data) = line.strip_prefix("data: ") else {
        return Vec::new();
    };
    if data == "[DONE]" {
        return vec![Ok(StreamChunk {
            done: true,
            ..Default::default()
        })];
    }
    let parsed = match serde_json::from_str::<CompStreamChunk>(data) {
        Ok(p) => p,
        Err(e) => {
            return vec![Err(LlmError::Parse(format!(
                "failed to parse OpenAI-compatible stream chunk: {e}"
            )))];
        }
    };
    // AUD#3 (C-AUD-COMPAT): surface an in-band error frame. Upstreams may
    // end a 200 stream with `data: {"error":{...},"choices":[]}`; the
    // choice-less shape used to fall through to the keepalive path, so the
    // only record of the failure (e.g. quota exhaustion) vanished — a
    // silent drop of an upstream error (AGENTS.md Providers; CONTRACTS.md
    // stream contract "No silent drops"). A numeric `code` in the HTTP
    // error range becomes the surfaced status; anything else maps to 502
    // (upstream-side failure reported in-band, no usable wire status).
    if let Some(err) = &parsed.error {
        let status = err
            .code
            .as_ref()
            .and_then(|c| c.as_u64())
            .filter(|c| (400..=599).contains(c))
            .map_or(502, |c| c as u16);
        return vec![Err(LlmError::Api {
            status,
            message: err.message.clone(),
        })];
    }
    let usage = parsed.usage.map(comp_usage_to_usage);
    match parsed.choices.first() {
        Some(choice) => {
            // DELTA-NULL: `delta` may be absent or explicitly `null` on terminal
            // frames; substitute an empty delta so downstream logic is unchanged.
            let empty_delta = CompDelta::default();
            let delta = choice.delta.as_ref().unwrap_or(&empty_delta);
            if let Some(deltas) = &delta.tool_calls {
                if let Err(e) = tools.ingest(deltas) {
                    return vec![Err(e)];
                }
            }
            let finish_reason = choice.finish_reason.clone().map(FinishReason::from);
            let tool_calls = if finish_reason.is_some() {
                tools.take()
            } else {
                None
            };
            // REA-003: accept both `reasoning` and `reasoning_content` delta
            // field names (O-1: tolerant parsing; the authoritative name is
            // verified against real endpoints in E2E-001).
            let thinking = delta
                .reasoning
                .clone()
                .or_else(|| delta.reasoning_content.clone());
            // AUD#1 (C-AUD-COMPAT): sticky observation — this frame streams
            // a thinking increment, so the stream HAS a thinking stage.
            // Recorded before the terminal decision below, so a terminal
            // frame that carries its own increment (o-series `reasoning` +
            // `finish_reason` on the same frame) still counts.
            if thinking.is_some() {
                tools.saw_thinking = true;
            }
            let done = finish_reason.is_some();
            // AUD#1 (C-AUD-COMPAT), architect ruling 2026-11-20: sticky
            // terminal marking. `thinking_done` reports whether THIS STREAM
            // has produced any thinking increment up to and including this
            // frame — the previous same-frame check (did the terminal frame
            // itself carry a reasoning increment?) was wrong twice over:
            // a reasoning stream that finishes on a bare frame
            // (`{"delta":null,"finish_reason":"stop"}`) got
            // `Some(false)` — an active lie about the stream (severity:
            // severe) — and the sticky semantics below is a superset of the
            // old one. `Some(true)` at most once on the terminal chunk per
            // Contract §7 is preserved by construction: only a terminal
            // frame (`done == true`) produces a `Some` at all.
            let thinking_done = done.then_some(tools.saw_thinking);
            vec![Ok(StreamChunk {
                delta: delta.content.clone().unwrap_or_default(),
                done,
                finish_reason,
                usage,
                tool_calls,
                thinking,
                thinking_done,
            })]
        }
        None => {
            if usage.is_some() {
                vec![Ok(StreamChunk {
                    usage,
                    ..Default::default()
                })]
            } else {
                Vec::new()
            }
        }
    }
}

/// Assemble [`StreamChunk`]s from a stream of complete SSE lines (as produced
/// by [`line_stream`]), threading a [`ToolCallAccumulator`] and closing the
/// tool-call flush gap at stream end.
///
/// A well-behaved OpenAI-compatible stream drains the accumulator onto the
/// chunk that carries `finish_reason` (inside [`parse_sse_line`]). Two
/// real-world shapes end without that frame: an upstream that sends `[DONE]`
/// while the arguments are already fully accumulated (observed on xunfei
/// spark-x2.6 — the same request with `stream=false` returns
/// `finish_reason:"tool_calls"` normally), and an upstream that simply closes
/// the connection. In both cases the accumulated calls are flushed onto the
/// stream's clean terminal point — the `[DONE]` chunk, or one final `done`
/// chunk at end-of-stream — with `finish_reason` left absent: an absent reason
/// is missing information, never invented as `Stop` (ERR-003 / CONTRACTS.md
/// stream contract rule 8). Nothing drains per chunk, so incremental
/// accumulation semantics are unchanged, and both flushes are no-ops for
/// well-behaved streams whose accumulator is empty by then.
///
/// A stream that surfaced an error — transport or parse — ends without
/// flushing: half-finished arguments are the caller's salvage problem, not a
/// success terminal to fabricate. llmrust keeps "an error is an error"
/// (CONTRACTS.md stream contract rules 4–5).
fn sse_chunk_stream<S>(
    lines: S,
) -> impl futures::Stream<Item = Result<StreamChunk>> + Send + 'static
where
    S: futures::Stream<Item = Result<String>> + Send + 'static,
{
    futures::stream::unfold(
        (lines.boxed(), ToolCallAccumulator::default(), false, false),
        |(mut lines, mut tools, mut errored, ended)| async move {
            // `ended` is set only on the error path (no flush afterwards) and
            // after a terminal flush chunk (no second terminal).
            if ended {
                return None;
            }
            loop {
                match lines.next().await {
                    Some(Ok(line)) => {
                        let mut chunks = parse_sse_line(&mut tools, &line);
                        if chunks.iter().any(|c| c.is_err()) {
                            errored = true;
                        }
                        // Flush point 1 — the clean `[DONE]` terminal: a done
                        // chunk carrying neither `finish_reason` nor calls
                        // (`parse_sse_line`'s finish_reason drain already
                        // attached them otherwise). A no-op for well-behaved
                        // streams, whose accumulator is empty by the time
                        // `[DONE]` arrives.
                        if !errored {
                            for c in chunks.iter_mut().flatten() {
                                let bare_done =
                                    c.done && c.finish_reason.is_none() && c.tool_calls.is_none();
                                if bare_done {
                                    c.tool_calls = tools.take();
                                }
                            }
                        }
                        if chunks.is_empty() {
                            continue;
                        }
                        return Some((
                            futures::stream::iter(chunks),
                            (lines, tools, errored, ended),
                        ));
                    }
                    Some(Err(e)) => {
                        // Forward the error, then stop. Deliberately no flush:
                        // emitting half-finished tool arguments as a done
                        // chunk would launder an interrupted stream into a
                        // clean one.
                        let error: Result<StreamChunk> = Err(e);
                        return Some((
                            futures::stream::iter(vec![error]),
                            (lines, tools, errored, true),
                        ));
                    }
                    None => {
                        // Flush point 2 — clean end-of-stream with no terminal
                        // frame at all (no finish_reason, no `[DONE]`): the
                        // pending calls surface as one final done chunk; an
                        // empty accumulator (well-behaved stream, or `[DONE]`
                        // already drained it) ends the stream exactly as
                        // before. After a surfaced error, never flush.
                        if errored {
                            return None;
                        }
                        match tools.take() {
                            Some(calls) => {
                                let chunk = StreamChunk {
                                    done: true,
                                    tool_calls: Some(calls),
                                    ..Default::default()
                                };
                                return Some((
                                    futures::stream::iter(vec![Ok(chunk)]),
                                    (lines, tools, errored, true),
                                ));
                            }
                            None => return None,
                        }
                    }
                }
            }
        },
    )
    .flatten()
}

/// Translate OpenAI-compatible usage into the frozen `Usage` shape
/// (REA-003, SPCC §6.4): `prompt_tokens_details.cached_tokens` →
/// `cache_read_tokens`, `reasoning_tokens` → `reasoning_tokens`; missing
/// fields stay `None`, present zero values become `Some(0)`.
fn comp_usage_to_usage(u: CompUsage) -> Usage {
    Usage {
        prompt_tokens: u.prompt_tokens,
        completion_tokens: u.completion_tokens,
        total_tokens: u.total_tokens,
        cache_read_tokens: u.prompt_tokens_details.and_then(|d| d.cached_tokens),
        reasoning_tokens: u.reasoning_tokens,
        ..Default::default()
    }
}

// ── The unified OpenAI-compatible provider ────────────

/// A generic provider for any OpenAI-compatible `/chat/completions` API.
///
/// Headers are built from:
/// 1. `Authorization: Bearer {api_key}` (always)
/// 2. Any extra headers provided at construction time (e.g. OpenRouter's
///    `HTTP-Referer` / `X-Title`)
pub struct OpenAiCompatibleProvider {
    client: Client,
    api_key: String,
    base_url: String,
    extra_headers: Vec<(String, String)>,
    /// REA-003: only the verified OpenAI endpoint may send reasoning fields.
    /// Third-party OpenAI-compatible wrappers must NOT inherit this capability.
    reasoning_supported: bool,
}

impl OpenAiCompatibleProvider {
    /// Create a new provider. Extra headers are appended after the standard
    /// `Authorization` header — they can override it if needed.
    pub fn new(
        config: ProviderConfig,
        extra_headers: impl IntoIterator<Item = (String, String)>,
    ) -> Self {
        let timeout = config
            .timeout_secs
            .map(Duration::from_secs)
            .or(Some(REQUEST_TIMEOUT));
        let base_url = config.base_url.unwrap_or_default();
        Self {
            client: build_http_client(timeout, config.custom_headers.as_ref()),
            api_key: config.api_key,
            base_url,
            extra_headers: extra_headers.into_iter().collect(),
            reasoning_supported: false,
        }
    }

    /// Opt into verified OpenAI reasoning support (only used by `OpenAIProvider`).
    pub fn with_reasoning(mut self, enabled: bool) -> Self {
        self.reasoning_supported = enabled;
        self
    }

    /// REA-003 reasoning gate. Returns the `reasoning_effort` value to send
    /// (streaming only) or an error:
    ///
    /// - `Disabled`/unset → `None`;
    /// - `Enabled{budget_tokens: Some(_)}` → `Unsupported` (O-5: no OpenAI
    ///   budget parameter; silently ignoring the budget is forbidden);
    /// - `Enabled{budget_tokens: None}` on an unverified wrapper → `Unsupported`;
    /// - `Enabled{budget_tokens: None}` on OpenAI, non-stream `chat` →
    ///   `Unsupported` (ChatResponse cannot carry reasoning, §6.3);
    /// - `Enabled{budget_tokens: None}` on OpenAI, streaming → `Some("medium")`.
    fn reasoning_effort(&self, req: &ChatRequest, streaming: bool) -> Result<Option<&'static str>> {
        match &req.thinking {
            None | Some(ThinkingConfig::Disabled) => Ok(None),
            Some(ThinkingConfig::Enabled {
                budget_tokens: Some(_),
            }) => Err(LlmError::Unsupported {
                feature: "reasoning".to_string(),
                message: "OpenAI reasoning has no budget_tokens equivalent; \
                          set budget_tokens to None or use ThinkingConfig::Disabled"
                    .to_string(),
            }),
            Some(ThinkingConfig::Enabled {
                budget_tokens: None,
            }) => {
                if !self.reasoning_supported {
                    return Err(LlmError::Unsupported {
                        feature: "reasoning".to_string(),
                        message: "this OpenAI-compatible endpoint has no verified reasoning \
                                  support; reasoning fields are not forwarded"
                            .to_string(),
                    });
                }
                if !streaming {
                    return Err(LlmError::Unsupported {
                        feature: "reasoning".to_string(),
                        message: "reasoning is only supported on the streaming path in 0.1.3 \
                                  (ChatResponse cannot carry reasoning)"
                            .to_string(),
                    });
                }
                Ok(Some("medium"))
            }
        }
    }

    /// Build the shared chat-completions body (used by `chat` and `stream`).
    fn build_body<'a>(
        req: &'a ChatRequest,
        messages: &'a [CompMessage],
        stream: bool,
        reasoning_effort: Option<&'a str>,
    ) -> CompChatRequest<'a> {
        CompChatRequest {
            model: &req.model,
            messages,
            temperature: req.temperature,
            max_tokens: req.max_tokens,
            top_p: req.top_p,
            stream,
            stream_options: if stream {
                Some(StreamOptions {
                    include_usage: true,
                })
            } else {
                None
            },
            tools: req.tools.as_deref(),
            tool_choice: req.tool_choice.as_ref(),
            response_format: req.response_format.as_ref(),
            stop: req.stop.as_deref(),
            n: req.n,
            seed: req.seed,
            presence_penalty: req.presence_penalty,
            frequency_penalty: req.frequency_penalty,
            logprobs: req.logprobs,
            top_logprobs: req.top_logprobs,
            parallel_tool_calls: req.parallel_tool_calls,
            service_tier: req.service_tier.as_deref(),
            store: req.store,
            metadata: req.metadata.as_ref(),
            user: req.user.as_deref(),
            reasoning_effort,
            extra: &req.extra,
        }
    }

    /// Send a fully-built request body and return the raw HTTP response.
    async fn send(&self, body: &CompChatRequest<'_>) -> Result<reqwest::Response> {
        let url = http::redirect::endpoint_url(&self.base_url, "chat/completions")?;
        let mut rb = self
            .client
            .post(url)
            .header("Authorization", format!("Bearer {}", self.api_key));

        for (k, v) in &self.extra_headers {
            rb = rb.header(k.as_str(), v.as_str());
        }

        let resp = rb
            .json(body)
            .send()
            .await
            .map_err(http::error::http_error)?;
        Ok(resp)
    }

    /// Common HTTP error -> LlmError conversion.
    ///
    /// `ERR-004`：捕获当前响应头并绑定到外层单次调用 scope；成功、无头或网络错误
    /// 都不会继承其它请求的提示。
    async fn parse_error(resp: reqwest::Response) -> LlmError {
        let status = resp.status().as_u16();
        http::retry_hint::capture_response(&resp);
        let text = resp.text().await.unwrap_or_default();
        let msg = serde_json::from_str::<CompErrorBody>(&text)
            .map(|e| e.error.message)
            .unwrap_or(text);
        LlmError::Api {
            status,
            message: msg,
        }
    }

    /// Send an embeddings request and return the raw HTTP response.
    async fn send_embed(&self, body: &CompEmbeddingRequest<'_>) -> Result<reqwest::Response> {
        let url = http::redirect::endpoint_url(&self.base_url, "embeddings")?;
        let mut rb = self
            .client
            .post(url)
            .header("Authorization", format!("Bearer {}", self.api_key));

        for (k, v) in &self.extra_headers {
            rb = rb.header(k.as_str(), v.as_str());
        }

        let resp = rb
            .json(body)
            .send()
            .await
            .map_err(http::error::http_error)?;
        Ok(resp)
    }

    /// Parse a non-streaming response into [`ChatResponse`].
    fn parse_response(parsed: CompResponse) -> ChatResponse {
        let usage = parsed.usage.map(comp_usage_to_usage);

        let (content, tool_calls, finish_reason, logprobs) = match parsed.choices.into_iter().next()
        {
            Some(choice) => {
                let content = match choice.message.content {
                    Some(c) => c.as_text(),
                    None => String::new(),
                };
                (
                    content,
                    choice.message.tool_calls,
                    choice.finish_reason.map(FinishReason::from),
                    choice.logprobs,
                )
            }
            None => (String::new(), None, None, None),
        };

        ChatResponse {
            content,
            model: parsed.model,
            usage,
            tool_calls,
            finish_reason,
            logprobs,
        }
    }
}

#[async_trait]
impl Provider for OpenAiCompatibleProvider {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse> {
        http::retry_hint::clear_response();
        // REA-003: reasoning gate (Unsupported before any network call).
        let reasoning_effort = self.reasoning_effort(req, false)?;
        let messages: Vec<CompMessage> = req.messages.iter().map(CompMessage::from).collect();

        let body = Self::build_body(req, &messages, false, reasoning_effort);

        tracing::debug!(
            provider = "openai-compatible",
            model = &req.model,
            "sending chat request"
        );
        let resp = self.send(&body).await?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let err = Self::parse_error(resp).await;
            tracing::error!(
                provider = "openai-compatible",
                status,
                error_kind = "api_error",
                "API error"
            );
            return Err(err);
        }

        let parsed: CompResponse = resp.json().await.map_err(|e| {
            LlmError::Parse(format!(
                "OpenAI-compatible parse: {}",
                http::error::error_message(e)
            ))
        })?;

        let result = Self::parse_response(parsed);
        tracing::debug!(provider = "openai-compatible", model = &result.model, finish_reason = ?result.finish_reason, "chat response received");
        Ok(result)
    }

    async fn stream(&self, req: &ChatRequest) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        http::retry_hint::clear_response();
        // REA-003: reasoning gate (Unsupported before any network call).
        let reasoning_effort = self.reasoning_effort(req, true)?;
        let messages: Vec<CompMessage> = req.messages.iter().map(CompMessage::from).collect();

        let body = Self::build_body(req, &messages, true, reasoning_effort);

        tracing::debug!(
            provider = "openai-compatible",
            model = &req.model,
            "sending stream request"
        );
        let resp = self.send(&body).await?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let err = Self::parse_error(resp).await;
            tracing::error!(
                provider = "openai-compatible",
                status,
                error_kind = "api_error",
                "API error"
            );
            return Err(err);
        }

        // C-018-STREAM-CORE: a 200 response is only a stream when it says so.
        // `application/json` (or a missing type) is an explicit protocol error,
        // never an empty success.
        sse::require_event_stream(resp.headers())?;

        let byte_stream = resp
            .bytes_stream()
            .map(|r| r.map_err(|e| LlmError::Stream(http::error::error_message(e))));

        let stream = sse_chunk_stream(sse::event_line_stream(byte_stream));

        Ok(stream.boxed())
    }

    async fn embed(&self, req: &EmbeddingRequest) -> Result<EmbeddingResponse> {
        http::retry_hint::clear_response();
        tracing::debug!(
            provider = "openai-compatible",
            model = %req.model,
            input_count = req.input.len(),
            "sending embedding request"
        );

        let body = CompEmbeddingRequest {
            model: &req.model,
            input: &req.input,
            dimensions: req.dimensions,
            user: req.user.as_deref(),
            extra: &req.extra,
        };

        let resp = self.send_embed(&body).await?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let err = Self::parse_error(resp).await;
            tracing::error!(
                provider = "openai-compatible",
                status,
                error_kind = "api_error",
                "embedding API error"
            );
            return Err(err);
        }

        let parsed: CompEmbeddingResponse = resp.json().await.map_err(|e| {
            LlmError::Parse(format!(
                "OpenAI-compatible embeddings parse: {}",
                http::error::error_message(e)
            ))
        })?;

        let data: Vec<Embedding> = parsed
            .data
            .into_iter()
            .map(|d| Embedding {
                index: d.index,
                embedding: d.embedding,
            })
            .collect();

        let usage = parsed.usage.map(|u| EmbeddingUsage {
            prompt_tokens: u.prompt_tokens,
            total_tokens: u.total_tokens,
        });

        let model = parsed.model.unwrap_or_else(|| req.model.clone());

        let result = EmbeddingResponse { model, data, usage };

        tracing::debug!(
            provider = "openai-compatible",
            model = %result.model,
            embedding_count = result.data.len(),
            "embedding response received"
        );
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ContentPart, FunctionCall};

    #[test]
    fn serializes_tools_and_tool_choice() {
        let tools = vec![Tool::function(
            "get_weather",
            Some("Get weather".to_string()),
            serde_json::json!({"type": "object"}),
        )];
        let choice = ToolChoice::auto();
        let messages = vec![CompMessage::from(&Message::user("hi"))];
        let body = CompChatRequest {
            model: "gpt-4o",
            messages: &messages,
            temperature: None,
            max_tokens: None,
            top_p: None,
            stream: false,
            stream_options: None,
            tools: Some(tools.as_slice()),
            tool_choice: Some(&choice),
            response_format: None,
            stop: None,
            n: None,
            seed: None,
            presence_penalty: None,
            frequency_penalty: None,
            logprobs: None,
            top_logprobs: None,
            parallel_tool_calls: None,
            service_tier: None,
            store: None,
            metadata: None,
            user: None,
            reasoning_effort: None,
            extra: &HashMap::new(),
        };

        let v = serde_json::to_value(&body).unwrap();
        assert_eq!(v["tools"][0]["type"], "function");
        assert_eq!(v["tools"][0]["function"]["name"], "get_weather");
        assert_eq!(v["tool_choice"], "auto");
    }

    #[test]
    fn parses_tool_calls_from_response() {
        let raw = serde_json::json!({
            "model": "gpt-4o",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {"name": "get_weather", "arguments": "{}"}
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        })
        .to_string();

        let parsed: CompResponse = serde_json::from_str(&raw).unwrap();
        let resp = OpenAiCompatibleProvider::parse_response(parsed);
        assert_eq!(resp.content, "");
        assert_eq!(resp.finish_reason, Some(FinishReason::ToolCalls));
        let calls = resp.tool_calls.expect("tool_calls present");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "get_weather");
    }

    #[test]
    fn parses_logprobs_from_response() {
        let raw = serde_json::json!({
            "model": "gpt-4o",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "Hi"
                },
                "finish_reason": "stop",
                "logprobs": {
                    "content": [{
                        "token": "Hi",
                        "logprob": -0.25,
                        "bytes": [72, 105],
                        "top_logprobs": [{
                            "token": "Hello",
                            "logprob": -0.5,
                            "bytes": [72, 101, 108, 108, 111]
                        }]
                    }]
                }
            }]
        })
        .to_string();

        let parsed: CompResponse = serde_json::from_str(&raw).unwrap();
        let resp = OpenAiCompatibleProvider::parse_response(parsed);
        assert_eq!(resp.content, "Hi");
        assert_eq!(resp.finish_reason, Some(FinishReason::Stop));

        let logprobs = resp.logprobs.expect("logprobs present");
        assert_eq!(logprobs.content.len(), 1);
        assert_eq!(logprobs.content[0].token, "Hi");
        assert_eq!(logprobs.content[0].logprob, -0.25);
        assert_eq!(logprobs.content[0].bytes, Some(vec![72, 105]));
        assert_eq!(logprobs.content[0].top_logprobs.len(), 1);
        assert_eq!(logprobs.content[0].top_logprobs[0].token, "Hello");
        assert_eq!(logprobs.content[0].top_logprobs[0].logprob, -0.5);
    }

    #[test]
    fn tool_message_serializes_with_id() {
        let comp = CompMessage::from(&Message::tool("call_1", "result"));
        let v = serde_json::to_value(&comp).unwrap();
        assert_eq!(v["role"], "tool");
        assert_eq!(v["content"], "result");
        assert_eq!(v["tool_call_id"], "call_1");
    }

    #[test]
    fn assistant_tool_call_message_omits_empty_content() {
        let msg = Message::assistant_tool_calls(vec![ToolCall {
            id: "call_1".to_string(),
            call_type: "function".to_string(),
            function: FunctionCall {
                name: "f".to_string(),
                arguments: "{}".to_string(),
            },
        }]);
        let comp = CompMessage::from(&msg);
        let v = serde_json::to_value(&comp).unwrap();
        assert!(v.get("content").is_none(), "content should be omitted");
        assert_eq!(v["tool_calls"][0]["id"], "call_1");
    }

    #[test]
    fn user_image_message_serializes_as_content_parts() {
        let msg = Message::user_with_parts(vec![
            ContentPart::text("what is this?"),
            ContentPart::image_url("https://example.com/cat.png"),
        ]);
        let comp = CompMessage::from(&msg);
        let v = serde_json::to_value(&comp).unwrap();
        assert_eq!(v["role"], "user");
        assert_eq!(v["content"][0]["type"], "text");
        assert_eq!(v["content"][0]["text"], "what is this?");
        assert_eq!(v["content"][1]["type"], "image_url");
        assert_eq!(
            v["content"][1]["image_url"]["url"],
            "https://example.com/cat.png"
        );
    }

    #[test]
    fn serializes_sampling_params_and_response_format() {
        let messages = vec![CompMessage::from(&Message::user("hi"))];
        let rf = ResponseFormat::json_object();
        let stop = vec!["STOP".to_string()];
        let metadata = serde_json::json!({"trace_id": "abc"});
        let body = CompChatRequest {
            model: "gpt-4o",
            messages: &messages,
            temperature: Some(0.2),
            max_tokens: Some(64),
            top_p: Some(0.9),
            stream: false,
            stream_options: None,
            tools: None,
            tool_choice: None,
            response_format: Some(&rf),
            stop: Some(stop.as_slice()),
            n: Some(2),
            seed: Some(7),
            presence_penalty: Some(0.5),
            frequency_penalty: Some(-0.25),
            logprobs: Some(true),
            top_logprobs: Some(3),
            parallel_tool_calls: Some(false),
            service_tier: Some("flex"),
            store: Some(true),
            metadata: Some(&metadata),
            user: Some("user-123"),
            reasoning_effort: None,
            extra: &HashMap::new(),
        };

        let v = serde_json::to_value(&body).unwrap();
        assert_eq!(v["response_format"]["type"], "json_object");
        assert_eq!(v["stop"][0], "STOP");
        assert_eq!(v["n"], 2);
        assert_eq!(v["seed"], 7);
        assert_eq!(v["presence_penalty"], 0.5);
        assert_eq!(v["frequency_penalty"], -0.25);
        assert_eq!(v["logprobs"], true);
        assert_eq!(v["top_logprobs"], 3);
        assert_eq!(v["parallel_tool_calls"], false);
        assert_eq!(v["service_tier"], "flex");
        assert_eq!(v["store"], true);
        assert_eq!(v["metadata"]["trace_id"], "abc");
        assert_eq!(v["user"], "user-123");
    }

    #[test]
    fn omits_unset_sampling_params() {
        let messages = vec![CompMessage::from(&Message::user("hi"))];
        let body = CompChatRequest {
            model: "gpt-4o",
            messages: &messages,
            temperature: None,
            max_tokens: None,
            top_p: None,
            stream: false,
            stream_options: None,
            tools: None,
            tool_choice: None,
            response_format: None,
            stop: None,
            n: None,
            seed: None,
            presence_penalty: None,
            frequency_penalty: None,
            logprobs: None,
            top_logprobs: None,
            parallel_tool_calls: None,
            service_tier: None,
            store: None,
            metadata: None,
            user: None,
            reasoning_effort: None,
            extra: &HashMap::new(),
        };

        let v = serde_json::to_value(&body).unwrap();
        assert!(v.get("response_format").is_none());
        assert!(v.get("stop").is_none());
        assert!(v.get("seed").is_none());
        assert!(v.get("n").is_none());
        assert!(v.get("presence_penalty").is_none());
        assert!(v.get("frequency_penalty").is_none());
        assert!(v.get("logprobs").is_none());
        assert!(v.get("top_logprobs").is_none());
        assert!(v.get("parallel_tool_calls").is_none());
        assert!(v.get("service_tier").is_none());
        assert!(v.get("store").is_none());
        assert!(v.get("metadata").is_none());
        assert!(v.get("user").is_none());
    }

    #[test]
    fn stream_accumulates_tool_call_fragments() {
        let mut tools = ToolCallAccumulator::default();
        let lines = [
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"get_weather","arguments":"{\"ci"}}]},"finish_reason":null}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"ty\":\"SF\"}"}}]},"finish_reason":null}]}"#,
            r#"data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
        ];
        let mut final_chunk = None;
        for line in lines {
            for chunk in parse_sse_line(&mut tools, line) {
                final_chunk = Some(chunk.unwrap());
            }
        }
        let chunk = final_chunk.expect("a terminal chunk");
        assert!(chunk.done);
        assert_eq!(chunk.finish_reason, Some(FinishReason::ToolCalls));
        let calls = chunk.tool_calls.expect("tool calls present");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].function.name, "get_weather");
        assert_eq!(calls[0].function.arguments, "{\"city\":\"SF\"}");
    }

    #[test]
    fn stream_without_tool_calls_has_none() {
        let mut tools = ToolCallAccumulator::default();
        let chunks = parse_sse_line(
            &mut tools,
            r#"data: {"choices":[{"delta":{"content":"hi"},"finish_reason":"stop"}]}"#,
        );
        let chunk = chunks.into_iter().next().unwrap().unwrap();
        assert_eq!(chunk.delta, "hi");
        assert!(chunk.done);
        assert!(chunk.tool_calls.is_none());
    }

    // DELTA-NULL regression: OpenAI-compatible endpoints (official OpenAI
    // semantics, llama.cpp, some vLLM builds, self-hosted proxies) send the
    // terminal frame as `{"delta":null,"finish_reason":"stop",...}`. A
    // non-optional `delta: CompDelta` rejected it with
    // `missing field \`delta\``, turning every streamed tool-calling turn
    // into `LlmError::Parse` / "Invalid response". The three tests below pin
    // the null / absent / normal shapes.
    #[test]
    fn stream_terminal_chunk_with_null_delta_parses() {
        let mut tools = ToolCallAccumulator::default();
        let chunks = parse_sse_line(
            &mut tools,
            r#"data: {"choices":[{"delta":null,"index":0,"finish_reason":"stop"}]}"#,
        );
        let chunk = chunks.into_iter().next().expect("one chunk").unwrap();
        assert_eq!(chunk.delta, "", "a null delta carries no content increment");
        assert!(chunk.done, "finish_reason present => done");
        assert_eq!(chunk.finish_reason, Some(FinishReason::Stop));
        assert!(chunk.tool_calls.is_none());
        assert!(chunk.thinking.is_none());
    }

    #[test]
    fn stream_delta_content_still_parses_with_option() {
        let mut tools = ToolCallAccumulator::default();
        let chunks = parse_sse_line(
            &mut tools,
            r#"data: {"choices":[{"delta":{"content":"hi"},"finish_reason":null}]}"#,
        );
        let chunk = chunks.into_iter().next().unwrap().unwrap();
        assert_eq!(chunk.delta, "hi", "normal increments must survive the fix");
        assert!(!chunk.done, "no finish_reason => not done");
    }

    #[test]
    fn stream_terminal_chunk_with_absent_delta_parses() {
        let mut tools = ToolCallAccumulator::default();
        let chunks = parse_sse_line(
            &mut tools,
            r#"data: {"choices":[{"finish_reason":"stop"}]}"#,
        );
        let chunk = chunks.into_iter().next().expect("one chunk").unwrap();
        assert_eq!(chunk.delta, "", "an absent delta means no increment");
        assert!(chunk.done, "finish_reason present => done");
        assert_eq!(chunk.finish_reason, Some(FinishReason::Stop));
    }

    #[test]
    fn compat_stream_malformed_data_returns_parse_error() {
        let mut tools = ToolCallAccumulator::default();
        let chunks = parse_sse_line(&mut tools, "data: {not valid json}");
        let err = chunks.into_iter().next().unwrap().unwrap_err();
        assert!(matches!(err, LlmError::Parse(_)));
        assert!(err.to_string().contains("stream chunk"));
    }

    #[test]
    fn compat_stream_ignores_non_data_lines() {
        let mut tools = ToolCallAccumulator::default();
        let chunks = parse_sse_line(&mut tools, ": keep-alive");
        assert!(chunks.is_empty());
    }

    #[test]
    fn stream_reasoning_delta_mapped_to_thinking() {
        let mut tools = ToolCallAccumulator::default();
        let chunks = parse_sse_line(
            &mut tools,
            r#"data: {"choices":[{"delta":{"reasoning":"thinking...","content":"hi"},"finish_reason":"stop"}]}"#,
        );
        let chunk = chunks.into_iter().next().unwrap().unwrap();
        assert_eq!(chunk.thinking.as_deref(), Some("thinking..."));
        assert_eq!(chunk.delta, "hi");
        assert!(chunk.done);
    }

    #[test]
    fn stream_reasoning_terminal_chunk_marks_thinking_done() {
        // FIX-R2 (FND-R2): Contract §7 (CONTRACTS.md L33) — providers that
        // map reasoning on the streaming path must mark the end of the
        // thinking stage with `StreamChunk.thinking_done = Some(true)` at
        // most once on the terminal chunk. OpenAI's o-series streams the
        // `reasoning` delta and then finishes with `finish_reason:"stop"`
        // on the same terminal chunk; the thinking stage ends there, so the
        // terminal chunk must carry `thinking_done: Some(true)`.
        let mut tools = ToolCallAccumulator::default();
        let chunks = parse_sse_line(
            &mut tools,
            r#"data: {"choices":[{"delta":{"reasoning":"thinking...","content":"hi"},"finish_reason":"stop"}]}"#,
        );
        let chunk = chunks.into_iter().next().unwrap().unwrap();
        assert!(chunk.done, "terminal chunk must be done");
        assert_eq!(
            chunk.thinking_done,
            Some(true),
            "terminal chunk that surfaced thinking must mark thinking_done = Some(true) per Contract §7"
        );
    }

    // ── AUD#1 / AUD#3 / AUD#7 (C-AUD-COMPAT) regression tests ──────────

    #[test]
    fn stream_terminal_after_thinking_marks_sticky_done() {
        // AUD#1 (severe): the terminal frame of a reasoning stream carries
        // no thinking increment of its own — upstreams finish reasoning
        // streams with `{"delta":null,"finish_reason":"stop"}`. `thinking_done`
        // is sticky: it reports whether THINKING WAS SEEN EARLIER in this
        // stream (architect ruling 2026-11-20: sticky terminal state). The
        // old same-frame expression returned `Some(false)` here — a lie
        // about the stream that had just streamed thinking.
        let mut tools = ToolCallAccumulator::default();
        let reasoning_frame = r#"data: {"choices":[{"delta":{"reasoning":"hmm"}}]}"#;
        let terminal_frame = r#"data: {"choices":[{"delta":null,"finish_reason":"stop"}]}"#;
        let first = parse_sse_line(&mut tools, reasoning_frame);
        assert!(
            first
                .iter()
                .all(|c| !c.as_ref().unwrap().thinking_done.is_some()),
            "an increment frame is not terminal; thinking_done must be None, got {first:?}"
        );
        let second = parse_sse_line(&mut tools, terminal_frame);
        let chunk = second
            .into_iter()
            .next()
            .expect("terminal frame yields one chunk")
            .unwrap();
        assert!(chunk.done);
        assert_eq!(
            chunk.thinking_done,
            Some(true),
            "a terminal frame after a thinking increment must mark the end of the thinking stage (sticky), got {:?}",
            chunk.thinking_done
        );
    }

    #[test]
    fn stream_no_thinking_terminal_done_is_false() {
        // AUD#1, other side of the sticky semantics: a stream that never
        // streamed a thinking increment must NOT report a thinking stage on
        // its terminal frame. A plain content stream ends with
        // `thinking_done == Some(false)` (not None and not true).
        let mut tools = ToolCallAccumulator::default();
        let content_frame = r#"data: {"choices":[{"delta":{"content":"hi"}}]}"#;
        let terminal_frame = r#"data: {"choices":[{"delta":null,"finish_reason":"stop"}]}"#;
        let first = parse_sse_line(&mut tools, content_frame);
        assert!(
            first
                .iter()
                .all(|c| !c.as_ref().unwrap().thinking_done.is_some()),
            "non-terminal frames never carry thinking_done, got {first:?}"
        );
        let second = parse_sse_line(&mut tools, terminal_frame);
        let chunk = second
            .into_iter()
            .next()
            .expect("terminal frame yields one chunk")
            .unwrap();
        assert!(chunk.done);
        assert_eq!(
            chunk.thinking_done,
            Some(false),
            "a terminal frame in a stream with no thinking increments must carry thinking_done = Some(false)"
        );
    }

    #[test]
    fn stream_error_frame_surfaces_llm_error() {
        // AUD#3 (high): an upstream may end a 200 stream with an in-band
        // error frame — measured wire shape on quota exhaustion:
        // `data: {"error":{"message":"quota exceeded","type":"insufficient_quota","code":429},"choices":[]}`.
        // The empty-choices arm dropped it as a keepalive, so the stream
        // ended cleanly with the failure vanished. It must surface as
        // LlmError::Api (status from the numeric `code` when in the HTTP
        // error range), per the stream contract "No silent drops".
        let mut tools = ToolCallAccumulator::default();
        let chunks = parse_sse_line(
            &mut tools,
            r#"data: {"error":{"message":"quota exceeded","type":"insufficient_quota","code":429},"choices":[]}"#,
        );
        let err = chunks
            .into_iter()
            .next()
            .expect("error frame yields one item")
            .expect_err("an in-band error frame must surface as Err");
        match &err {
            LlmError::Api {
                status: 429,
                message,
            } => {
                assert!(
                    message.contains("quota exceeded"),
                    "the upstream error message must be preserved, got {message}"
                );
            }
            other => panic!("expected LlmError::Api {{ status: 429, .. }}, got {other:?}"),
        }
    }

    #[test]
    fn stream_tool_call_delta_with_huge_index_is_rejected() {
        // AUD#7 (high): the accumulator grows by the upstream-supplied
        // `index`, so an unbounded grow let a single frame size the
        // allocation (index 1000000 ordered a million builders; usize::MAX
        // overflowed). The index is now bounded
        // (MAX_TOOL_CALL_INDEX); the hostile frame surfaces as
        // Err(LlmError::Parse) — not a panic, not an OOM, not an ok chunk.
        let mut tools = ToolCallAccumulator::default();
        let chunks = parse_sse_line(
            &mut tools,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":1000000,"id":"x","function":{"name":"f","arguments":""}}]},"finish_reason":null}]}"#,
        );
        let err = chunks
            .into_iter()
            .next()
            .expect("huge-index frame yields one item")
            .expect_err("a tool_call index beyond the bound must be an Err");
        assert!(
            matches!(err, LlmError::Parse(_)),
            "expected LlmError::Parse, got {err:?}"
        );
        assert!(
            tools.builders.is_empty(),
            "nothing may be allocated for a rejected index, got {} builders",
            tools.builders.len()
        );
    }

    #[test]
    fn stream_empty_choices_without_usage_stays_keepalive() {
        // AUD#3 guard: the #3 fix must not eat the choice-less keepalive
        // semantics. A frame with no `error`, no usage and no choices still
        // produces no chunks and no Err (the old empty-choices behaviour).
        let mut tools = ToolCallAccumulator::default();
        let chunks = parse_sse_line(&mut tools, r#"data: {"choices":[],"created":1}"#);
        assert!(
            chunks.is_empty(),
            "a keepalive frame with no error and no usage must stay invisible, got {chunks:?}"
        );
    }

    #[test]
    fn compat_stream_done_still_returns_done_chunk() {
        let mut tools = ToolCallAccumulator::default();
        let chunks = parse_sse_line(&mut tools, "data: [DONE]");
        let chunk = chunks.into_iter().next().unwrap().unwrap();
        assert!(chunk.done);
    }

    #[tokio::test]
    async fn stream_done_without_finish_reason_flushes_tool_calls() {
        // Regression (stream-close flush): some OpenAI-compatible upstreams —
        // observed on xunfei spark-x2.6 with large tool-call arguments — end
        // the stream with `[DONE]` without ever sending a finish_reason
        // frame. The arguments are already fully accumulated in the
        // accumulator; the `[DONE]` done chunk must carry the completed
        // calls instead of leaving them swallowed inside the library (the
        // same request with stream=false returns finish_reason "tool_calls"
        // normally). Before the fix the calls were silently dropped.
        let lines = vec![
            Ok(r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"get_weather","arguments":"{\"ci"}}]},"finish_reason":null}]}"#.to_string()),
            Ok(r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"ty\":\"SF\"}"}}]},"finish_reason":null}]}"#.to_string()),
            Ok("data: [DONE]".to_string()),
        ];
        let mut chunks: Vec<Result<StreamChunk>> = sse_chunk_stream(futures::stream::iter(lines))
            .collect()
            .await;
        assert_eq!(
            chunks.len(),
            3,
            "two delta chunks plus the [DONE] chunk, got {chunks:?}"
        );
        let done = chunks.pop().unwrap().expect("[DONE] chunk is Ok");
        assert!(done.done);
        assert!(
            done.finish_reason.is_none(),
            "an absent finish_reason must stay absent (ERR-003: never invent a reason)"
        );
        let calls = done.tool_calls.expect("tool calls flushed on [DONE]");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].function.name, "get_weather");
        assert_eq!(calls[0].function.arguments, "{\"city\":\"SF\"}");
    }

    #[tokio::test]
    async fn sse_chunk_stream_flushes_pending_tool_calls_on_clean_eof() {
        // Clean end-of-stream without any terminal frame: no finish_reason,
        // no `[DONE]` — the line stream just ends. The pending calls must
        // surface on a final done chunk (and only one), with the absent
        // finish_reason left absent.
        let lines = vec![
            Ok(r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_2","function":{"name":"read","arguments":"{\""}}]},"finish_reason":null}]}"#.to_string()),
            Ok(r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"path\":\"x\"}"}}]},"finish_reason":null}]}"#.to_string()),
        ];
        let mut chunks: Vec<Result<StreamChunk>> = sse_chunk_stream(futures::stream::iter(lines))
            .collect()
            .await;
        assert_eq!(
            chunks.len(),
            3,
            "two delta chunks plus exactly one flush chunk, got {chunks:?}"
        );
        let flushed = chunks.pop().unwrap().expect("flush chunk is Ok");
        assert!(flushed.done);
        assert!(flushed.finish_reason.is_none());
        let calls = flushed.tool_calls.expect("pending calls flushed at EOF");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_2");
        assert_eq!(calls[0].function.name, "read");
        assert_eq!(calls[0].function.arguments, "{\"path\":\"x\"}");
    }

    #[tokio::test]
    async fn sse_chunk_stream_error_line_does_not_flush() {
        // A transport error mid-stream must terminate without flushing the
        // half-accumulated arguments: an error stays an error, and salvage of
        // partial tool calls is the caller's job, not a fabricated success
        // terminal.
        let lines = vec![
            Ok(r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"write","arguments":"{\"pa"}}]},"finish_reason":null}]}"#.to_string()),
            Err(LlmError::Stream("connection reset mid-stream".to_string())),
        ];
        let chunks: Vec<Result<StreamChunk>> = sse_chunk_stream(futures::stream::iter(lines))
            .collect()
            .await;
        assert!(
            chunks.iter().any(|c| c.is_err()),
            "the transport error must be surfaced, got {chunks:?}"
        );
        assert!(
            chunks
                .iter()
                .all(|c| c.is_err() || c.as_ref().unwrap().tool_calls.is_none()),
            "no chunk may carry flushed tool calls after an error, got {chunks:?}"
        );
        assert!(
            chunks
                .iter()
                .all(|c| c.is_err() || !c.as_ref().unwrap().done),
            "no success done chunk may follow an error, got {chunks:?}"
        );
    }

    #[tokio::test]
    async fn sse_chunk_stream_parse_error_does_not_flush() {
        // Same contract as the transport error, reached through a malformed
        // JSON line: parse errors surface as Err(LlmError::Parse), the stream
        // ends, and the half-accumulated arguments stay unflushed.
        let lines = vec![
            Ok(r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"write","arguments":"{\"pa"}}]},"finish_reason":null}]}"#.to_string()),
            Ok("data: {not valid json}".to_string()),
        ];
        let chunks: Vec<Result<StreamChunk>> = sse_chunk_stream(futures::stream::iter(lines))
            .collect()
            .await;
        assert!(
            chunks.iter().any(|c| matches!(c, Err(LlmError::Parse(_)))),
            "the parse error must be surfaced, got {chunks:?}"
        );
        assert!(
            chunks
                .iter()
                .all(|c| c.is_err() || c.as_ref().unwrap().tool_calls.is_none()),
            "no chunk may carry flushed tool calls after a parse error, got {chunks:?}"
        );
    }

    #[tokio::test]
    async fn sse_chunk_stream_parse_error_then_done_does_not_flush() {
        // The distinctive error gate on flush point 1: a `[DONE]` that arrives
        // after a surfaced error must stay bare — flushing there would
        // launder half-accumulated arguments into a clean terminal.
        let lines = vec![
            Ok(r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"write","arguments":"{\"pa"}}]},"finish_reason":null}]}"#.to_string()),
            Ok("data: {not valid json}".to_string()),
            Ok("data: [DONE]".to_string()),
        ];
        let chunks: Vec<Result<StreamChunk>> = sse_chunk_stream(futures::stream::iter(lines))
            .collect()
            .await;
        assert_eq!(
            chunks.len(),
            3,
            "delta chunk, parse error, then bare [DONE], got {chunks:?}"
        );
        assert!(matches!(chunks[1], Err(LlmError::Parse(_))));
        let done = chunks[2].as_ref().unwrap();
        assert!(done.done);
        assert!(
            done.tool_calls.is_none(),
            "[DONE] after a surfaced error must not flush partial arguments, got {done:?}"
        );
        assert!(
            done.finish_reason.is_none(),
            "the [DONE] chunk invents no finish_reason"
        );
    }

    #[tokio::test]
    async fn sse_chunk_stream_well_behaved_stream_unchanged() {
        // A well-behaved stream (finish_reason present, then `[DONE]`) must
        // behave exactly as before: one terminal chunk carrying the calls and
        // the finish_reason, `[DONE]` draining nothing extra, and no
        // end-of-stream flush chunk appended.
        let lines = vec![
            Ok(r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"get_weather","arguments":"{\"ci"}}]},"finish_reason":null}]}"#.to_string()),
            Ok(r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"ty\":\"SF\"}"}}]},"finish_reason":null}]}"#.to_string()),
            Ok(r#"data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#.to_string()),
            Ok("data: [DONE]".to_string()),
        ];
        let chunks: Vec<Result<StreamChunk>> = sse_chunk_stream(futures::stream::iter(lines))
            .collect()
            .await;
        assert_eq!(
            chunks.len(),
            4,
            "two deltas + terminal chunk + [DONE] chunk, nothing extra, got {chunks:?}"
        );
        let terminal = chunks[2].as_ref().expect("terminal chunk is Ok");
        assert!(terminal.done);
        assert_eq!(terminal.finish_reason, Some(FinishReason::ToolCalls));
        assert!(terminal.tool_calls.is_some());
        let done = chunks[3].as_ref().expect("[DONE] chunk is Ok");
        assert!(done.done);
        assert!(done.tool_calls.is_none(), "[DONE] must drain nothing extra");
    }

    // ── embedding contract tests ──────────────────────────────────

    use std::sync::{Arc, Mutex};

    #[test]
    fn embed_request_serializes_correctly() {
        let req = EmbeddingRequest::new("model", "hi")
            .with_dimensions(1024)
            .with_user("user-1")
            .with_extra("task", "search");
        let body = CompEmbeddingRequest {
            model: &req.model,
            input: &req.input,
            dimensions: req.dimensions,
            user: req.user.as_deref(),
            extra: &req.extra,
        };
        let v = serde_json::to_value(&body).expect("embedding request should serialize");
        assert_eq!(v["model"], "model");
        assert_eq!(v["input"][0], "hi");
        assert_eq!(v["dimensions"], 1024);
        assert_eq!(v["user"], "user-1");
        assert_eq!(v["task"], "search");
    }

    #[test]
    fn embed_response_parses_correctly() {
        let json = r#"{"model":"text-embedding-3-small","data":[{"index":0,"embedding":[0.1,0.2]},{"index":1,"embedding":[0.3,0.4]}],"usage":{"prompt_tokens":5,"total_tokens":5}}"#;
        let parsed: CompEmbeddingResponse =
            serde_json::from_str(json).expect("embedding response should parse");
        assert_eq!(parsed.model.as_deref(), Some("text-embedding-3-small"));
        assert_eq!(parsed.data.len(), 2);
        assert_eq!(parsed.data[0].index, 0);
        assert_eq!(parsed.data[0].embedding, vec![0.1_f32, 0.2]);
        assert_eq!(parsed.data[1].index, 1);
        assert_eq!(parsed.data[1].embedding, vec![0.3_f32, 0.4]);
        let usage = parsed.usage.expect("usage present");
        assert_eq!(usage.prompt_tokens, 5);
        assert_eq!(usage.total_tokens, 5);
    }

    #[test]
    fn embed_response_falls_back_to_request_model() {
        let json = r#"{"data":[]}"#;
        let parsed: CompEmbeddingResponse =
            serde_json::from_str(json).expect("should parse without model");
        assert!(parsed.model.is_none());
        let resp = EmbeddingResponse {
            model: parsed.model.unwrap_or_else(|| "fallback".to_string()),
            data: vec![],
            usage: None,
        };
        assert_eq!(resp.model, "fallback");
    }

    #[test]
    fn embed_error_response_parses_message() {
        let json = r#"{"error":{"message":"bad input"}}"#;
        let parsed: CompErrorBody = serde_json::from_str(json).expect("error body should parse");
        assert!(parsed.error.message.contains("bad input"));
    }

    #[tokio::test]
    async fn openai_compatible_embed_via_lmrs_client_strips_prefix() {
        use crate::LmrsClient;
        let llm = LmrsClient::new();
        let seen: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let seen_clone = Arc::clone(&seen);

        struct CapturingEmbedProvider {
            seen: Arc<Mutex<Option<String>>>,
        }

        #[async_trait]
        impl Provider for CapturingEmbedProvider {
            async fn chat(&self, _: &ChatRequest) -> Result<ChatResponse> {
                unimplemented!()
            }
            async fn stream(
                &self,
                _: &ChatRequest,
            ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
                unimplemented!()
            }
            async fn embed(&self, req: &EmbeddingRequest) -> Result<EmbeddingResponse> {
                *self.seen.lock().unwrap() = Some(req.model.clone());
                Ok(EmbeddingResponse {
                    model: req.model.clone(),
                    data: vec![],
                    usage: None,
                })
            }
        }

        llm.set_custom(
            "openai",
            Arc::new(CapturingEmbedProvider { seen: seen_clone }),
        )
        .await;

        llm.embed("openai/text-embedding-3-small", "hello")
            .await
            .expect("embed should succeed via LmrsClient");

        assert_eq!(
            seen.lock().unwrap().as_deref(),
            Some("text-embedding-3-small")
        );
    }

    // ── HTTP integration tests (axum, requires proxy feature) ────

    #[cfg(feature = "proxy")]
    #[tokio::test]
    async fn openai_compatible_embed_posts_to_embeddings_endpoint() {
        let embed_ok = axum::Router::new().route(
            "/embeddings",
            axum::routing::post(
                |headers: axum::http::HeaderMap,
                 axum::Json(body): axum::Json<serde_json::Value>| async move {
                    assert!(headers.get("authorization").is_some());
                    assert_eq!(body["model"], "text-embedding-3-small");
                    assert_eq!(body["input"][0], "hello");
                    assert_eq!(body["input"][1], "world");
                    axum::Json(serde_json::json!({"data":[],"model":"m"}))
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{}", addr);
        tokio::spawn(async move { axum::serve(listener, embed_ok).await.unwrap() });
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let config = ProviderConfig::new("sk-test").with_base_url(&url);
        let provider = OpenAiCompatibleProvider::new(config, []);
        let req = EmbeddingRequest::batch("text-embedding-3-small", ["hello", "world"]);
        provider.embed(&req).await.expect("embed should succeed");
    }

    #[cfg(feature = "proxy")]
    #[tokio::test]
    async fn openai_compatible_embed_sends_dimensions_user_and_extra() {
        let embed_ok = axum::Router::new().route(
            "/embeddings",
            axum::routing::post(
                |axum::Json(body): axum::Json<serde_json::Value>| async move {
                    assert_eq!(body["dimensions"], 1024);
                    assert_eq!(body["user"], "u");
                    assert_eq!(body["task"], "search");
                    axum::Json(serde_json::json!({"data":[],"model":"m"}))
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{}", addr);
        tokio::spawn(async move { axum::serve(listener, embed_ok).await.unwrap() });
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let config = ProviderConfig::new("sk-test").with_base_url(&url);
        let provider = OpenAiCompatibleProvider::new(config, []);
        let req = EmbeddingRequest::new("m", "hi")
            .with_dimensions(1024)
            .with_user("u")
            .with_extra("task", "search");
        provider.embed(&req).await.expect("embed should succeed");
    }

    #[cfg(feature = "proxy")]
    #[tokio::test]
    async fn openai_compatible_embed_maps_api_error() {
        let embed_400 = axum::Router::new().route(
            "/embeddings",
            axum::routing::post(|| async {
                (
                    axum::http::StatusCode::BAD_REQUEST,
                    axum::Json(serde_json::json!({"error":{"message":"bad input"}})),
                )
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{}", addr);
        tokio::spawn(async move { axum::serve(listener, embed_400).await.unwrap() });
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let config = ProviderConfig::new("sk-test").with_base_url(&url);
        let provider = OpenAiCompatibleProvider::new(config, []);
        let req = EmbeddingRequest::new("m", "hi");
        let err = provider.embed(&req).await.unwrap_err();
        assert!(
            matches!(&err, LlmError::Api { status: 400, message } if message.contains("bad input")),
            "expected Api 400, got: {err:?}"
        );
    }

    #[cfg(feature = "proxy")]
    #[tokio::test]
    async fn openai_compatible_stream_flushes_tool_calls_without_finish_reason() {
        // Regression (stream-close flush), end-to-end through `stream()`:
        // an upstream that sends tool-call argument deltas and then `[DONE]`
        // — but never a finish_reason frame — must still surface the completed
        // tool calls on the done chunk. Wire shape observed on xunfei
        // spark-x2.6; before the fix the accumulated arguments (tens of
        // kilobytes in practice) were swallowed inside the accumulator.
        let sse = [
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"write\",\"arguments\":\"{\\\"path\\\":\"}}]},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"a.rs\\\"}\"}}]},\"finish_reason\":null}]}\n\n",
            "data: [DONE]\n\n",
        ]
        .concat();
        let app = axum::Router::new().route(
            "/chat/completions",
            axum::routing::post(move || async move {
                (
                    [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
                    axum::body::Body::from(sse),
                )
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{}", addr);
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let config = ProviderConfig::new("sk-test").with_base_url(&url);
        let provider = OpenAiCompatibleProvider::new(config, []);
        let req = ChatRequest::new("spark-x2.6", "hi");
        let mut stream = provider.stream(&req).await.expect("stream established");
        let mut done_chunk = None;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.expect("stream items must be Ok in this fixture");
            if chunk.done {
                done_chunk = Some(chunk);
            }
        }
        let done = done_chunk.expect("a done chunk must terminate the stream");
        assert!(done.finish_reason.is_none());
        let calls = done.tool_calls.expect("tool calls flushed on [DONE]");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].function.name, "write");
        assert_eq!(calls[0].function.arguments, "{\"path\":\"a.rs\"}");
    }

    #[cfg(feature = "proxy")]
    #[tokio::test]
    async fn openai_provider_embed_strips_prefix_with_real_wrapper() {
        use crate::providers::openai::OpenAIProvider;

        let embed_ok = axum::Router::new().route(
            "/embeddings",
            axum::routing::post(
                |axum::Json(body): axum::Json<serde_json::Value>| async move {
                    assert_eq!(body["model"], "text-embedding-3-small");
                    axum::Json(serde_json::json!({"data":[],"model":"text-embedding-3-small"}))
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{}", addr);
        tokio::spawn(async move { axum::serve(listener, embed_ok).await.unwrap() });
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let config = ProviderConfig::new("sk-test").with_base_url(&url);
        let provider: Arc<dyn Provider> = Arc::new(OpenAIProvider::new(config));

        use crate::LmrsClient;
        let llm = LmrsClient::new();
        llm.set_custom("openai", provider).await;
        llm.embed("openai/text-embedding-3-small", "hello")
            .await
            .expect("embed should succeed via real OpenAIProvider");
    }

    // --- REA-003: OpenAI-compatible reasoning isolation ---

    fn wrapper_provider() -> OpenAiCompatibleProvider {
        // Same construction path as DeepSeek/Moonshot/OpenRouter: no
        // `with_reasoning(true)` call.
        OpenAiCompatibleProvider::new(ProviderConfig::new("sk-test"), [])
    }

    #[tokio::test]
    async fn openai_chat_with_thinking_is_unsupported_before_network() {
        let provider =
            crate::providers::openai::OpenAIProvider::new(ProviderConfig::new("sk-test"));
        let req = ChatRequest::new("gpt-4o", "hi").with_thinking(ThinkingConfig::Enabled {
            budget_tokens: None,
        });
        let err = provider.chat(&req).await.unwrap_err();
        assert!(
            matches!(err, LlmError::Unsupported { .. }),
            "non-stream chat with reasoning must be Unsupported, got {err:?}"
        );
    }

    #[tokio::test]
    async fn openai_stream_with_budget_is_unsupported() {
        let provider =
            crate::providers::openai::OpenAIProvider::new(ProviderConfig::new("sk-test"));
        let req = ChatRequest::new("o3", "hi").with_thinking(ThinkingConfig::Enabled {
            budget_tokens: Some(1000),
        });
        match provider.stream(&req).await {
            Err(LlmError::Unsupported { .. }) => {}
            Err(other) => panic!(
                "budget_tokens has no OpenAI equivalent (O-5), expected Unsupported, got {other:?}"
            ),
            Ok(_) => panic!("expected Unsupported"),
        }
    }

    #[tokio::test]
    async fn wrapper_chat_and_stream_with_thinking_are_unsupported() {
        // Same shared compat path used by DeepSeek/Moonshot/OpenRouter.
        let provider = wrapper_provider();
        let req = ChatRequest::new("some-model", "hi").with_thinking(ThinkingConfig::Enabled {
            budget_tokens: None,
        });
        let chat_err = provider.chat(&req).await.unwrap_err();
        assert!(matches!(chat_err, LlmError::Unsupported { .. }));
        match provider.stream(&req).await {
            Err(LlmError::Unsupported { .. }) => {}
            Err(other) => panic!("wrapper stream must be Unsupported, got {other:?}"),
            Ok(_) => panic!("expected Unsupported"),
        }
    }

    #[tokio::test]
    async fn wrapper_disabled_thinking_is_not_blocked() {
        let provider = wrapper_provider();
        let req = ChatRequest::new("some-model", "hi").with_thinking(ThinkingConfig::Disabled);
        let effort = provider.reasoning_effort(&req, true).unwrap();
        assert!(effort.is_none());
        let req = ChatRequest::new("some-model", "hi");
        let effort = provider.reasoning_effort(&req, false).unwrap();
        assert!(effort.is_none());
    }

    #[test]
    fn openai_stream_thinking_sends_reasoning_effort() {
        let provider = wrapper_provider().with_reasoning(true);
        let req = ChatRequest::new("o3", "hi").with_thinking(ThinkingConfig::Enabled {
            budget_tokens: None,
        });
        let effort = provider.reasoning_effort(&req, true).unwrap();
        assert_eq!(effort, Some("medium"));
        let messages = vec![CompMessage::from(&Message::user("hi"))];
        let body = OpenAiCompatibleProvider::build_body(&req, &messages, true, effort);
        let v = serde_json::to_value(&body).unwrap();
        assert_eq!(v["reasoning_effort"], "medium");
    }

    #[test]
    fn disabled_thinking_omits_reasoning_effort() {
        let messages = vec![CompMessage::from(&Message::user("hi"))];
        let req = ChatRequest::new("gpt-4o", "hi").with_thinking(ThinkingConfig::Disabled);
        let body = OpenAiCompatibleProvider::build_body(&req, &messages, true, None);
        let v = serde_json::to_value(&body).unwrap();
        assert!(v.get("reasoning_effort").is_none());
    }

    #[test]
    fn stream_reasoning_content_delta_mapped_to_thinking() {
        // O-1 tolerant parsing: `reasoning_content` (official o-series chat
        // completions name) and `reasoning` (M2-16 name) both map to thinking.
        let mut tools = ToolCallAccumulator::default();
        let chunks = parse_sse_line(
            &mut tools,
            r#"data: {"choices":[{"delta":{"reasoning_content":"thinking...","content":"hi"},"finish_reason":null}]}"#,
        );
        let chunk = chunks.into_iter().next().unwrap().unwrap();
        assert_eq!(chunk.thinking.as_deref(), Some("thinking..."));
        assert_eq!(chunk.delta, "hi");
    }

    #[test]
    fn usage_translates_cache_and_reasoning_tokens() {
        let raw = serde_json::json!({
            "model": "o3",
            "choices": [{ "message": { "role": "assistant", "content": "ok" }, "finish_reason": "stop" }],
            "usage": {
                "prompt_tokens": 100,
                "completion_tokens": 20,
                "total_tokens": 120,
                "reasoning_tokens": 8,
                "prompt_tokens_details": { "cached_tokens": 5 }
            }
        })
        .to_string();
        let parsed: CompResponse = serde_json::from_str(&raw).unwrap();
        let resp = OpenAiCompatibleProvider::parse_response(parsed);
        let u = resp.usage.unwrap();
        assert_eq!(u.cache_read_tokens, Some(5));
        assert_eq!(u.reasoning_tokens, Some(8));
        assert_eq!(u.prompt_tokens, 100);
    }

    #[test]
    fn usage_missing_cache_fields_stay_none() {
        let raw = serde_json::json!({
            "model": "gpt-4o",
            "choices": [{ "message": { "role": "assistant", "content": "ok" }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15 }
        })
        .to_string();
        let parsed: CompResponse = serde_json::from_str(&raw).unwrap();
        let resp = OpenAiCompatibleProvider::parse_response(parsed);
        let u = resp.usage.unwrap();
        assert_eq!(u.cache_read_tokens, None);
        assert_eq!(u.reasoning_tokens, None);
    }
}
