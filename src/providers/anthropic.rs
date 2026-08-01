//! Anthropic Claude API provider.

use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;

use crate::providers::http::{build_http_client, REQUEST_TIMEOUT};
use crate::providers::stream_util::line_stream;
use crate::providers::{LlmError, Provider, ProviderConfig, Result};
use crate::types::{
    ChatRequest, ChatResponse, Content, ContentPart, FinishReason, FunctionCall, StreamChunk,
    ThinkingConfig, Tool, ToolCall, ToolChoice, Usage,
};

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com/v1";

pub struct AnthropicProvider {
    client: Client,
    api_key: String,
    base_url: String,
}

impl AnthropicProvider {
    pub fn new(config: ProviderConfig) -> Self {
        let timeout = config
            .timeout_secs
            .map(Duration::from_secs)
            .or(Some(REQUEST_TIMEOUT));
        Self {
            client: build_http_client(timeout, config.custom_headers.as_ref()),
            api_key: config.api_key,
            base_url: config
                .base_url
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
        }
    }
}

// --- Anthropic API types ---

#[derive(Serialize)]
struct AnthropicRequest<'a> {
    model: &'a str,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop_sequences: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<AnthropicToolChoice>,
    /// REA-002: extended thinking. `Enabled{budget_tokens: None}` is rejected
    /// before any network call — the Anthropic API requires `budget_tokens`.
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<AnthropicThinking>,
    stream: bool,
}

/// Claude `thinking` parameter. Maps from the frozen `ThinkingConfig`; the
/// `adaptive` type (newer SDK) is not expressible in the 0.1.3 contract (O-2).
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicThinking {
    Enabled { budget_tokens: u64 },
}

/// Map the frozen `ThinkingConfig` to Claude's `thinking` parameter.
///
/// - `Disabled` → omitted (API default);
/// - `Enabled { budget_tokens: Some(n) }` → `{"type":"enabled","budget_tokens":n}`;
/// - `Enabled { budget_tokens: None }` → `LlmError::Unsupported` — the Anthropic
///   API requires `budget_tokens` when thinking is enabled, so omitting it would
///   either produce an invalid request or silently pick an arbitrary budget
///   (SPCC §6.1: no silent degradation of an explicitly set field).
fn thinking_to_anthropic(cfg: &ThinkingConfig) -> Result<Option<AnthropicThinking>> {
    match cfg {
        ThinkingConfig::Disabled => Ok(None),
        ThinkingConfig::Enabled {
            budget_tokens: Some(budget),
        } => Ok(Some(AnthropicThinking::Enabled {
            budget_tokens: *budget,
        })),
        ThinkingConfig::Enabled {
            budget_tokens: None,
        } => Err(LlmError::Unsupported {
            feature: "reasoning".to_string(),
            message: "Anthropic extended thinking requires budget_tokens".to_string(),
        }),
    }
}

/// True when reasoning is enabled — used by the non-stream `chat` gate
/// (`ChatResponse` cannot carry reasoning in 0.1.3, SPCC §6.3).
fn thinking_enabled(cfg: &Option<ThinkingConfig>) -> bool {
    matches!(cfg, Some(ThinkingConfig::Enabled { .. }))
}

#[derive(Serialize)]
struct AnthropicMessage {
    role: String,
    content: AnthropicMessageContent,
}

/// A Claude message body is either a plain string (text-only, the common case)
/// or an array of typed content blocks (used for images, tool calls, and tool
/// results).
#[derive(Serialize)]
#[serde(untagged)]
enum AnthropicMessageContent {
    Text(String),
    Blocks(Vec<AnthropicContentBlock>),
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicContentBlock {
    Text {
        text: String,
    },
    Image {
        source: AnthropicImageSource,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
    },
}

/// Claude accepts images either as inline base64 data or, on recent API
/// versions, by URL. Data URLs are decomposed into a base64 source; any other
/// URL is passed through as a `url` source.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicImageSource {
    Base64 { media_type: String, data: String },
    Url { url: String },
}

/// An Anthropic tool definition. Claude uses `input_schema` (JSON Schema)
/// where the OpenAI wire format uses `function.parameters`.
#[derive(Serialize)]
struct AnthropicTool {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    input_schema: serde_json::Value,
}

/// Claude's `tool_choice` object. `auto` / `any` / `none` are bare modes;
/// `tool` forces a specific function by name.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicToolChoice {
    Auto,
    Any,
    None,
    Tool { name: String },
}

/// Map llmrust (OpenAI-style) tools to Claude tool definitions.
fn to_anthropic_tools(tools: &[Tool]) -> Vec<AnthropicTool> {
    tools
        .iter()
        .map(|t| AnthropicTool {
            name: t.function.name.clone(),
            description: t.function.description.clone(),
            input_schema: t.function.parameters.clone(),
        })
        .collect()
}

/// Map llmrust [`ToolChoice`] to Claude's `tool_choice`. `required` becomes
/// `any`; a forced function becomes `tool`.
fn to_anthropic_tool_choice(choice: &ToolChoice) -> AnthropicToolChoice {
    match choice {
        ToolChoice::Mode(mode) => match mode.as_str() {
            "required" => AnthropicToolChoice::Any,
            "none" => AnthropicToolChoice::None,
            _ => AnthropicToolChoice::Auto,
        },
        ToolChoice::Function { function, .. } => AnthropicToolChoice::Tool {
            name: function.name.clone(),
        },
    }
}

/// Map an llmrust [`Content`] into Claude's message content. Text stays a
/// plain string; mixed text/image parts become typed content blocks.
fn to_anthropic_content(content: &Content) -> AnthropicMessageContent {
    match content {
        Content::Text(text) => AnthropicMessageContent::Text(text.clone()),
        Content::Parts(parts) => {
            let mut blocks = Vec::new();
            for part in parts {
                match part {
                    ContentPart::Text { text } => {
                        blocks.push(AnthropicContentBlock::Text { text: text.clone() });
                    }
                    ContentPart::ImageUrl { image_url } => {
                        let source = anthropic_image_source(&image_url.url);
                        blocks.push(AnthropicContentBlock::Image { source });
                    }
                }
            }
            AnthropicMessageContent::Blocks(blocks)
        }
    }
}

/// Build a Claude image source from a URL. `data:` URLs are split into their
/// media type and base64 payload; everything else becomes a URL source.
fn anthropic_image_source(url: &str) -> AnthropicImageSource {
    if let Some((meta, data)) = url
        .strip_prefix("data:")
        .and_then(|rest| rest.split_once(','))
    {
        let media_type = meta
            .split(';')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("image/png")
            .to_string();
        return AnthropicImageSource::Base64 {
            media_type,
            data: data.to_string(),
        };
    }
    AnthropicImageSource::Url {
        url: url.to_string(),
    }
}

/// Anthropic reports tool calls via the `tool_use` stop reason; normalize it
/// to the cross-provider [`FinishReason::ToolCalls`] so callers can branch uniformly.
fn normalize_stop_reason(reason: String) -> FinishReason {
    if reason == "tool_use" {
        FinishReason::ToolCalls
    } else {
        FinishReason::from(reason)
    }
}

/// Flush accumulated tool results into a single `user` message. Claude requires
/// all tool results for one assistant turn to arrive together in one user turn.
fn flush_tool_results(
    messages: &mut Vec<AnthropicMessage>,
    pending: &mut Vec<AnthropicContentBlock>,
) {
    if !pending.is_empty() {
        messages.push(AnthropicMessage {
            role: "user".to_string(),
            content: AnthropicMessageContent::Blocks(std::mem::take(pending)),
        });
    }
}

/// Split request messages into Claude's separate `system` prompt and the
/// `messages` array. System content is flattened to text; assistant tool calls
/// become `tool_use` blocks and `tool` results are folded into `user` turns as
/// `tool_result` blocks (consecutive results are merged into one user turn).
fn split_messages(req: &ChatRequest) -> (Option<String>, Vec<AnthropicMessage>) {
    let mut system = None;
    let mut messages = Vec::new();
    let mut pending_tool_results: Vec<AnthropicContentBlock> = Vec::new();

    for msg in &req.messages {
        match msg.role {
            crate::types::Role::System => {
                flush_tool_results(&mut messages, &mut pending_tool_results);
                let text = msg.content.as_text();
                system = Some(match system {
                    Some(existing) => format!("{existing}\n\n{text}"),
                    None => text,
                });
            }
            crate::types::Role::Tool => {
                pending_tool_results.push(AnthropicContentBlock::ToolResult {
                    tool_use_id: msg.tool_call_id.clone().unwrap_or_default(),
                    content: msg.content.as_text(),
                });
            }
            crate::types::Role::User => {
                flush_tool_results(&mut messages, &mut pending_tool_results);
                messages.push(AnthropicMessage {
                    role: "user".to_string(),
                    content: to_anthropic_content(&msg.content),
                });
            }
            crate::types::Role::Assistant => {
                flush_tool_results(&mut messages, &mut pending_tool_results);
                match &msg.tool_calls {
                    Some(tool_calls) if !tool_calls.is_empty() => {
                        let mut blocks = Vec::new();
                        let text = msg.content.as_text();
                        if !text.is_empty() {
                            blocks.push(AnthropicContentBlock::Text { text });
                        }
                        for call in tool_calls {
                            let input: serde_json::Value =
                                serde_json::from_str(&call.function.arguments)
                                    .unwrap_or_else(|_| serde_json::json!({}));
                            blocks.push(AnthropicContentBlock::ToolUse {
                                id: call.id.clone(),
                                name: call.function.name.clone(),
                                input,
                            });
                        }
                        messages.push(AnthropicMessage {
                            role: "assistant".to_string(),
                            content: AnthropicMessageContent::Blocks(blocks),
                        });
                    }
                    _ => messages.push(AnthropicMessage {
                        role: "assistant".to_string(),
                        content: to_anthropic_content(&msg.content),
                    }),
                }
            }
        }
    }
    flush_tool_results(&mut messages, &mut pending_tool_results);
    (system, messages)
}

/// Build the Claude request body shared by `chat` and `stream`.
///
/// Anthropic's Messages API supports `stop_sequences`, `temperature`, `top_p`
/// and `max_tokens`. The OpenAI-style `seed`, `n`, `presence_penalty`,
/// `frequency_penalty`, `logprobs`/`top_logprobs` and structured
/// `response_format` have no Claude equivalent, so they are intentionally not
/// forwarded.
fn build_body(req: &ChatRequest, stream: bool) -> Result<AnthropicRequest<'_>> {
    let (system, messages) = split_messages(req);
    let thinking = match &req.thinking {
        Some(cfg) => thinking_to_anthropic(cfg)?,
        None => None,
    };
    Ok(AnthropicRequest {
        model: &req.model,
        messages,
        system,
        temperature: req.temperature,
        max_tokens: req.max_tokens.or(Some(4096)), // Anthropic requires max_tokens
        top_p: req.top_p,
        stop_sequences: req.stop.clone(),
        tools: req.tools.as_deref().map(to_anthropic_tools),
        tool_choice: req.tool_choice.as_ref().map(to_anthropic_tool_choice),
        thinking,
        stream,
    })
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContent>,
    model: String,
    usage: Option<AnthropicUsage>,
    #[serde(default)]
    stop_reason: Option<String>,
}

/// A single content block in a Claude response. Text blocks carry `text`;
/// `tool_use` blocks carry `id` / `name` / `input`. All payload fields are
/// optional so any block type deserializes cleanly and unknown blocks are
/// simply skipped instead of failing the whole response.
#[derive(Deserialize)]
struct AnthropicContent {
    #[serde(rename = "type", default)]
    block_type: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    input: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct AnthropicUsage {
    /// Optional so stream `message_delta` usage (which carries only
    /// `output_tokens` upstream) deserializes cleanly (REA-002 robustness).
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
    /// REA-002: prompt-cache write tokens (Anthropic `cache_creation_input_tokens`).
    #[serde(default)]
    cache_creation_input_tokens: Option<u64>,
    /// REA-002: prompt-cache read tokens (Anthropic `cache_read_input_tokens`).
    #[serde(default)]
    cache_read_input_tokens: Option<u64>,
    /// REA-002: split of `output_tokens` into text vs. thinking tokens.
    #[serde(default)]
    output_tokens_details: Option<AnthropicOutputTokensDetails>,
}

#[derive(Deserialize)]
struct AnthropicOutputTokensDetails {
    /// Thinking tokens inside `output_tokens` (`output_tokens - thinking_tokens`
    /// approximates the non-reasoning output, per the official SDK).
    #[serde(default)]
    thinking_tokens: Option<u64>,
}

/// Translate Anthropic usage into the frozen `Usage` shape (SPCC §6.4).
///
/// - `prompt_tokens` = `input_tokens + cache_creation + cache_read` (the SDK
///   documents this summation as the total input);
/// - missing cache/reasoning fields → `None`, present zero values → `Some(0)`;
/// - `total_tokens` = prompt + completion (Anthropic reports no total field).
fn usage_from_anthropic(
    input_tokens: u64,
    cache_creation: Option<u64>,
    cache_read: Option<u64>,
    output_tokens: u64,
    thinking_tokens: Option<u64>,
) -> Usage {
    let prompt_tokens = input_tokens
        .saturating_add(cache_creation.unwrap_or(0))
        .saturating_add(cache_read.unwrap_or(0));
    let completion_tokens = output_tokens;
    Usage {
        prompt_tokens,
        completion_tokens,
        total_tokens: prompt_tokens.saturating_add(completion_tokens),
        cache_read_tokens: cache_read,
        cache_write_tokens: cache_creation,
        reasoning_tokens: thinking_tokens,
    }
}

#[derive(Deserialize)]
struct AnthropicErrorBody {
    error: AnthropicErrorDetail,
}

#[derive(Deserialize)]
struct AnthropicErrorDetail {
    message: String,
}

// Stream event types
#[derive(Deserialize)]
struct AnthropicStreamEvent {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(default)]
    index: Option<usize>,
    #[serde(default)]
    content_block: Option<AnthropicStreamContentBlock>,
    #[serde(default)]
    delta: Option<AnthropicDelta>,
    /// STR-002A: stream-level error event payload (`{"type":"error","error":{...}}`).
    #[serde(default)]
    error: Option<AnthropicStreamError>,
    /// STR-002A: usage is reported on the `message_delta` event, at the event
    /// level (sibling of `delta`), not inside `delta`. Previously missing → lost.
    #[serde(default)]
    usage: Option<AnthropicUsage>,
    /// REA-002: `message_start` carries the full request-side usage
    /// (`input_tokens` + cache fields), which `message_delta` does not repeat.
    #[serde(default)]
    message: Option<AnthropicStreamMessage>,
}

#[derive(Deserialize)]
struct AnthropicStreamMessage {
    #[serde(default)]
    usage: Option<AnthropicUsage>,
}

/// Payload of an Anthropic `error` stream event. Only the `message` is needed
/// to surface a stream-level error; the inner error `type` is intentionally not
/// modeled (forward-compat: we never branch on it).
#[derive(Deserialize)]
struct AnthropicStreamError {
    message: String,
}

/// The `content_block` of a `content_block_start` event. A `tool_use` block
/// opens a streamed tool call and carries its `id` and `name`.
#[derive(Deserialize)]
struct AnthropicStreamContentBlock {
    #[serde(rename = "type", default)]
    block_type: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize)]
struct AnthropicDelta {
    #[serde(rename = "type", default)]
    delta_type: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    partial_json: Option<String>,
    #[serde(default)]
    stop_reason: Option<String>,
    /// M2-16：thinking 增量文本（Anthropic `thinking_delta`）。
    #[serde(default)]
    thinking: Option<String>,
}

/// Accumulates streamed `tool_use` blocks from an Anthropic stream, keyed by
/// their stream `index`. A `content_block_start` opens a call (id + name), the
/// JSON arguments arrive as `input_json_delta` fragments on
/// `content_block_delta`, and the call set is drained on `message_delta`.
#[derive(Default)]
struct AnthropicToolAccumulator {
    builders: BTreeMap<usize, AnthropicToolBuilder>,
    /// REA-002: usage captured from `message_start` (full input + cache side).
    start_usage: Option<AnthropicUsage>,
    /// REA-002: `thinking_done` is emitted at most once (signature_delta or
    /// redacted_thinking), SPCC §6.3 "thinking 结束最多标记一次".
    thinking_done_emitted: bool,
}

#[derive(Default)]
struct AnthropicToolBuilder {
    id: String,
    name: String,
    arguments: String,
}

impl AnthropicToolAccumulator {
    /// Capture the full request-side usage from `message_start`.
    fn capture_start_usage(&mut self, usage: Option<AnthropicUsage>) {
        if usage.is_some() && self.start_usage.is_none() {
            self.start_usage = usage;
        }
    }

    /// Mark the end of the thinking block. Emits at most one
    /// `thinking_done = true` chunk across the whole stream.
    fn thinking_done(&mut self) -> Vec<Result<StreamChunk>> {
        if self.thinking_done_emitted {
            return Vec::new();
        }
        self.thinking_done_emitted = true;
        vec![Ok(StreamChunk {
            thinking_done: Some(true),
            ..Default::default()
        })]
    }

    /// Build the terminal usage: `message_start` side (input + cache) merged
    /// with the `message_delta` side (output + thinking tokens).
    fn terminal_usage(&self, delta_usage: Option<AnthropicUsage>) -> Option<Usage> {
        let start = self.start_usage.as_ref();
        let (input, cache_creation, cache_read) = match start {
            Some(u) => (
                u.input_tokens.unwrap_or(0),
                u.cache_creation_input_tokens,
                u.cache_read_input_tokens,
            ),
            None => {
                let u = delta_usage.as_ref()?;
                (
                    u.input_tokens.unwrap_or(0),
                    u.cache_creation_input_tokens,
                    u.cache_read_input_tokens,
                )
            }
        };
        let (output, thinking) = match &delta_usage {
            Some(u) => (
                u.output_tokens.unwrap_or(0),
                u.output_tokens_details
                    .as_ref()
                    .and_then(|d| d.thinking_tokens),
            ),
            None => (start.and_then(|u| u.output_tokens).unwrap_or(0), None),
        };
        Some(usage_from_anthropic(
            input,
            cache_creation,
            cache_read,
            output,
            thinking,
        ))
    }

    fn start(&mut self, index: usize, id: String, name: String) {
        self.builders.insert(
            index,
            AnthropicToolBuilder {
                id,
                name,
                arguments: String::new(),
            },
        );
    }

    fn push_arguments(&mut self, index: usize, partial_json: &str) {
        if let Some(builder) = self.builders.get_mut(&index) {
            builder.arguments.push_str(partial_json);
        }
    }

    fn take(&mut self) -> Option<Vec<ToolCall>> {
        if self.builders.is_empty() {
            return None;
        }
        let calls: Vec<ToolCall> = std::mem::take(&mut self.builders)
            .into_values()
            .filter(|b| !b.name.is_empty())
            .map(|b| {
                let arguments = if b.arguments.is_empty() {
                    "{}".to_string()
                } else {
                    b.arguments
                };
                ToolCall {
                    id: b.id,
                    call_type: "function".to_string(),
                    function: FunctionCall {
                        name: b.name,
                        arguments,
                    },
                }
            })
            .collect();
        if calls.is_empty() {
            None
        } else {
            Some(calls)
        }
    }
}

/// Parse a single SSE line from an Anthropic stream into zero or more
/// [`StreamChunk`]s, threading an [`AnthropicToolAccumulator`] so streamed
/// `tool_use` blocks can be reconstructed across events. Lines are guaranteed
/// complete by [`line_stream`].
///
/// Anthropic delivers the stop reason on the `message_delta` event rather than
/// `message_stop`, so completion is keyed off that event's `stop_reason`. Text
/// is surfaced as deltas on `content_block_delta`; `tool_use` blocks are
/// reconstructed from `content_block_start` + `input_json_delta` fragments and
/// surfaced as `tool_calls` on the terminal chunk.
fn parse_sse_line(tools: &mut AnthropicToolAccumulator, line: &str) -> Vec<Result<StreamChunk>> {
    let line = line.trim();
    let Some(data) = line.strip_prefix("data: ") else {
        // Framing lines (empty, `event:`, `:comment`, etc.) carry no data.
        return Vec::new();
    };
    let event = match serde_json::from_str::<AnthropicStreamEvent>(data) {
        Ok(event) => event,
        Err(e) => {
            // STR-002A: malformed / truncated JSON must surface a Parse error
            // (and, via the STR-001 shared layer, close the stream) instead of
            // being silently dropped — the previous behavior hid real failures.
            return vec![Err(LlmError::Parse(format!(
                "anthropic stream: malformed SSE data: {e}"
            )))];
        }
    };
    match event.event_type.as_str() {
        "message_start" => {
            // REA-002: capture the full request-side usage (input + cache);
            // `message_delta` only repeats output-side usage.
            let usage = event.message.and_then(|m| m.usage);
            tools.capture_start_usage(usage);
            Vec::new()
        }
        "content_block_start" => {
            if let Some(block) = event.content_block {
                match block.block_type.as_deref() {
                    Some("tool_use") => {
                        tools.start(
                            event.index.unwrap_or(0),
                            block.id.unwrap_or_default(),
                            block.name.unwrap_or_default(),
                        );
                    }
                    // REA-002: a redacted thinking block carries no deltas —
                    // it is its own end marker for the thinking phase.
                    Some("redacted_thinking") => return tools.thinking_done(),
                    _ => {}
                }
            }
            Vec::new()
        }
        "content_block_delta" => {
            let index = event.index.unwrap_or(0);
            let Some(delta) = event.delta else {
                return Vec::new();
            };
            match delta.delta_type.as_deref() {
                Some("input_json_delta") => {
                    if let Some(partial) = delta.partial_json {
                        tools.push_arguments(index, &partial);
                    }
                    Vec::new()
                }
                Some("thinking_delta") => {
                    // M2-16：思考增量，逐块产出 thinking 字段。
                    let text = delta.thinking.clone().unwrap_or_default();
                    if text.is_empty() {
                        Vec::new()
                    } else {
                        vec![Ok(StreamChunk {
                            thinking: Some(text),
                            ..Default::default()
                        })]
                    }
                }
                // REA-002: `signature_delta` is the official end marker of the
                // thinking block (SPCC §6.3: 结束最多标记一次).
                Some("signature_delta") => tools.thinking_done(),
                _ => {
                    let text = delta.text.unwrap_or_default();
                    if text.is_empty() {
                        Vec::new()
                    } else {
                        vec![Ok(StreamChunk {
                            delta: text,
                            ..Default::default()
                        })]
                    }
                }
            }
        }
        "message_delta" => {
            let finish_reason = event
                .delta
                .and_then(|d| d.stop_reason)
                .map(normalize_stop_reason);
            // STR-002A + REA-002: translate usage — `message_start` side
            // (input + cache) merged with `message_delta` side (output +
            // thinking tokens); falls back to `message_delta` alone.
            let usage = tools.terminal_usage(event.usage);
            let tool_calls = tools.take();
            vec![Ok(StreamChunk {
                done: finish_reason.is_some(),
                finish_reason,
                tool_calls,
                usage,
                ..Default::default()
            })]
        }
        "error" => {
            // STR-002A: a stream-level error event carries no HTTP status, so
            // surface it as LlmError::Stream. The STR-001 shared layer then
            // guarantees the stream is closed without a success terminal.
            let msg = event
                .error
                .map(|e| e.message)
                .unwrap_or_else(|| "anthropic stream error event".to_string());
            vec![Err(LlmError::Stream(msg))]
        }
        other => {
            // Forward-compat: unknown / future event types (including
            // `message_stop`, `ping`, `comment`) are ignored. Log only the type,
            // never the content (logging discipline: no payload, no secrets).
            tracing::debug!(
                event_type = other,
                "ignoring unknown anthropic stream event"
            );
            Vec::new()
        }
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse> {
        crate::providers::warn_if_unsupported_n("anthropic", req.n);
        // REA-002 (SPCC §6.3): `ChatResponse` cannot carry reasoning, so a
        // non-stream chat with thinking enabled fails BEFORE any network call.
        // `build_body` also rejects `Enabled{budget_tokens: None}` here.
        if thinking_enabled(&req.thinking) {
            return Err(LlmError::Unsupported {
                feature: "reasoning".to_string(),
                message: "Anthropic reasoning is only supported on the streaming path in 0.1.3 \
                          (ChatResponse cannot carry reasoning)"
                    .to_string(),
            });
        }
        let body = build_body(req, false)?;

        tracing::debug!(
            provider = "anthropic",
            model = &req.model,
            "sending chat request"
        );
        let resp = self
            .client
            .post(format!("{}/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            let msg = serde_json::from_str::<AnthropicErrorBody>(&text)
                .map(|e| e.error.message)
                .unwrap_or(text);
            let err = LlmError::Api {
                status: status.as_u16(),
                message: msg,
            };
            tracing::error!(
                provider = "anthropic",
                status = status.as_u16(),
                error_kind = "api_error",
                "API error"
            );
            return Err(err);
        }

        let parsed: AnthropicResponse = resp
            .json()
            .await
            .map_err(|e| LlmError::Parse(e.to_string()))?;

        let content = parsed
            .content
            .iter()
            .filter_map(|c| c.text.as_deref())
            .collect::<String>();

        let tool_calls: Vec<ToolCall> = parsed
            .content
            .iter()
            .filter(|c| c.block_type.as_deref() == Some("tool_use"))
            .filter_map(|c| match (&c.id, &c.name) {
                (Some(id), Some(name)) => {
                    let arguments = c
                        .input
                        .as_ref()
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "{}".to_string());
                    Some(ToolCall {
                        id: id.clone(),
                        call_type: "function".to_string(),
                        function: FunctionCall {
                            name: name.clone(),
                            arguments,
                        },
                    })
                }
                _ => None,
            })
            .collect();
        let tool_calls = if tool_calls.is_empty() {
            None
        } else {
            Some(tool_calls)
        };

        let result = ChatResponse {
            content,
            model: parsed.model,
            usage: parsed.usage.map(|u| {
                usage_from_anthropic(
                    u.input_tokens.unwrap_or(0),
                    u.cache_creation_input_tokens,
                    u.cache_read_input_tokens,
                    u.output_tokens.unwrap_or(0),
                    u.output_tokens_details.and_then(|d| d.thinking_tokens),
                )
            }),
            tool_calls,
            finish_reason: parsed.stop_reason.map(normalize_stop_reason),
            logprobs: None,
        };
        tracing::debug!(provider = "anthropic", model = &result.model, finish_reason = ?result.finish_reason, "chat response received");
        Ok(result)
    }

    async fn stream(&self, req: &ChatRequest) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        crate::providers::warn_if_unsupported_n("anthropic", req.n);
        // REA-002: `Enabled{budget_tokens: None}` fails here, before network.
        let body = build_body(req, true)?;

        tracing::debug!(
            provider = "anthropic",
            model = &req.model,
            "sending stream request"
        );
        let resp = self
            .client
            .post(format!("{}/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            let msg = serde_json::from_str::<AnthropicErrorBody>(&text)
                .map(|e| e.error.message)
                .unwrap_or(text);
            let err = LlmError::Api {
                status: status.as_u16(),
                message: msg,
            };
            tracing::error!(
                provider = "anthropic",
                status = status.as_u16(),
                error_kind = "api_error",
                "API error"
            );
            return Err(err);
        }

        let byte_stream = resp
            .bytes_stream()
            .map(|r| r.map_err(|e| LlmError::Stream(e.to_string())));

        let stream = line_stream(byte_stream)
            .scan(AnthropicToolAccumulator::default(), |tools, line_result| {
                let chunks = match line_result {
                    Ok(line) => parse_sse_line(tools, &line),
                    Err(e) => vec![Err(e)],
                };
                futures::future::ready(Some(futures::stream::iter(chunks)))
            })
            .flatten();

        Ok(stream.boxed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Message;

    #[test]
    fn text_message_serializes_as_plain_string() {
        let content = to_anthropic_content(&Content::Text("hello".to_string()));
        let v = serde_json::to_value(&content).unwrap();
        assert_eq!(v, serde_json::json!("hello"));
    }

    #[test]
    fn data_url_image_becomes_base64_block() {
        let source = anthropic_image_source("data:image/jpeg;base64,QUJD");
        let v = serde_json::to_value(&source).unwrap();
        assert_eq!(v["type"], "base64");
        assert_eq!(v["media_type"], "image/jpeg");
        assert_eq!(v["data"], "QUJD");
    }

    #[test]
    fn http_url_image_becomes_url_block() {
        let source = anthropic_image_source("https://example.com/cat.png");
        let v = serde_json::to_value(&source).unwrap();
        assert_eq!(v["type"], "url");
        assert_eq!(v["url"], "https://example.com/cat.png");
    }

    #[test]
    fn split_messages_extracts_system_text() {
        let req = ChatRequest::from_messages(
            "claude",
            vec![Message::system("be brief"), Message::user("hi")],
        );
        let (system, messages) = split_messages(&req);
        assert_eq!(system.as_deref(), Some("be brief"));
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role.as_str(), "user");
    }

    #[test]
    fn split_messages_concatenates_multiple_system_messages() {
        let req = ChatRequest::from_messages(
            "claude",
            vec![
                Message::system("be brief"),
                Message::user("hi"),
                Message::system("also be polite"),
                Message::user("thanks"),
            ],
        );
        let (system, messages) = split_messages(&req);
        assert_eq!(system.as_deref(), Some("be brief\n\nalso be polite"));
        assert_eq!(messages.len(), 2);
    }

    #[test]
    fn image_message_serializes_as_content_blocks() {
        let content = to_anthropic_content(&Content::Parts(vec![
            ContentPart::text("describe"),
            ContentPart::image_url("https://example.com/cat.png"),
        ]));
        let v = serde_json::to_value(&content).unwrap();
        assert_eq!(v[0]["type"], "text");
        assert_eq!(v[0]["text"], "describe");
        assert_eq!(v[1]["type"], "image");
        assert_eq!(v[1]["source"]["type"], "url");
        assert_eq!(v[1]["source"]["url"], "https://example.com/cat.png");
    }

    #[test]
    fn response_with_non_text_blocks_is_tolerated() {
        let raw = serde_json::json!({
            "content": [
                { "type": "text", "text": "Hello " },
                { "type": "tool_use", "id": "t1", "name": "f", "input": {} },
                { "type": "text", "text": "world" }
            ],
            "model": "claude-3-5-sonnet",
            "usage": { "input_tokens": 1, "output_tokens": 2 }
        })
        .to_string();
        let parsed: AnthropicResponse = serde_json::from_str(&raw).unwrap();
        let content = parsed
            .content
            .iter()
            .filter_map(|c| c.text.as_deref())
            .collect::<String>();
        assert_eq!(content, "Hello world");
    }

    #[test]
    fn tools_serialize_with_input_schema() {
        let tools = to_anthropic_tools(&[Tool::function(
            "get_weather",
            Some("Get weather".to_string()),
            serde_json::json!({"type": "object"}),
        )]);
        let v = serde_json::to_value(&tools).unwrap();
        assert_eq!(v[0]["name"], "get_weather");
        assert_eq!(v[0]["description"], "Get weather");
        assert_eq!(v[0]["input_schema"]["type"], "object");
    }

    #[test]
    fn tool_choice_maps_to_anthropic() {
        assert_eq!(
            serde_json::to_value(to_anthropic_tool_choice(&ToolChoice::auto())).unwrap(),
            serde_json::json!({"type": "auto"})
        );
        assert_eq!(
            serde_json::to_value(to_anthropic_tool_choice(&ToolChoice::required())).unwrap(),
            serde_json::json!({"type": "any"})
        );
        let forced =
            serde_json::to_value(to_anthropic_tool_choice(&ToolChoice::function("f"))).unwrap();
        assert_eq!(forced["type"], "tool");
        assert_eq!(forced["name"], "f");
    }

    #[test]
    fn stop_sequences_are_forwarded() {
        let req = ChatRequest::new("claude", "hi").with_stop(vec!["END".to_string()]);
        let body = build_body(&req, false).unwrap();
        let v = serde_json::to_value(&body).unwrap();
        assert_eq!(v["stop_sequences"][0], "END");
    }

    #[test]
    fn omitted_stop_is_not_serialized() {
        let req = ChatRequest::new("claude", "hi");
        let body = build_body(&req, false).unwrap();
        let v = serde_json::to_value(&body).unwrap();
        assert!(v.get("stop_sequences").is_none());
    }

    #[test]
    fn response_tool_use_block_parsed_into_tool_calls() {
        let raw = serde_json::json!({
            "content": [
                { "type": "text", "text": "let me check" },
                { "type": "tool_use", "id": "toolu_1", "name": "get_weather", "input": {"city": "SF"} }
            ],
            "model": "claude-3-5-sonnet",
            "stop_reason": "tool_use",
            "usage": { "input_tokens": 1, "output_tokens": 2 }
        })
        .to_string();
        let parsed: AnthropicResponse = serde_json::from_str(&raw).unwrap();
        let calls: Vec<ToolCall> = parsed
            .content
            .iter()
            .filter(|c| c.block_type.as_deref() == Some("tool_use"))
            .filter_map(|c| match (&c.id, &c.name) {
                (Some(id), Some(name)) => Some(ToolCall {
                    id: id.clone(),
                    call_type: "function".to_string(),
                    function: FunctionCall {
                        name: name.clone(),
                        arguments: c.input.as_ref().unwrap().to_string(),
                    },
                }),
                _ => None,
            })
            .collect();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "toolu_1");
        assert_eq!(calls[0].function.name, "get_weather");
        assert_eq!(
            normalize_stop_reason(parsed.stop_reason.unwrap()),
            FinishReason::ToolCalls
        );
    }

    #[test]
    fn assistant_tool_calls_and_results_serialize_as_blocks() {
        let req = ChatRequest::from_messages(
            "claude",
            vec![
                Message::user("weather?"),
                Message::assistant_tool_calls(vec![ToolCall {
                    id: "toolu_1".to_string(),
                    call_type: "function".to_string(),
                    function: FunctionCall {
                        name: "get_weather".to_string(),
                        arguments: "{\"city\":\"SF\"}".to_string(),
                    },
                }]),
                Message::tool("toolu_1", "sunny"),
            ],
        );
        let (_system, messages) = split_messages(&req);
        assert_eq!(messages.len(), 3);
        let v = serde_json::to_value(&messages).unwrap();
        assert_eq!(v[1]["role"], "assistant");
        assert_eq!(v[1]["content"][0]["type"], "tool_use");
        assert_eq!(v[1]["content"][0]["id"], "toolu_1");
        assert_eq!(v[1]["content"][0]["input"]["city"], "SF");
        assert_eq!(v[2]["role"], "user");
        assert_eq!(v[2]["content"][0]["type"], "tool_result");
        assert_eq!(v[2]["content"][0]["tool_use_id"], "toolu_1");
        assert_eq!(v[2]["content"][0]["content"], "sunny");
    }

    #[test]
    fn stream_reconstructs_tool_calls() {
        let mut tools = AnthropicToolAccumulator::default();
        let lines = [
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"get_weather"}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"city\":"}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"\"SF\"}"}}"#,
            r#"data: {"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
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
        assert_eq!(calls[0].id, "toolu_1");
        assert_eq!(calls[0].function.name, "get_weather");
        assert_eq!(calls[0].function.arguments, "{\"city\":\"SF\"}");
    }

    #[test]
    fn stream_text_deltas_have_no_tool_calls() {
        let mut tools = AnthropicToolAccumulator::default();
        let mut chunks = Vec::new();
        for line in [
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#,
            r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#,
        ] {
            for chunk in parse_sse_line(&mut tools, line) {
                chunks.push(chunk.unwrap());
            }
        }
        assert_eq!(chunks[0].delta, "Hello");
        let last = chunks.last().unwrap();
        assert!(last.done);
        assert_eq!(last.finish_reason, Some(FinishReason::EndTurn));
        assert!(last.tool_calls.is_none());
    }

    #[test]
    fn stream_thinking_delta_produces_thinking_chunk() {
        let mut tools = AnthropicToolAccumulator::default();
        let mut chunks = Vec::new();
        for line in [
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Let me think"}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}"#,
            r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#,
        ] {
            for chunk in parse_sse_line(&mut tools, line) {
                chunks.push(chunk.unwrap());
            }
        }
        assert_eq!(chunks[0].thinking.as_deref(), Some("Let me think"));
        assert_eq!(chunks[1].delta, "Hi");
        let last = chunks.last().unwrap();
        assert!(last.done);
        assert_eq!(last.finish_reason, Some(FinishReason::EndTurn));
    }

    // --- STR-002A: failure-first fixtures for surfaced errors / usage ---
    // These must be RED against the old implementation (errors silently
    // dropped, usage untranslated) and turn GREEN once parse_sse_line
    // surfaces parse/stream errors and translates message_delta.usage.

    #[test]
    fn stream_malformed_json_yields_parse_error() {
        let mut tools = AnthropicToolAccumulator::default();
        let chunks = parse_sse_line(&mut tools, "data: {not valid json");
        assert_eq!(
            chunks.len(),
            1,
            "malformed JSON must surface one Parse error"
        );
        assert!(matches!(chunks[0], Err(LlmError::Parse(_))));
    }

    #[test]
    fn stream_truncated_json_yields_parse_error() {
        let mut tools = AnthropicToolAccumulator::default();
        let chunks = parse_sse_line(
            &mut tools,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello""#,
        );
        assert_eq!(
            chunks.len(),
            1,
            "truncated JSON must surface one Parse error"
        );
        assert!(matches!(chunks[0], Err(LlmError::Parse(_))));
    }

    #[test]
    fn stream_error_event_yields_stream_error() {
        let mut tools = AnthropicToolAccumulator::default();
        let chunks = parse_sse_line(
            &mut tools,
            r#"data: {"type":"error","error":{"type":"overloaded_error","message":"upstream overloaded"}}"#,
        );
        assert_eq!(
            chunks.len(),
            1,
            "Anthropic error event must surface one Stream error"
        );
        match &chunks[0] {
            Err(LlmError::Stream(msg)) => assert!(msg.contains("upstream overloaded")),
            other => panic!("expected LlmError::Stream, got {other:?}"),
        }
    }

    #[test]
    fn stream_message_delta_translates_usage() {
        let mut tools = AnthropicToolAccumulator::default();
        let chunks = parse_sse_line(
            &mut tools,
            r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"input_tokens":10,"output_tokens":20}}"#,
        );
        let chunk = chunks
            .into_iter()
            .next()
            .expect("one chunk")
            .expect("ok chunk");
        assert!(chunk.done);
        assert_eq!(chunk.finish_reason, Some(FinishReason::EndTurn));
        let u = chunk.usage.expect("message_delta.usage must be translated");
        assert_eq!(u.prompt_tokens, 10);
        assert_eq!(u.completion_tokens, 20);
        assert_eq!(u.total_tokens, 30);
    }

    #[test]
    fn stream_message_stop_and_unknown_events_are_ignored() {
        let mut tools = AnthropicToolAccumulator::default();
        // message_stop: terminal signal is already carried by message_delta.done.
        let stop = parse_sse_line(&mut tools, r#"data: {"type":"message_stop"}"#);
        assert!(stop.is_empty(), "message_stop must be ignored");
        // Unknown / future event types must be ignored (forward-compat), never error.
        let future = parse_sse_line(&mut tools, r#"data: {"type":"some_future_event","foo":1}"#);
        assert!(future.is_empty(), "unknown event type must be ignored");
    }

    // --- REA-002: extended thinking request mapping ---

    #[test]
    fn thinking_enabled_serializes_with_budget() {
        let req = ChatRequest::new("claude", "hi").with_thinking(ThinkingConfig::Enabled {
            budget_tokens: Some(2048),
        });
        let body = build_body(&req, true).unwrap();
        let v = serde_json::to_value(&body).unwrap();
        assert_eq!(v["thinking"]["type"], "enabled");
        assert_eq!(v["thinking"]["budget_tokens"], 2048);
    }

    #[test]
    fn thinking_disabled_omits_field() {
        let req = ChatRequest::new("claude", "hi").with_thinking(ThinkingConfig::Disabled);
        let body = build_body(&req, true).unwrap();
        let v = serde_json::to_value(&body).unwrap();
        assert!(
            v.get("thinking").is_none(),
            "disabled thinking must be omitted"
        );
        // Default request also omits the field.
        let plain = ChatRequest::new("claude", "hi");
        let body = build_body(&plain, true).unwrap();
        let v = serde_json::to_value(&body).unwrap();
        assert!(v.get("thinking").is_none());
    }

    #[test]
    fn thinking_enabled_without_budget_errors() {
        let req = ChatRequest::new("claude", "hi").with_thinking(ThinkingConfig::Enabled {
            budget_tokens: None,
        });
        match build_body(&req, true) {
            Err(LlmError::Unsupported { .. }) => {}
            Err(other) => panic!(
                "Anthropic thinking without budget_tokens must be Unsupported, got {other:?}"
            ),
            Ok(_) => panic!("expected Unsupported, got a request body"),
        }
    }

    #[tokio::test]
    async fn chat_with_thinking_is_unsupported_before_network() {
        let provider = AnthropicProvider::new(ProviderConfig::new("sk-ant-test"));
        let req = ChatRequest::new("claude", "hi").with_thinking(ThinkingConfig::Enabled {
            budget_tokens: Some(1024),
        });
        let err = provider.chat(&req).await.unwrap_err();
        assert!(
            matches!(err, LlmError::Unsupported { .. }),
            "non-stream chat with reasoning must fail with Unsupported, got {err:?}"
        );
    }

    // --- REA-002: streaming thinking lifecycle ---

    #[test]
    fn stream_signature_delta_marks_thinking_done() {
        let mut tools = AnthropicToolAccumulator::default();
        let mut chunks = Vec::new();
        for line in [
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"think"}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig"}}"#,
            r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#,
        ] {
            for chunk in parse_sse_line(&mut tools, line) {
                chunks.push(chunk.unwrap());
            }
        }
        assert_eq!(chunks[0].thinking.as_deref(), Some("think"));
        assert_eq!(chunks[1].thinking_done, Some(true));
    }

    #[test]
    fn stream_redacted_thinking_marks_thinking_done() {
        let mut tools = AnthropicToolAccumulator::default();
        let chunks = parse_sse_line(
            &mut tools,
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"redacted_thinking","data":"xxx"}}"#,
        );
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].as_ref().unwrap().thinking_done, Some(true));
    }

    #[test]
    fn thinking_done_emitted_only_once() {
        let mut tools = AnthropicToolAccumulator::default();
        let first = parse_sse_line(
            &mut tools,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"a"}}"#,
        );
        let second = parse_sse_line(
            &mut tools,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"b"}}"#,
        );
        assert_eq!(first.len(), 1);
        assert!(
            second.is_empty(),
            "thinking_done must be emitted at most once"
        );
    }

    // --- REA-002: usage translation (cache + thinking tokens) ---

    #[test]
    fn message_start_usage_merges_cache_and_thinking() {
        let mut tools = AnthropicToolAccumulator::default();
        let mut chunks = Vec::new();
        for line in [
            r#"data: {"type":"message_start","message":{"usage":{"input_tokens":100,"cache_creation_input_tokens":10,"cache_read_input_tokens":5,"output_tokens":0}}}"#,
            r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"input_tokens":100,"output_tokens":20,"output_tokens_details":{"thinking_tokens":8}}}"#,
        ] {
            for chunk in parse_sse_line(&mut tools, line) {
                chunks.push(chunk.unwrap());
            }
        }
        let terminal = chunks.last().expect("terminal chunk");
        let u = terminal.usage.as_ref().expect("usage present");
        assert_eq!(u.prompt_tokens, 115); // 100 input + 10 cache write + 5 cache read
        assert_eq!(u.completion_tokens, 20);
        assert_eq!(u.total_tokens, 135);
        assert_eq!(u.cache_write_tokens, Some(10));
        assert_eq!(u.cache_read_tokens, Some(5));
        assert_eq!(u.reasoning_tokens, Some(8));
    }

    #[test]
    fn usage_missing_fields_stay_none() {
        let mut tools = AnthropicToolAccumulator::default();
        let mut chunks = Vec::new();
        for line in [
            r#"data: {"type":"message_start","message":{"usage":{"input_tokens":10,"output_tokens":0}}}"#,
            r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":5}}"#,
        ] {
            for chunk in parse_sse_line(&mut tools, line) {
                chunks.push(chunk.unwrap());
            }
        }
        let u = chunks.last().unwrap().usage.as_ref().unwrap();
        assert_eq!(u.prompt_tokens, 10);
        assert_eq!(u.completion_tokens, 5);
        assert_eq!(u.total_tokens, 15);
        assert_eq!(u.cache_read_tokens, None);
        assert_eq!(u.cache_write_tokens, None);
        assert_eq!(u.reasoning_tokens, None);
    }

    #[test]
    fn chat_usage_maps_cache_and_thinking() {
        let raw = serde_json::json!({
            "content": [{ "type": "text", "text": "ok" }],
            "model": "claude",
            "stop_reason": "end_turn",
            "usage": {
                "input_tokens": 100,
                "cache_creation_input_tokens": 10,
                "cache_read_input_tokens": 5,
                "output_tokens": 20,
                "output_tokens_details": { "thinking_tokens": 8 }
            }
        })
        .to_string();
        let parsed: AnthropicResponse = serde_json::from_str(&raw).unwrap();
        let u = parsed
            .usage
            .map(|u| {
                usage_from_anthropic(
                    u.input_tokens.unwrap_or(0),
                    u.cache_creation_input_tokens,
                    u.cache_read_input_tokens,
                    u.output_tokens.unwrap_or(0),
                    u.output_tokens_details.and_then(|d| d.thinking_tokens),
                )
            })
            .unwrap();
        assert_eq!(u.prompt_tokens, 115);
        assert_eq!(u.total_tokens, 135);
        assert_eq!(u.cache_write_tokens, Some(10));
        assert_eq!(u.reasoning_tokens, Some(8));
    }
}
