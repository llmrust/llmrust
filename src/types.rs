//! Core types for llmrust — unified LLM API interface.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Chat message role.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    /// A tool/function result fed back into the conversation.
    Tool,
}

/// An OpenAI-style tool the model may call. Currently only `function` tools
/// are supported.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Tool {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: FunctionDef,
}

/// The schema of a callable function advertised to the model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FunctionDef {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// JSON Schema describing the function's parameters.
    pub parameters: serde_json::Value,
}

impl Tool {
    /// Build a `function` tool from a name, optional description, and a JSON
    /// Schema for its parameters.
    pub fn function(
        name: impl Into<String>,
        description: Option<String>,
        parameters: serde_json::Value,
    ) -> Self {
        Self {
            tool_type: "function".to_string(),
            function: FunctionDef {
                name: name.into(),
                description,
                parameters,
            },
        }
    }
}

/// Controls whether (and which) tool the model may call.
///
/// Serializes as either a bare string (`"auto"`, `"none"`, `"required"`) or an
/// object selecting a specific function, matching the OpenAI wire format.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ToolChoice {
    /// One of `"auto"`, `"none"`, or `"required"`.
    Mode(String),
    /// Force a specific function call.
    Function {
        #[serde(rename = "type")]
        choice_type: String,
        function: ToolChoiceFunction,
    },
}

/// The function selected by a [`ToolChoice::Function`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolChoiceFunction {
    pub name: String,
}

impl ToolChoice {
    /// Let the model decide whether to call a tool (`"auto"`).
    pub fn auto() -> Self {
        Self::Mode("auto".to_string())
    }

    /// Forbid tool calls (`"none"`).
    pub fn none() -> Self {
        Self::Mode("none".to_string())
    }

    /// Require the model to call at least one tool (`"required"`).
    pub fn required() -> Self {
        Self::Mode("required".to_string())
    }

    /// Force the model to call a specific function by name.
    pub fn function(name: impl Into<String>) -> Self {
        Self::Function {
            choice_type: "function".to_string(),
            function: ToolChoiceFunction { name: name.into() },
        }
    }
}

/// A tool call requested by the model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: FunctionCall,
}

/// The function invocation inside a [`ToolCall`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FunctionCall {
    pub name: String,
    /// Raw JSON string of arguments, exactly as emitted by the model.
    pub arguments: String,
}

/// A single part of a multimodal message: either a text span or an image.
///
/// Serializes to OpenAI's "content parts" wire format, e.g.
/// `{"type":"text","text":"..."}` or
/// `{"type":"image_url","image_url":{"url":"..."}}`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    /// A span of text.
    Text { text: String },
    /// An image referenced by URL or `data:` URI.
    ImageUrl { image_url: ImageUrl },
}

impl ContentPart {
    /// Build a text content part.
    pub fn text(text: impl Into<String>) -> Self {
        ContentPart::Text { text: text.into() }
    }

    /// Build an image content part from a URL or `data:` URI.
    pub fn image_url(url: impl Into<String>) -> Self {
        ContentPart::ImageUrl {
            image_url: ImageUrl {
                url: url.into(),
                detail: None,
            },
        }
    }
}

/// An image reference inside a [`ContentPart::ImageUrl`].
///
/// `url` may be a public `https://` URL or an inline `data:<mime>;base64,<...>`
/// URI. `detail` is an optional OpenAI hint (`"low"`, `"high"`, `"auto"`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageUrl {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Message content: either a plain string or a list of multimodal parts
/// (text + images), matching OpenAI's `content` field.
///
/// A bare string serializes as a JSON string and a part list as a JSON array,
/// so the wire format stays byte-compatible with OpenAI in both directions.
/// All the string-based constructors on [`Message`] keep working unchanged via
/// the `From<String>` / `From<&str>` conversions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Content {
    /// Plain text content.
    Text(String),
    /// Multimodal content parts (text and/or images).
    Parts(Vec<ContentPart>),
}

impl Default for Content {
    fn default() -> Self {
        Content::Text(String::new())
    }
}

impl Content {
    /// True if this is empty text or an empty part list.
    pub fn is_empty(&self) -> bool {
        match self {
            Content::Text(s) => s.is_empty(),
            Content::Parts(parts) => parts.is_empty(),
        }
    }

    /// Concatenate all text, ignoring image parts. Used by text-only providers
    /// and to extract system-prompt text.
    pub fn as_text(&self) -> String {
        match self {
            Content::Text(s) => s.clone(),
            Content::Parts(parts) => {
                let mut out = String::new();
                for part in parts {
                    if let ContentPart::Text { text } = part {
                        out.push_str(text);
                    }
                }
                out
            }
        }
    }

    /// The image references in this content, in order.
    pub fn images(&self) -> Vec<&ImageUrl> {
        let mut out = Vec::new();
        if let Content::Parts(parts) = self {
            for part in parts {
                if let ContentPart::ImageUrl { image_url } = part {
                    out.push(image_url);
                }
            }
        }
        out
    }
}

impl From<String> for Content {
    fn from(s: String) -> Self {
        Content::Text(s)
    }
}

impl From<&str> for Content {
    fn from(s: &str) -> Self {
        Content::Text(s.to_string())
    }
}

impl From<Vec<ContentPart>> for Content {
    fn from(parts: Vec<ContentPart>) -> Self {
        Content::Parts(parts)
    }
}

/// A single chat message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    pub role: Role,
    pub content: Content,
    /// Tool calls requested by the assistant (present on assistant turns that
    /// invoke tools).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// The id of the tool call this message responds to (present on `tool`
    /// messages).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Optional participant name (e.g. the tool/function name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: Content::Text(content.into()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: Content::Text(content.into()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: Content::Text(content.into()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    /// Build a user message from multimodal content parts (text and/or images).
    pub fn user_with_parts(parts: Vec<ContentPart>) -> Self {
        Self {
            role: Role::User,
            content: Content::Parts(parts),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    /// Convenience: a user message pairing a text prompt with a single image
    /// (an `https://` URL or a `data:` URI).
    pub fn user_with_image(text: impl Into<String>, image_url: impl Into<String>) -> Self {
        Self::user_with_parts(vec![
            ContentPart::text(text),
            ContentPart::image_url(image_url),
        ])
    }

    /// Build a `tool` message carrying the result of a tool call, keyed by the
    /// originating [`ToolCall::id`].
    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: Content::Text(content.into()),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
            name: None,
        }
    }

    /// Build an assistant message that requests one or more tool calls.
    pub fn assistant_tool_calls(tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: Role::Assistant,
            content: Content::Text(String::new()),
            tool_calls: Some(tool_calls),
            tool_call_id: None,
            name: None,
        }
    }
}

/// Token usage statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    /// 缓存读取 token（提示缓存命中，Anthropic/OpenAI）。M2-16：additive。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u64>,
    /// 缓存写入 token（写入提示缓存）。M2-16：additive。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
    /// 思考/reasoning 专用 token 数。M2-16：additive。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
}

/// Log-probability information for the generated tokens, returned when
/// `logprobs` / `top_logprobs` were requested and the provider reports them.
///
/// Mirrors OpenAI's `choices[].logprobs` object; other providers (e.g. Gemini)
/// are normalized into this same shape.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LogProbs {
    /// Per-token log probabilities for the generated message content, in order.
    #[serde(default)]
    pub content: Vec<TokenLogProb>,
}

/// Log-probability data for a single generated token, optionally including the
/// most-likely alternative tokens at that position.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TokenLogProb {
    /// The token text.
    pub token: String,
    /// The natural-log probability of this token.
    pub logprob: f64,
    /// The token's raw UTF-8 bytes, when reported by the provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<Vec<u8>>,
    /// The most-likely alternative tokens at this position, when `top_logprobs`
    /// was requested.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub top_logprobs: Vec<TopLogProb>,
}

/// A single alternative token and its log probability at one position.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TopLogProb {
    /// The token text.
    pub token: String,
    /// The natural-log probability of this token.
    pub logprob: f64,
    /// The token's raw UTF-8 bytes, when reported by the provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<Vec<u8>>,
}

// ── Finish reason ──────────────────────────────────

/// The reason the model stopped generating.
///
/// Covers both OpenAI semantics (`"stop"`, `"length"`, `"tool_calls"`,
/// `"content_filter"`) and Anthropic semantics (`"end_turn"`, `"max_tokens"`,
/// `"stop_sequence"`, `"tool_use"`).
///
/// [`FinishReason::Other`] acts as an escape hatch for provider-specific or
/// future values not yet in the enum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinishReason {
    /// Model reached a natural stopping point.
    Stop,
    /// Model output was truncated due to `max_tokens` or a provider token limit.
    Length,
    /// Model decided to call one or more tools (OpenAI `"tool_calls"`).
    ToolCalls,
    /// Output was omitted due to a content filter flag.
    ContentFilter,
    /// Natural end of a conversational turn (Anthropic `"end_turn"`).
    EndTurn,
    /// Output was truncated due to a token limit (Anthropic / Gemini `"max_tokens"`).
    MaxTokens,
    /// A custom stop sequence was encountered (Anthropic `"stop_sequence"`).
    StopSequence,
    /// Model decided to call a tool (Anthropic `"tool_use"`).
    ToolUse,
    /// Unknown reason (captured verbatim for forward-compatibility).
    Other(String),
}

impl FinishReason {
    /// Human-readable string form matching the llmrust JSON wire format.
    pub fn as_str(&self) -> &str {
        match self {
            FinishReason::Stop => "stop",
            FinishReason::Length => "length",
            FinishReason::ToolCalls => "tool_calls",
            FinishReason::ContentFilter => "content_filter",
            FinishReason::EndTurn => "end_turn",
            FinishReason::MaxTokens => "max_tokens",
            FinishReason::StopSequence => "stop_sequence",
            FinishReason::ToolUse => "tool_use",
            FinishReason::Other(s) => s.as_str(),
        }
    }
}

impl From<String> for FinishReason {
    fn from(s: String) -> Self {
        match s.as_str() {
            "stop" => FinishReason::Stop,
            "length" => FinishReason::Length,
            "tool_calls" => FinishReason::ToolCalls,
            "content_filter" => FinishReason::ContentFilter,
            "end_turn" => FinishReason::EndTurn,
            "max_tokens" => FinishReason::MaxTokens,
            "stop_sequence" => FinishReason::StopSequence,
            "tool_use" => FinishReason::ToolUse,
            _ => FinishReason::Other(s),
        }
    }
}

impl Serialize for FinishReason {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for FinishReason {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(FinishReason::from(s))
    }
}

/// A complete chat completion response.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatResponse {
    pub content: String,
    pub model: String,
    pub usage: Option<Usage>,
    /// Tool calls requested by the model, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// The reason the model stopped generating, when reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<FinishReason>,
    /// Log probabilities for the generated tokens, when requested via
    /// `logprobs` / `top_logprobs` and reported by the provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<LogProbs>,
}

/// A single chunk from a streaming response.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StreamChunk {
    /// The text delta for this chunk (may be empty for the final chunk).
    pub delta: String,
    /// Set to true when the stream is complete.
    pub done: bool,
    /// The provider-reported reason the stream stopped, when available.
    /// Populated on the terminal content chunk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<FinishReason>,
    /// Token usage for the request. Only populated on the terminal usage
    /// chunk, and only for providers that report usage while streaming.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    /// Tool calls reconstructed from the stream, surfaced on the terminal
    /// chunk for providers that support streaming tool calls. `None` for
    /// chunks that carry no tool calls (and for providers, such as Ollama,
    /// that do not reconstruct tool calls while streaming).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// 思考/reasoning 增量（Anthropic `thinking_delta` / OpenAI `reasoning`）。
    /// M2-16：additive，逐块追加而非覆盖。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    /// 思考块结束标记（M2-16）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_done: Option<bool>,
}

/// The format the model's response must take, mirroring OpenAI's
/// `response_format` field.
///
/// Serializes to OpenAI's wire format, e.g. `{"type":"text"}`,
/// `{"type":"json_object"}`, or `{"type":"json_schema","json_schema":{...}}`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseFormat {
    /// Plain text (the provider default).
    Text,
    /// Constrain the output to a syntactically valid JSON object ("JSON mode").
    JsonObject,
    /// Constrain the output to a specific JSON Schema (structured outputs).
    JsonSchema {
        /// The OpenAI `json_schema` object (typically `{name, schema, strict}`).
        json_schema: serde_json::Value,
    },
}

impl ResponseFormat {
    /// JSON mode: any syntactically valid JSON object.
    pub fn json_object() -> Self {
        ResponseFormat::JsonObject
    }

    /// Structured outputs constrained to the given `json_schema` object.
    pub fn json_schema(json_schema: serde_json::Value) -> Self {
        ResponseFormat::JsonSchema { json_schema }
    }
}

/// 思考/reasoning 控制配置（M2-16）。
///
/// 对齐 Anthropic `thinking`（`enabled` + `budget_tokens`）与 OpenAI-style reasoning。
/// 序列化为 provider 友好形态（`disabled` / `enabled`）。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingConfig {
    /// 关闭思考（默认）。
    #[default]
    Disabled,
    /// 启用思考，可选预算 token。
    Enabled {
        /// 思考预算 token 数（Anthropic `budget_tokens`）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        budget_tokens: Option<u64>,
    },
}

/// A chat completion request.
///
/// This struct is marked `#[non_exhaustive]`, which lets new optional fields
/// be added in minor releases without breaking downstream code. Because of
/// that attribute, code **outside this crate** cannot build a `ChatRequest`
/// with struct-literal syntax (not even with `..Default::default()`); use the
/// constructors and builder methods below instead. Public fields may still be
/// assigned directly after construction.
///
/// # Example
///
/// ```rust
/// use llmrust::{ChatRequest, Message};
///
/// // Builder pattern for a single prompt
/// let req = ChatRequest::new("gpt-4o", "Hello!")
///     .with_temperature(0.7)
///     .with_max_tokens(1000);
///
/// // From a pre-built message list (multi-turn / multimodal)
/// let req = ChatRequest::from_messages("gpt-4o", vec![Message::user("Hello!")])
///     .with_stream();
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<u64>,
    pub stream: bool,
    pub top_p: Option<f64>,
    /// Tools the model is allowed to call.
    pub tools: Option<Vec<Tool>>,
    /// Tool-choice policy.
    pub tool_choice: Option<ToolChoice>,
    /// Desired response format (e.g. JSON mode / structured outputs).
    pub response_format: Option<ResponseFormat>,
    /// Sequences that, when produced, stop generation.
    pub stop: Option<Vec<String>>,
    /// Number of completions to generate.
    ///
    /// **Note:** llmrust currently returns only the first completion regardless
    /// of `n`.
    ///
    /// **Provider layer:** `n` may be forwarded to OpenAI/Gemini upstream APIs
    /// and a warning is emitted when `n > 1`. Anthropic ignores `n` entirely.
    ///
    /// **OpenAI-compatible proxy:** `n` must be absent or exactly `1`. The
    /// proxy rejects other values to avoid hidden multi-completion billing
    /// (upstream charges for `n` completions but llmrust only returns one).
    pub n: Option<u32>,
    /// Seed for best-effort deterministic sampling.
    pub seed: Option<i64>,
    /// Penalize tokens by whether they have already appeared (-2.0..=2.0).
    pub presence_penalty: Option<f64>,
    /// Penalize tokens by how often they have appeared (-2.0..=2.0).
    pub frequency_penalty: Option<f64>,
    /// Whether to return log probabilities of the output tokens.
    pub logprobs: Option<bool>,
    /// Number of most-likely tokens to return log probabilities for at each
    /// position (implies `logprobs = true`).
    pub top_logprobs: Option<u32>,
    /// Whether models may execute tool calls in parallel.
    pub parallel_tool_calls: Option<bool>,
    /// OpenAI-compatible processing tier hint (e.g. `"auto"`, `"default"`,
    /// `"flex"`, `"scale"`, `"priority"`).
    pub service_tier: Option<String>,
    /// Whether compatible providers should store the completion, when they
    /// support stored completions.
    pub store: Option<bool>,
    /// Provider-specific request metadata. This is forwarded to
    /// OpenAI-compatible providers when set and is never logged by llmrust.
    pub metadata: Option<serde_json::Value>,
    /// End-user identifier for abuse monitoring on compatible providers.
    pub user: Option<String>,
    /// Provider-specific extra fields that are flattened into the request
    /// body by OpenAI-compatible providers. Useful for forward-compatibility
    /// with provider features not yet first-class in llmrust. Only forwarded
    /// to OpenAI-compatible providers; Anthropic and Google ignore this field.
    pub extra: HashMap<String, serde_json::Value>,
    /// 思考/reasoning 控制（Anthropic `thinking` / OpenAI-style reasoning）。
    /// M2-16：additive，缺省为 `None`（关闭）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingConfig>,
}

impl ChatRequest {
    /// Create a simple single-message request.
    pub fn new(model: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            messages: vec![Message::user(prompt)],
            ..Default::default()
        }
    }

    /// Create a request from a pre-built list of messages.
    ///
    /// Useful for multi-turn conversations or multimodal messages, where the
    /// caller assembles the [`Message`] list directly instead of passing a
    /// single text prompt.
    pub fn from_messages(model: impl Into<String>, messages: Vec<Message>) -> Self {
        Self {
            model: model.into(),
            messages,
            ..Default::default()
        }
    }

    /// Replace the request's message list.
    pub fn with_messages(mut self, messages: Vec<Message>) -> Self {
        self.messages = messages;
        self
    }

    /// Add a system message at the beginning.
    pub fn with_system(mut self, system: impl Into<String>) -> Self {
        self.messages.insert(0, Message::system(system));
        self
    }

    /// Set temperature.
    pub fn with_temperature(mut self, temp: f64) -> Self {
        self.temperature = Some(temp);
        self
    }

    /// Set max tokens.
    pub fn with_max_tokens(mut self, max: u64) -> Self {
        self.max_tokens = Some(max);
        self
    }

    /// Enable streaming.
    pub fn with_stream(mut self) -> Self {
        self.stream = true;
        self
    }

    /// Set top-p nucleus sampling.
    pub fn with_top_p(mut self, top_p: f64) -> Self {
        self.top_p = Some(top_p);
        self
    }

    /// Advertise tools the model may call.
    pub fn with_tools(mut self, tools: Vec<Tool>) -> Self {
        self.tools = Some(tools);
        self
    }

    /// Set the tool-choice policy.
    pub fn with_tool_choice(mut self, tool_choice: ToolChoice) -> Self {
        self.tool_choice = Some(tool_choice);
        self
    }

    /// Set the response format (e.g. [`ResponseFormat::json_object`] for JSON
    /// mode).
    pub fn with_response_format(mut self, format: ResponseFormat) -> Self {
        self.response_format = Some(format);
        self
    }

    /// Shortcut for JSON mode (`response_format = {"type":"json_object"}`).
    pub fn with_json_mode(mut self) -> Self {
        self.response_format = Some(ResponseFormat::JsonObject);
        self
    }

    /// 设置思考/reasoning 控制（M2-16）。
    pub fn with_thinking(mut self, thinking: ThinkingConfig) -> Self {
        self.thinking = Some(thinking);
        self
    }

    /// Set stop sequences.
    pub fn with_stop(mut self, stop: Vec<String>) -> Self {
        self.stop = Some(stop);
        self
    }

    /// Set the number of completions to generate.
    pub fn with_n(mut self, n: u32) -> Self {
        self.n = Some(n);
        self
    }

    /// Set the sampling seed for best-effort determinism.
    pub fn with_seed(mut self, seed: i64) -> Self {
        self.seed = Some(seed);
        self
    }

    /// Set the presence penalty (-2.0..=2.0).
    pub fn with_presence_penalty(mut self, penalty: f64) -> Self {
        self.presence_penalty = Some(penalty);
        self
    }

    /// Set the frequency penalty (-2.0..=2.0).
    pub fn with_frequency_penalty(mut self, penalty: f64) -> Self {
        self.frequency_penalty = Some(penalty);
        self
    }

    /// Request log probabilities for the output tokens.
    pub fn with_logprobs(mut self, logprobs: bool) -> Self {
        self.logprobs = Some(logprobs);
        self
    }

    /// Request the top-N token log probabilities at each position (implies
    /// `logprobs = true`).
    pub fn with_top_logprobs(mut self, top_logprobs: u32) -> Self {
        self.top_logprobs = Some(top_logprobs);
        self
    }

    /// Set whether tool calls may run in parallel.
    pub fn with_parallel_tool_calls(mut self, enabled: bool) -> Self {
        self.parallel_tool_calls = Some(enabled);
        self
    }

    /// Set an OpenAI-compatible processing tier hint.
    pub fn with_service_tier(mut self, tier: impl Into<String>) -> Self {
        self.service_tier = Some(tier.into());
        self
    }

    /// Set whether compatible providers should store the completion.
    pub fn with_store(mut self, store: bool) -> Self {
        self.store = Some(store);
        self
    }

    /// Attach provider-specific request metadata.
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }

    /// Set an end-user identifier for compatible providers.
    pub fn with_user(mut self, user: impl Into<String>) -> Self {
        self.user = Some(user.into());
        self
    }
}

// ── Embeddings ────────────────────────────────────────────────────

/// A single embedding request, wrapping one or more input strings.
///
/// llmrust uses float (`f32`) vectors. Base64 encoding is not supported.
/// Vector dimensions are defined by the upstream provider/model.
///
/// Like [`ChatRequest`], this struct is marked `#[non_exhaustive]`, which lets
/// new optional fields be added in minor releases without breaking downstream
/// code. Because of that attribute, code **outside this crate** cannot build an
/// `EmbeddingRequest` with struct-literal syntax; use [`EmbeddingRequest::new`]
/// or [`EmbeddingRequest::batch`] and the builder methods below instead.
/// Public fields may still be assigned directly after construction.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct EmbeddingRequest {
    pub model: String,
    pub input: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// Provider-specific extra fields, merged into the request body by
    /// providers that support it (e.g. Ollama reads `truncate`, `options`, and
    /// `keep_alive` from here). Keys should not shadow the first-class fields
    /// above (`model`, `input`, `dimensions`, `user`), as doing so can produce
    /// duplicate keys in the serialized body.
    #[serde(default, skip_serializing_if = "HashMap::is_empty", flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl EmbeddingRequest {
    /// Create an embedding request for a single input string.
    pub fn new(model: impl Into<String>, input: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            input: vec![input.into()],
            dimensions: None,
            user: None,
            extra: HashMap::new(),
        }
    }

    /// Create an embedding request for multiple input strings.
    pub fn batch<I, S>(model: impl Into<String>, inputs: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            model: model.into(),
            input: inputs.into_iter().map(Into::into).collect(),
            dimensions: None,
            user: None,
            extra: HashMap::new(),
        }
    }

    /// Request a specific embedding dimension (provider-dependent).
    pub fn with_dimensions(mut self, dimensions: u32) -> Self {
        self.dimensions = Some(dimensions);
        self
    }

    /// Set an end-user identifier for compatible providers.
    pub fn with_user(mut self, user: impl Into<String>) -> Self {
        self.user = Some(user.into());
        self
    }

    /// Attach a provider-specific extra field.
    pub fn with_extra(
        mut self,
        key: impl Into<String>,
        value: impl Into<serde_json::Value>,
    ) -> Self {
        self.extra.insert(key.into(), value.into());
        self
    }
}

/// A single embedding vector.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Embedding {
    /// Positional index in the batch (0-based, matches `input` order).
    pub index: usize,
    /// The embedding vector itself.
    pub embedding: Vec<f32>,
}

/// Usage statistics reported by the upstream API.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EmbeddingUsage {
    pub prompt_tokens: u64,
    pub total_tokens: u64,
}

/// The response from an embeddings provider.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EmbeddingResponse {
    pub model: String,
    pub data: Vec<Embedding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<EmbeddingUsage>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_choice_mode_serializes_as_string() {
        assert_eq!(
            serde_json::to_value(ToolChoice::auto()).unwrap(),
            serde_json::json!("auto")
        );
        assert_eq!(
            serde_json::to_value(ToolChoice::required()).unwrap(),
            serde_json::json!("required")
        );
    }

    #[test]
    fn tool_choice_function_serializes_as_object() {
        let v = serde_json::to_value(ToolChoice::function("get_weather")).unwrap();
        assert_eq!(v["type"], "function");
        assert_eq!(v["function"]["name"], "get_weather");
    }

    #[test]
    fn tool_serializes_with_function_schema() {
        let tool = Tool::function(
            "get_weather",
            Some("Get the weather".to_string()),
            serde_json::json!({"type": "object"}),
        );
        let v = serde_json::to_value(&tool).unwrap();
        assert_eq!(v["type"], "function");
        assert_eq!(v["function"]["name"], "get_weather");
        assert_eq!(v["function"]["description"], "Get the weather");
        assert_eq!(v["function"]["parameters"]["type"], "object");
    }

    #[test]
    fn tool_message_has_role_and_id() {
        let msg = Message::tool("call_1", "result");
        assert_eq!(msg.role, Role::Tool);
        assert_eq!(msg.tool_call_id.as_deref(), Some("call_1"));
    }

    #[test]
    fn content_text_serializes_as_string() {
        let c = Content::Text("hi".to_string());
        assert_eq!(serde_json::to_value(&c).unwrap(), serde_json::json!("hi"));
    }

    #[test]
    fn content_parts_serialize_as_openai_array() {
        let c = Content::Parts(vec![
            ContentPart::text("look"),
            ContentPart::image_url("https://example.com/a.png"),
        ]);
        let v = serde_json::to_value(&c).unwrap();
        assert!(v.is_array());
        assert_eq!(v[0]["type"], "text");
        assert_eq!(v[0]["text"], "look");
        assert_eq!(v[1]["type"], "image_url");
        assert_eq!(v[1]["image_url"]["url"], "https://example.com/a.png");
        assert!(v[1]["image_url"].get("detail").is_none());
    }

    #[test]
    fn content_deserializes_from_string_and_array() {
        let from_str: Content = serde_json::from_value(serde_json::json!("hi")).unwrap();
        assert_eq!(from_str, Content::Text("hi".to_string()));

        let from_arr: Content = serde_json::from_value(serde_json::json!([
            {"type": "text", "text": "a"},
            {"type": "image_url", "image_url": {"url": "u"}}
        ]))
        .unwrap();
        assert_eq!(from_arr.images().len(), 1);
        assert_eq!(from_arr.as_text(), "a");
    }

    #[test]
    fn message_user_with_image_holds_parts() {
        let msg = Message::user_with_image("desc", "https://example.com/a.png");
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.content.images().len(), 1);
        assert_eq!(msg.content.as_text(), "desc");
    }

    #[test]
    fn response_format_serializes_as_openai_wire_format() {
        assert_eq!(
            serde_json::to_value(ResponseFormat::Text).unwrap(),
            serde_json::json!({"type": "text"})
        );
        assert_eq!(
            serde_json::to_value(ResponseFormat::json_object()).unwrap(),
            serde_json::json!({"type": "json_object"})
        );
        let schema = serde_json::json!({"name": "person", "schema": {"type": "object"}});
        let v = serde_json::to_value(ResponseFormat::json_schema(schema.clone())).unwrap();
        assert_eq!(v["type"], "json_schema");
        assert_eq!(v["json_schema"], schema);
    }

    #[test]
    fn chat_request_sampling_builders_set_fields() {
        let req = ChatRequest::new("gpt-4o", "hi")
            .with_json_mode()
            .with_stop(vec!["\n".to_string()])
            .with_seed(42)
            .with_top_p(0.9)
            .with_n(2)
            .with_presence_penalty(0.5)
            .with_frequency_penalty(-0.3)
            .with_logprobs(true)
            .with_top_logprobs(5)
            .with_parallel_tool_calls(false)
            .with_service_tier("flex")
            .with_store(true)
            .with_metadata(serde_json::json!({"trace_id": "abc"}))
            .with_user("user-123");

        assert_eq!(req.response_format, Some(ResponseFormat::JsonObject));
        assert_eq!(req.stop, Some(vec!["\n".to_string()]));
        assert_eq!(req.seed, Some(42));
        assert_eq!(req.top_p, Some(0.9));
        assert_eq!(req.n, Some(2));
        assert_eq!(req.presence_penalty, Some(0.5));
        assert_eq!(req.frequency_penalty, Some(-0.3));
        assert_eq!(req.logprobs, Some(true));
        assert_eq!(req.top_logprobs, Some(5));
        assert_eq!(req.parallel_tool_calls, Some(false));
        assert_eq!(req.service_tier.as_deref(), Some("flex"));
        assert_eq!(req.store, Some(true));
        assert_eq!(req.metadata, Some(serde_json::json!({"trace_id": "abc"})));
        assert_eq!(req.user.as_deref(), Some("user-123"));
    }

    #[test]
    fn chat_request_from_messages_sets_messages() {
        let req = ChatRequest::from_messages("gpt-4o", vec![Message::user("hi")]);
        assert_eq!(req.model, "gpt-4o");
        assert_eq!(req.messages.len(), 1);

        let req = req.with_messages(vec![Message::user("a"), Message::user("b")]);
        assert_eq!(req.messages.len(), 2);
    }

    #[test]
    fn stream_chunk_serializes_tool_calls_only_when_present() {
        let empty = StreamChunk::default();
        let v = serde_json::to_value(&empty).unwrap();
        assert!(v.get("tool_calls").is_none());

        let chunk = StreamChunk {
            done: true,
            finish_reason: Some(FinishReason::ToolCalls),
            tool_calls: Some(vec![ToolCall {
                id: "call_1".to_string(),
                call_type: "function".to_string(),
                function: FunctionCall {
                    name: "get_weather".to_string(),
                    arguments: "{\"city\":\"SF\"}".to_string(),
                },
            }]),
            ..Default::default()
        };
        let v = serde_json::to_value(&chunk).unwrap();
        assert_eq!(v["tool_calls"][0]["id"], "call_1");
        assert_eq!(v["tool_calls"][0]["function"]["name"], "get_weather");
        assert_eq!(v["finish_reason"], "tool_calls");
    }

    #[test]
    fn chat_response_serializes_logprobs_only_when_present() {
        let resp = ChatResponse {
            content: "Hi".to_string(),
            model: "gpt-4o".to_string(),
            ..Default::default()
        };
        let v = serde_json::to_value(&resp).unwrap();
        assert!(v.get("logprobs").is_none());

        let resp = ChatResponse {
            content: "Hi".to_string(),
            model: "gpt-4o".to_string(),
            logprobs: Some(LogProbs {
                content: vec![TokenLogProb {
                    token: "Hi".to_string(),
                    logprob: -0.25,
                    bytes: Some(vec![72, 105]),
                    top_logprobs: vec![TopLogProb {
                        token: "Hi".to_string(),
                        logprob: -0.25,
                        bytes: Some(vec![72, 105]),
                    }],
                }],
            }),
            ..Default::default()
        };
        let v = serde_json::to_value(&resp).unwrap();
        assert_eq!(v["logprobs"]["content"][0]["token"], "Hi");
        assert_eq!(v["logprobs"]["content"][0]["logprob"], -0.25);
        assert_eq!(
            v["logprobs"]["content"][0]["top_logprobs"][0]["token"],
            "Hi"
        );
    }
}
