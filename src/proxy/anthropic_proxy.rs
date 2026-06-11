//! Anthropic Messages API protocol handler for the proxy server.
//!
//! Provides a `/v1/messages` endpoint that speaks Anthropic's native protocol,
//! allowing Anthropic SDK clients (Claude Code, anthropic-python, etc.) to
//! talk to any registered provider through automatic format conversion.

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use bytes::Bytes;
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};

use crate::types::{
    ChatRequest, ChatResponse, Content, ContentPart, FinishReason, FunctionCall, Message,
    StreamChunk, Tool, ToolCall, ToolChoice, Usage,
};
use crate::LlmError;

use super::{generate_id, split_model, AppState};

// ── Anthropic API types (request) ──────────────────────

/// Anthropic Messages API request.
#[derive(Debug, Deserialize)]
pub struct AnthropicRequest {
    pub model: String,
    #[serde(default)]
    pub messages: Vec<AnthropicMessage>,
    #[serde(default)]
    pub system: Option<AnthropicSystemContent>,
    #[serde(default)]
    pub max_tokens: Option<u64>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub top_p: Option<f64>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub stop_sequences: Option<Vec<String>>,
    #[serde(default)]
    pub tools: Option<Vec<AnthropicToolDef>>,
    #[serde(default)]
    pub tool_choice: Option<AnthropicToolChoiceDef>,
}

/// System prompt: either a plain string or an array of text blocks.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum AnthropicSystemContent {
    Text(String),
    Blocks(Vec<AnthropicSystemBlock>),
}

#[derive(Debug, Deserialize)]
pub struct AnthropicSystemBlock {
    #[serde(rename = "type", default)]
    #[allow(dead_code)]
    pub block_type: Option<String>,
    pub text: String,
}

/// A message in the conversation. `content` can be a plain string or an array
/// of typed content blocks (text, image, tool_use, tool_result).
#[derive(Debug, Deserialize)]
pub struct AnthropicMessage {
    pub role: String,
    pub content: AnthropicMessageContent,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum AnthropicMessageContent {
    Text(String),
    Blocks(Vec<AnthropicContentBlock>),
}

/// Typed content block: text, image, tool_use, or tool_result.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnthropicContentBlock {
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
        content: AnthropicToolResultContent,
    },
}

/// Tool result content: either a string or content blocks.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum AnthropicToolResultContent {
    Text(String),
    Blocks(Vec<AnthropicToolResultBlock>),
}

#[derive(Debug, Deserialize)]
pub struct AnthropicToolResultBlock {
    #[serde(rename = "type", default)]
    #[allow(dead_code)]
    pub block_type: Option<String>,
    pub text: String,
}

/// Image source: base64 inline data or a URL.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnthropicImageSource {
    Base64 { media_type: String, data: String },
    Url { url: String },
}

/// Tool definition in Anthropic format.
#[derive(Debug, Deserialize)]
pub struct AnthropicToolDef {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub input_schema: serde_json::Value,
}

/// Tool choice in Anthropic format.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnthropicToolChoiceDef {
    Auto,
    Any,
    None,
    Tool { name: String },
}

// ── Anthropic API types (response) ──────────────────────

/// Non-streaming Messages API response.
#[derive(Debug, Serialize)]
pub struct AnthropicResponse {
    pub id: String,
    #[serde(rename = "type")]
    pub response_type: String,
    pub role: String,
    pub content: Vec<AnthropicResponseBlock>,
    pub model: String,
    pub stop_reason: Option<String>,
    pub stop_sequence: Option<String>,
    pub usage: AnthropicUsageOut,
}

/// A content block in the response: text or tool_use.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnthropicResponseBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
}

/// Usage stats in Anthropic format.
#[derive(Debug, Serialize)]
pub struct AnthropicUsageOut {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

// ── Anthropic API types (streaming events) ──────────────────

/// Error response body.
#[derive(Debug, Serialize)]
pub struct AnthropicErrorBody {
    #[serde(rename = "type")]
    pub error_type: String,
    pub error: AnthropicErrorDetail,
}

#[derive(Debug, Serialize)]
pub struct AnthropicErrorDetail {
    #[serde(rename = "type")]
    pub detail_type: String,
    pub message: String,
}

// ── Request conversion: Anthropic → ChatRequest ──────────────

/// Convert an Anthropic request into an internal ChatRequest.
pub fn convert_request(req: &AnthropicRequest) -> Result<ChatRequest, String> {
    let mut messages: Vec<Message> = Vec::new();

    // System prompt → System message
    if let Some(system) = &req.system {
        let text = match system {
            AnthropicSystemContent::Text(t) => t.clone(),
            AnthropicSystemContent::Blocks(blocks) => blocks
                .iter()
                .map(|b| b.text.as_str())
                .collect::<Vec<_>>()
                .join(""),
        };
        if !text.is_empty() {
            messages.push(Message::system(text));
        }
    }

    // Messages
    for msg in &req.messages {
        match msg.role.as_str() {
            "user" => {
                match &msg.content {
                    AnthropicMessageContent::Text(t) => {
                        messages.push(Message::user(t.as_str()));
                    }
                    AnthropicMessageContent::Blocks(blocks) => {
                        // Separate tool_result blocks from regular content blocks.
                        // In Anthropic's protocol, tool results come inside user messages
                        // but they map to `role: tool` in OpenAI format.
                        let regular: Vec<&AnthropicContentBlock> = blocks
                            .iter()
                            .filter(|b| !matches!(b, AnthropicContentBlock::ToolResult { .. }))
                            .collect();

                        // Emit regular content as user message(s)
                        if !regular.is_empty() {
                            let parts: Vec<ContentPart> = regular
                                .iter()
                                .filter_map(|block| match block {
                                    AnthropicContentBlock::Text { text } => {
                                        Some(ContentPart::text(text))
                                    }
                                    AnthropicContentBlock::Image { source } => {
                                        let url = match source {
                                            AnthropicImageSource::Base64 { media_type, data } => {
                                                format!("data:{media_type};base64,{data}")
                                            }
                                            AnthropicImageSource::Url { url } => url.clone(),
                                        };
                                        Some(ContentPart::image_url(url))
                                    }
                                    _ => None,
                                })
                                .collect();
                            let parts = if parts.is_empty() {
                                vec![ContentPart::text("")]
                            } else {
                                parts
                            };
                            if parts.len() == 1 {
                                if let ContentPart::Text { text } = &parts[0] {
                                    messages.push(Message::user(text.as_str()));
                                } else {
                                    messages.push(Message::user_with_parts(parts));
                                }
                            } else {
                                messages.push(Message::user_with_parts(parts));
                            }
                        }

                        // Emit each tool_result as a separate tool message
                        for block in blocks {
                            if let AnthropicContentBlock::ToolResult {
                                tool_use_id,
                                content,
                            } = block
                            {
                                let text = match content {
                                    AnthropicToolResultContent::Text(t) => t.clone(),
                                    AnthropicToolResultContent::Blocks(bs) => bs
                                        .iter()
                                        .map(|b| b.text.as_str())
                                        .collect::<Vec<_>>()
                                        .join(""),
                                };
                                messages.push(Message::tool(tool_use_id.as_str(), text));
                            }
                        }
                    }
                };
            }
            "assistant" => {
                match &msg.content {
                    AnthropicMessageContent::Text(t) => {
                        messages.push(Message::assistant(t.as_str()));
                    }
                    AnthropicMessageContent::Blocks(blocks) => {
                        let mut text_parts = Vec::new();
                        let mut tool_calls = Vec::new();
                        for block in blocks {
                            match block {
                                AnthropicContentBlock::Text { text } => {
                                    text_parts.push(text.as_str());
                                }
                                AnthropicContentBlock::ToolUse { id, name, input } => {
                                    tool_calls.push(ToolCall {
                                        id: id.clone(),
                                        call_type: "function".to_string(),
                                        function: FunctionCall {
                                            name: name.clone(),
                                            arguments: input.to_string(),
                                        },
                                    });
                                }
                                _ => {} // ignore other blocks in assistant messages
                            }
                        }
                        if !tool_calls.is_empty() {
                            let mut msg = Message::assistant_tool_calls(tool_calls);
                            let text = text_parts.join("");
                            if !text.is_empty() {
                                msg.content = Content::Text(text);
                            }
                            messages.push(msg);
                        } else {
                            messages.push(Message::assistant(text_parts.join("")));
                        }
                    }
                }
            }
            other => return Err(format!("Unknown role: {other}")),
        }
    }

    // Convert tools
    let tools: Option<Vec<Tool>> = req.tools.as_ref().map(|ts| {
        ts.iter()
            .map(|t| {
                Tool::function(
                    t.name.clone(),
                    t.description.clone(),
                    t.input_schema.clone(),
                )
            })
            .collect()
    });

    // Convert tool_choice
    let tool_choice: Option<ToolChoice> = req.tool_choice.as_ref().map(|tc| match tc {
        AnthropicToolChoiceDef::Auto => ToolChoice::auto(),
        AnthropicToolChoiceDef::Any => ToolChoice::required(),
        AnthropicToolChoiceDef::None => ToolChoice::none(),
        AnthropicToolChoiceDef::Tool { name } => ToolChoice::function(name.as_str()),
    });

    let mut chat_req = ChatRequest::from_messages("", messages);
    chat_req.temperature = req.temperature;
    chat_req.max_tokens = req.max_tokens;
    chat_req.top_p = req.top_p;
    chat_req.stop = req.stop_sequences.clone();
    chat_req.tools = tools;
    chat_req.tool_choice = tool_choice;
    chat_req.stream = req.stream;

    Ok(chat_req)
}

// ── Response conversion: ChatResponse → AnthropicResponse ────────

/// Convert an internal ChatResponse into an Anthropic Messages API response.
pub fn build_response(resp: ChatResponse, id: &str) -> AnthropicResponse {
    let mut content: Vec<AnthropicResponseBlock> = Vec::new();

    if !resp.content.is_empty() {
        content.push(AnthropicResponseBlock::Text { text: resp.content });
    }

    if let Some(tool_calls) = &resp.tool_calls {
        for tc in tool_calls {
            let input: serde_json::Value = serde_json::from_str(&tc.function.arguments)
                .unwrap_or_else(|_| serde_json::json!({}));
            content.push(AnthropicResponseBlock::ToolUse {
                id: tc.id.clone(),
                name: tc.function.name.clone(),
                input,
            });
        }
    }

    let stop_reason = resp.finish_reason.as_ref().map(normalize_stop_reason);
    let (input_tokens, output_tokens) = match &resp.usage {
        Some(u) => (u.prompt_tokens, u.completion_tokens),
        None => (0, 0),
    };

    AnthropicResponse {
        id: id.to_string(),
        response_type: "message".to_string(),
        role: "assistant".to_string(),
        content,
        model: resp.model,
        stop_reason,
        stop_sequence: None,
        usage: AnthropicUsageOut {
            input_tokens,
            output_tokens,
        },
    }
}

/// Normalize a provider-specific stop reason to Anthropic conventions.
fn normalize_stop_reason(reason: &FinishReason) -> String {
    match reason {
        FinishReason::Stop | FinishReason::EndTurn => "end_turn".to_string(),
        FinishReason::ToolCalls | FinishReason::ToolUse => "tool_use".to_string(),
        FinishReason::Length | FinishReason::MaxTokens => "max_tokens".to_string(),
        other => other.as_str().to_string(),
    }
}

// ── Streaming handler ──────────────────────────────────

/// Streaming state for synthesizing Anthropic SSE event sequences.
struct AnthropicStreamState {
    inner: futures::stream::BoxStream<'static, Result<StreamChunk, LlmError>>,
    id: String,
    model: String,
    started: bool,
    in_text_block: bool,
    next_block_index: usize,
    current_text_index: Option<usize>,
    pending_events: Vec<String>,
    usage: Option<Usage>,
    done_emitted: bool,
}

impl AnthropicStreamState {
    fn new(
        inner: futures::stream::BoxStream<'static, Result<StreamChunk, LlmError>>,
        id: String,
        model: String,
    ) -> Self {
        Self {
            inner,
            id,
            model,
            started: false,
            in_text_block: false,
            next_block_index: 0,
            current_text_index: None,
            pending_events: Vec::new(),
            usage: None,
            done_emitted: false,
        }
    }

    /// Format an SSE event with optional event name and data.
    fn sse_event(event: &str, data: &str) -> String {
        if event.is_empty() {
            format!("data: {data}\n\n")
        } else {
            format!("event: {event}\ndata: {data}\n\n")
        }
    }

    /// Process an incoming chunk and produce zero or more SSE events.
    fn process_chunk(&mut self, chunk: StreamChunk) -> Vec<String> {
        let mut events = Vec::new();

        // Capture usage from any chunk
        if let Some(u) = chunk.usage {
            self.usage = Some(u);
        }

        // Emit message_start on the very first chunk
        if !self.started {
            self.started = true;
            let (input_tokens, _output_tokens) = match &self.usage {
                Some(u) => (u.prompt_tokens, u.completion_tokens),
                None => (0, 0),
            };
            events.push(Self::sse_event(
                "message_start",
                &serde_json::json!({
                    "type": "message_start",
                    "message": {
                        "id": self.id,
                        "type": "message",
                        "role": "assistant",
                        "content": [],
                        "model": self.model,
                        "stop_reason": null,
                        "stop_sequence": null,
                        "usage": {
                            "input_tokens": input_tokens,
                            "output_tokens": 0
                        }
                    }
                })
                .to_string(),
            ));
        }

        let has_text = !chunk.delta.is_empty();
        let has_tools = chunk.tool_calls.as_ref().is_some_and(|c| !c.is_empty());

        // Open text block for text content
        if has_text && !self.in_text_block {
            let index = self.next_block_index;
            self.next_block_index += 1;
            self.current_text_index = Some(index);
            events.push(Self::sse_event(
                "content_block_start",
                &serde_json::json!({
                    "type": "content_block_start",
                    "index": index,
                    "content_block": { "type": "text", "text": "" }
                })
                .to_string(),
            ));
            self.in_text_block = true;
        }

        // Emit text delta
        if has_text {
            if let Some(idx) = self.current_text_index {
                events.push(Self::sse_event(
                    "content_block_delta",
                    &serde_json::json!({
                        "type": "content_block_delta",
                        "index": idx,
                        "delta": { "type": "text_delta", "text": chunk.delta }
                    })
                    .to_string(),
                ));
            }
        }

        // Close text block before tool calls — handles both
        // prior-chunk text and same-chunk text.
        if has_tools && self.in_text_block {
            if let Some(idx) = self.current_text_index {
                events.push(Self::sse_event(
                    "content_block_stop",
                    &serde_json::json!({
                        "type": "content_block_stop",
                        "index": idx
                    })
                    .to_string(),
                ));
            }
            self.in_text_block = false;
            self.current_text_index = None;
        }

        // Emit tool use blocks
        if let Some(tool_calls) = &chunk.tool_calls {
            for tc in tool_calls {
                let index = self.next_block_index;
                self.next_block_index += 1;

                events.push(Self::sse_event(
                    "content_block_start",
                    &serde_json::json!({
                        "type": "content_block_start",
                        "index": index,
                        "content_block": {
                            "type": "tool_use",
                            "id": tc.id,
                            "name": tc.function.name,
                            "input": {}
                        }
                    })
                    .to_string(),
                ));

                events.push(Self::sse_event(
                    "content_block_delta",
                    &serde_json::json!({
                        "type": "content_block_delta",
                        "index": index,
                        "delta": {
                            "type": "input_json_delta",
                            "partial_json": tc.function.arguments
                        }
                    })
                    .to_string(),
                ));

                events.push(Self::sse_event(
                    "content_block_stop",
                    &serde_json::json!({
                        "type": "content_block_stop",
                        "index": index
                    })
                    .to_string(),
                ));
            }
        }

        // Terminal chunk: close blocks and emit message_delta + message_stop
        if chunk.done || chunk.finish_reason.is_some() {
            if let Some(idx) = self.current_text_index {
                events.push(Self::sse_event(
                    "content_block_stop",
                    &serde_json::json!({
                        "type": "content_block_stop",
                        "index": idx
                    })
                    .to_string(),
                ));
                self.in_text_block = false;
                self.current_text_index = None;
            }

            // If no content was emitted at all, emit an empty text block.
            // `self.started` is always true here (set on the first chunk).
            if !has_text && !has_tools && self.next_block_index == 0 && events.is_empty() {
                events.push(Self::sse_event(
                    "content_block_start",
                    &serde_json::json!({
                        "type": "content_block_start",
                        "index": 0,
                        "content_block": { "type": "text", "text": "" }
                    })
                    .to_string(),
                ));
                events.push(Self::sse_event(
                    "content_block_stop",
                    &serde_json::json!({
                        "type": "content_block_stop",
                        "index": 0
                    })
                    .to_string(),
                ));
                self.next_block_index = 1;
            }

            let stop_reason = chunk
                .finish_reason
                .as_ref()
                .map(normalize_stop_reason)
                .unwrap_or_else(|| {
                    if has_tools {
                        "tool_use".to_string()
                    } else {
                        "end_turn".to_string()
                    }
                });

            let output_tokens = self.usage.as_ref().map_or(0, |u| u.completion_tokens);

            events.push(Self::sse_event(
                "message_delta",
                &serde_json::json!({
                    "type": "message_delta",
                    "delta": { "stop_reason": stop_reason, "stop_sequence": null },
                    "usage": { "output_tokens": output_tokens }
                })
                .to_string(),
            ));

            events.push(Self::sse_event(
                "message_stop",
                "{\"type\":\"message_stop\"}",
            ));
            self.done_emitted = true;
        }

        events
    }

    /// Flush any remaining events (called when the inner stream is exhausted).
    fn flush(&mut self) -> Vec<String> {
        if self.done_emitted {
            return Vec::new();
        }
        let mut events = Vec::new();
        if let Some(idx) = self.current_text_index {
            events.push(Self::sse_event(
                "content_block_stop",
                &serde_json::json!({
                    "type": "content_block_stop",
                    "index": idx
                })
                .to_string(),
            ));
        }
        events.push(Self::sse_event(
            "message_delta",
            &serde_json::json!({
                "type": "message_delta",
                "delta": { "stop_reason": "end_turn", "stop_sequence": null },
                "usage": { "output_tokens": self.usage.as_ref().map_or(0, |u| u.completion_tokens) }
            })
            .to_string(),
        ));
        events.push(Self::sse_event(
            "message_stop",
            "{\"type\":\"message_stop\"}",
        ));
        self.done_emitted = true;
        events
    }
}

/// Build the streaming byte response for Anthropic protocol.
pub fn build_stream_response(
    inner_stream: futures::stream::BoxStream<'static, Result<StreamChunk, LlmError>>,
    id: String,
    model: String,
) -> Response {
    let state = AnthropicStreamState::new(inner_stream, id, model);

    let byte_stream = stream::unfold(state, |mut state| async move {
        loop {
            // Drain pending events first
            if !state.pending_events.is_empty() {
                let event = state.pending_events.remove(0);
                return Some((Ok::<_, std::convert::Infallible>(Bytes::from(event)), state));
            }

            // Pull next chunk from inner stream
            match state.inner.next().await {
                Some(Ok(chunk)) => {
                    let events = state.process_chunk(chunk);
                    if events.is_empty() {
                        // No events produced, continue to next chunk
                        continue;
                    }
                    let first = events[0].clone();
                    state.pending_events = events[1..].to_vec();
                    return Some((Ok(Bytes::from(first)), state));
                }
                Some(Err(e)) => {
                    state.pending_events.clear();
                    state.done_emitted = true;
                    let error_event = AnthropicStreamState::sse_event(
                        "error",
                        &serde_json::json!({
                            "type": "error",
                            "error": {
                                "type": "api_error",
                                "message": e.to_string()
                            }
                        })
                        .to_string(),
                    );
                    return Some((Ok(Bytes::from(error_event)), state));
                }
                None => {
                    let events = state.flush();
                    if events.is_empty() {
                        return None;
                    }
                    let first = events[0].clone();
                    state.pending_events = events[1..].to_vec();
                    return Some((Ok(Bytes::from(first)), state));
                }
            }
        }
    })
    .filter(|result| {
        let bytes = match result {
            Ok(b) => b,
            Err(_) => return futures::future::ready(true),
        };
        futures::future::ready(!bytes.is_empty())
    });

    let body = axum::body::Body::from_stream(byte_stream);
    (
        StatusCode::OK,
        [
            (axum::http::header::CONTENT_TYPE, "text/event-stream"),
            (axum::http::header::CACHE_CONTROL, "no-cache"),
            (axum::http::header::CONNECTION, "keep-alive"),
        ],
        body,
    )
        .into_response()
}

// ── Handlers ──────────────────────────────────────────

/// Handle POST /v1/messages.
pub async fn handle_messages(
    State(state): State<AppState>,
    Json(req): Json<AnthropicRequest>,
) -> Response {
    let stream = req.stream;
    let model = req.model.clone();

    tracing::info!(model = &model, stream, "proxy: Anthropic messages request");
    let chat_req = match convert_request(&req) {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(
                model = &model,
                error = &e,
                "proxy: Anthropic request conversion failed"
            );
            return anthropic_error(StatusCode::BAD_REQUEST, "invalid_request_error", &e);
        }
    };

    if stream {
        handle_stream(state, &model, chat_req).await
    } else {
        handle_non_stream(state, &model, chat_req).await
    }
}

async fn handle_non_stream(state: AppState, model: &str, req: ChatRequest) -> Response {
    if let Err(e) = split_model(model) {
        return anthropic_error(StatusCode::BAD_REQUEST, "invalid_request_error", e);
    }
    let id = generate_id();
    match state.llm.chat_with(model, req).await {
        Ok(resp) => {
            let anthropic_resp = build_response(resp, &id);
            Json(anthropic_resp).into_response()
        }
        Err(e) => anthropic_error_from_llm_error(e),
    }
}

async fn handle_stream(state: AppState, model: &str, mut req: ChatRequest) -> Response {
    let (provider_name, model_name) = match split_model(model) {
        Ok(pair) => pair,
        Err(e) => {
            return anthropic_error(StatusCode::BAD_REQUEST, "invalid_request_error", e);
        }
    };

    req.model = model_name.to_string();
    req.stream = true;

    let provider = match state.llm.get_provider(provider_name).await {
        Ok(p) => p,
        Err(e) => return anthropic_error_from_llm_error(e),
    };

    let id = generate_id();
    match provider.stream(&req).await {
        Ok(inner_stream) => build_stream_response(inner_stream, id, model_name.to_string()),
        Err(e) => anthropic_error_from_llm_error(e),
    }
}

// ── Error helpers ──────────────────────────────────────────

fn anthropic_error(status: StatusCode, error_type: &str, message: &str) -> Response {
    let body = AnthropicErrorBody {
        error_type: "error".to_string(),
        error: AnthropicErrorDetail {
            detail_type: error_type.to_string(),
            message: message.to_string(),
        },
    };
    (status, Json(body)).into_response()
}

fn anthropic_error_from_llm_error(e: LlmError) -> Response {
    let (status, error_type) = match &e {
        LlmError::Api { status, .. } => {
            let code = StatusCode::from_u16(*status).unwrap_or(StatusCode::BAD_GATEWAY);
            (code, "api_error")
        }
        LlmError::UnknownProvider(_) => (StatusCode::NOT_FOUND, "not_found_error"),
        LlmError::Parse(_) => (StatusCode::BAD_REQUEST, "invalid_request_error"),
        _ => (StatusCode::BAD_GATEWAY, "api_error"),
    };
    anthropic_error(status, error_type, &e.to_string())
}

// ── Tests ──────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChatResponse, FunctionCall, Role, StreamChunk, ToolCall, Usage};
    use crate::LmrsClient;
    use axum::http::StatusCode;

    #[test]
    fn text_request_converts() {
        let raw = serde_json::json!({
            "model": "claude-sonnet-4-20250514",
            "messages": [{"role": "user", "content": "Hello!"}],
            "max_tokens": 1024
        });
        let req: AnthropicRequest = serde_json::from_value(raw).unwrap();
        let chat_req = convert_request(&req).unwrap();
        assert_eq!(chat_req.messages.len(), 1);
        assert_eq!(chat_req.messages[0].role, Role::User);
        assert_eq!(chat_req.messages[0].content.as_text(), "Hello!");
        assert_eq!(chat_req.max_tokens, Some(1024));
    }

    #[test]
    fn system_prompt_text_converts() {
        let raw = serde_json::json!({
            "model": "claude",
            "system": "Be helpful",
            "messages": [{"role": "user", "content": "hi"}]
        });
        let req: AnthropicRequest = serde_json::from_value(raw).unwrap();
        let chat_req = convert_request(&req).unwrap();
        assert_eq!(chat_req.messages.len(), 2);
        assert_eq!(chat_req.messages[0].role, Role::System);
        assert_eq!(chat_req.messages[0].content.as_text(), "Be helpful");
    }

    #[test]
    fn system_prompt_blocks_converts() {
        let raw = serde_json::json!({
            "model": "claude",
            "system": [{"type": "text", "text": "part1"}, {"type": "text", "text": "part2"}],
            "messages": [{"role": "user", "content": "hi"}]
        });
        let req: AnthropicRequest = serde_json::from_value(raw).unwrap();
        let chat_req = convert_request(&req).unwrap();
        assert_eq!(chat_req.messages[0].content.as_text(), "part1part2");
    }

    #[test]
    fn image_content_block_converts() {
        let raw = serde_json::json!({
            "model": "claude",
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": "What is this?"},
                    {"type": "image", "source": {"type": "base64", "media_type": "image/jpeg", "data": "abc"}}
                ]
            }]
        });
        let req: AnthropicRequest = serde_json::from_value(raw).unwrap();
        let chat_req = convert_request(&req).unwrap();
        let msg = &chat_req.messages[0];
        let images = msg.content.images();
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].url, "data:image/jpeg;base64,abc");
    }

    #[test]
    fn tool_use_in_assistant_converts() {
        let raw = serde_json::json!({
            "model": "claude",
            "messages": [
                {"role": "user", "content": "weather?"},
                {"role": "assistant", "content": [
                    {"type": "tool_use", "id": "toolu_1", "name": "get_weather", "input": {"city": "SF"}}
                ]}
            ]
        });
        let req: AnthropicRequest = serde_json::from_value(raw).unwrap();
        let chat_req = convert_request(&req).unwrap();
        let assistant_msg = &chat_req.messages[1];
        let calls = assistant_msg.tool_calls.as_ref().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "toolu_1");
        assert_eq!(calls[0].function.name, "get_weather");
    }

    #[test]
    fn tool_result_converts() {
        let raw = serde_json::json!({
            "model": "claude",
            "messages": [
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": [
                    {"type": "tool_use", "id": "t1", "name": "f", "input": {}}
                ]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "t1", "content": "sunny"}
                ]}
            ]
        });
        let req: AnthropicRequest = serde_json::from_value(raw).unwrap();
        let chat_req = convert_request(&req).unwrap();
        assert_eq!(chat_req.messages.len(), 3);
        // user "hi", assistant with tool_call, tool result
        assert_eq!(chat_req.messages[0].role, Role::User);
        assert_eq!(chat_req.messages[1].role, Role::Assistant);
        assert!(chat_req.messages[1].tool_calls.is_some());
        assert_eq!(chat_req.messages[2].role, Role::Tool);
        assert_eq!(chat_req.messages[2].tool_call_id.as_deref(), Some("t1"));
        assert_eq!(chat_req.messages[2].content.as_text(), "sunny");
    }

    #[test]
    fn tools_and_tool_choice_convert() {
        let raw = serde_json::json!({
            "model": "claude",
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{
                "name": "get_weather",
                "description": "Get weather",
                "input_schema": {"type": "object", "properties": {"city": {"type": "string"}}}
            }],
            "tool_choice": {"type": "auto"}
        });
        let req: AnthropicRequest = serde_json::from_value(raw).unwrap();
        let chat_req = convert_request(&req).unwrap();
        let tools = chat_req.tools.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].function.name, "get_weather");
        assert_eq!(chat_req.tool_choice.unwrap(), ToolChoice::auto());
    }

    #[test]
    fn tool_choice_any_maps_to_required() {
        let raw = serde_json::json!({
            "model": "claude",
            "messages": [{"role": "user", "content": "hi"}],
            "tool_choice": {"type": "any"}
        });
        let req: AnthropicRequest = serde_json::from_value(raw).unwrap();
        let chat_req = convert_request(&req).unwrap();
        assert_eq!(chat_req.tool_choice.unwrap(), ToolChoice::required());
    }

    #[test]
    fn tool_choice_tool_maps_to_function() {
        let raw = serde_json::json!({
            "model": "claude",
            "messages": [{"role": "user", "content": "hi"}],
            "tool_choice": {"type": "tool", "name": "get_weather"}
        });
        let req: AnthropicRequest = serde_json::from_value(raw).unwrap();
        let chat_req = convert_request(&req).unwrap();
        match chat_req.tool_choice.unwrap() {
            ToolChoice::Function { function, .. } => {
                assert_eq!(function.name, "get_weather");
            }
            _ => panic!("expected Function tool choice"),
        }
    }

    #[test]
    fn response_with_text_builds_correctly() {
        let chat_resp = ChatResponse {
            content: "Hello world".to_string(),
            model: "claude-sonnet".to_string(),
            usage: Some(Usage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
            }),
            tool_calls: None,
            finish_reason: Some(FinishReason::Stop),
            logprobs: None,
        };
        let resp = build_response(chat_resp, "msg_1");
        assert_eq!(resp.id, "msg_1");
        assert_eq!(resp.response_type, "message");
        assert_eq!(resp.role, "assistant");
        assert_eq!(resp.stop_reason.as_deref(), Some("end_turn"));
        assert_eq!(resp.usage.input_tokens, 10);
        assert_eq!(resp.usage.output_tokens, 5);
        assert_eq!(resp.content.len(), 1);
        match &resp.content[0] {
            AnthropicResponseBlock::Text { text } => assert_eq!(text, "Hello world"),
            _ => panic!("expected text block"),
        }
    }

    #[test]
    fn response_with_tool_use_builds_correctly() {
        let chat_resp = ChatResponse {
            content: String::new(),
            model: "claude-sonnet".to_string(),
            usage: Some(Usage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
            }),
            tool_calls: Some(vec![ToolCall {
                id: "toolu_1".to_string(),
                call_type: "function".to_string(),
                function: FunctionCall {
                    name: "get_weather".to_string(),
                    arguments: "{\"city\":\"SF\"}".to_string(),
                },
            }]),
            finish_reason: Some(FinishReason::ToolCalls),
            logprobs: None,
        };
        let resp = build_response(chat_resp, "msg_2");
        assert_eq!(resp.stop_reason.as_deref(), Some("tool_use"));
        assert_eq!(resp.content.len(), 1);
        match &resp.content[0] {
            AnthropicResponseBlock::ToolUse { id, name, input } => {
                assert_eq!(id, "toolu_1");
                assert_eq!(name, "get_weather");
                assert_eq!(input["city"], "SF");
            }
            _ => panic!("expected tool_use block"),
        }
    }

    #[test]
    fn response_with_text_and_tool_use() {
        let chat_resp = ChatResponse {
            content: "Let me check".to_string(),
            model: "claude-sonnet".to_string(),
            usage: None,
            tool_calls: Some(vec![ToolCall {
                id: "toolu_1".to_string(),
                call_type: "function".to_string(),
                function: FunctionCall {
                    name: "get_weather".to_string(),
                    arguments: "{}".to_string(),
                },
            }]),
            finish_reason: Some(FinishReason::ToolCalls),
            logprobs: None,
        };
        let resp = build_response(chat_resp, "msg_3");
        assert_eq!(resp.content.len(), 2);
        assert!(matches!(
            &resp.content[0],
            AnthropicResponseBlock::Text { .. }
        ));
        assert!(matches!(
            &resp.content[1],
            AnthropicResponseBlock::ToolUse { .. }
        ));
    }

    #[test]
    fn stop_reason_normalization() {
        assert_eq!(normalize_stop_reason(&FinishReason::Stop), "end_turn");
        assert_eq!(normalize_stop_reason(&FinishReason::EndTurn), "end_turn");
        assert_eq!(normalize_stop_reason(&FinishReason::ToolCalls), "tool_use");
        assert_eq!(normalize_stop_reason(&FinishReason::ToolUse), "tool_use");
        assert_eq!(normalize_stop_reason(&FinishReason::Length), "max_tokens");
    }

    #[test]
    fn error_response_serializes_correctly() {
        let err = AnthropicErrorBody {
            error_type: "error".to_string(),
            error: AnthropicErrorDetail {
                detail_type: "api_error".to_string(),
                message: "something went wrong".to_string(),
            },
        };
        let v = serde_json::to_value(&err).unwrap();
        assert_eq!(v["type"], "error");
        assert_eq!(v["error"]["type"], "api_error");
        assert_eq!(v["error"]["message"], "something went wrong");
    }

    #[tokio::test]
    async fn stream_state_emits_message_start_and_stop() {
        let empty_stream = Box::pin(futures::stream::empty::<Result<StreamChunk, LlmError>>());
        let mut state = AnthropicStreamState::new(
            empty_stream,
            "msg_test".to_string(),
            "test-model".to_string(),
        );

        // First chunk: text only
        let chunk1 = StreamChunk {
            delta: "Hello".to_string(),
            ..Default::default()
        };
        let events1 = state.process_chunk(chunk1);

        // Terminal chunk: more text + done
        let chunk2 = StreamChunk {
            delta: " world".to_string(),
            done: true,
            finish_reason: Some(FinishReason::Stop),
            ..Default::default()
        };
        let events2 = state.process_chunk(chunk2);

        let all_events: Vec<String> = events1.into_iter().chain(events2).collect();

        // Should have: message_start, content_block_start, content_block_delta,
        //               content_block_delta, content_block_stop, message_delta, message_stop
        let event_types: Vec<&str> = all_events
            .iter()
            .filter_map(|e| e.strip_prefix("event: ").and_then(|s| s.split('\n').next()))
            .collect();
        assert!(event_types.contains(&"message_start"));
        assert!(event_types.contains(&"content_block_start"));
        assert!(event_types.contains(&"content_block_delta"));
        assert!(event_types.contains(&"content_block_stop"));
        assert!(event_types.contains(&"message_delta"));
        assert!(event_types.contains(&"message_stop"));
    }

    #[tokio::test]
    async fn stream_state_handles_tool_calls() {
        let empty_stream = Box::pin(futures::stream::empty::<Result<StreamChunk, LlmError>>());
        let mut state =
            AnthropicStreamState::new(empty_stream, "msg_tc".to_string(), "test-model".to_string());

        // Terminal chunk with tool calls (no preceding text)
        let chunk = StreamChunk {
            delta: String::new(),
            done: true,
            finish_reason: Some(FinishReason::ToolCalls),
            tool_calls: Some(vec![ToolCall {
                id: "toolu_1".to_string(),
                call_type: "function".to_string(),
                function: FunctionCall {
                    name: "get_weather".to_string(),
                    arguments: "{\"city\":\"SF\"}".to_string(),
                },
            }]),
            ..Default::default()
        };

        let events = state.process_chunk(chunk);
        let event_types: Vec<&str> = events
            .iter()
            .filter_map(|e| e.strip_prefix("event: ").and_then(|s| s.split('\n').next()))
            .collect();

        // Should have message_start, then tool_use content blocks, message_delta, message_stop
        assert!(event_types.contains(&"message_start"));
        assert!(event_types.contains(&"content_block_start"));
        assert!(event_types.contains(&"message_delta"));
        assert!(event_types.contains(&"message_stop"));

        // Verify stop_reason is tool_use
        let delta_event = events
            .iter()
            .find(|e| e.starts_with("event: message_delta"))
            .unwrap();
        assert!(delta_event.contains("tool_use"));
    }

    #[tokio::test]
    async fn stream_rejects_model_no_slash() {
        let llm = std::sync::Arc::new(LmrsClient::new());
        let state = AppState { llm };
        let req = crate::ChatRequest::new("noslash", "hi");
        let resp = handle_stream(state, "noslash", req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn stream_rejects_empty_provider() {
        let llm = std::sync::Arc::new(LmrsClient::new());
        let state = AppState { llm };
        let req = crate::ChatRequest::new("/gpt", "hi");
        let resp = handle_stream(state, "/gpt", req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn stream_rejects_empty_model() {
        let llm = std::sync::Arc::new(LmrsClient::new());
        let state = AppState { llm };
        let req = crate::ChatRequest::new("openai/", "hi");
        let resp = handle_stream(state, "openai/", req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn non_stream_rejects_model_no_slash() {
        let llm = std::sync::Arc::new(LmrsClient::new());
        let state = AppState { llm };
        let req = crate::ChatRequest::new("noslash", "hi");
        let resp = handle_non_stream(state, "noslash", req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // ── block index tests ──────────────────────────────

    fn block_start_indices(events: &[String]) -> Vec<usize> {
        events
            .iter()
            .filter(|e| e.contains("\"content_block_start\""))
            .filter_map(|e| {
                e.split("\"index\":")
                    .nth(1)?
                    .split(',')
                    .next()?
                    .trim()
                    .parse()
                    .ok()
            })
            .collect()
    }

    #[test]
    fn tool_only_block_index_starts_at_zero() {
        let empty_stream = Box::pin(futures::stream::empty::<Result<StreamChunk, LlmError>>());
        let mut state = AnthropicStreamState::new(
            empty_stream,
            "msg_tool".to_string(),
            "test-model".to_string(),
        );
        let chunk = StreamChunk {
            delta: String::new(),
            done: true,
            finish_reason: Some(FinishReason::ToolCalls),
            tool_calls: Some(vec![ToolCall {
                id: "t1".to_string(),
                call_type: "function".to_string(),
                function: FunctionCall {
                    name: "f".to_string(),
                    arguments: "{}".to_string(),
                },
            }]),
            ..Default::default()
        };
        let events = state.process_chunk(chunk);
        assert_eq!(block_start_indices(&events), vec![0]);
    }

    #[test]
    fn text_then_tool_block_indices() {
        let empty_stream = Box::pin(futures::stream::empty::<Result<StreamChunk, LlmError>>());
        let mut state =
            AnthropicStreamState::new(empty_stream, "msg_tt".to_string(), "test-model".to_string());
        let e1 = state.process_chunk(StreamChunk {
            delta: "Hello".to_string(),
            ..Default::default()
        });
        let e2 = state.process_chunk(StreamChunk {
            delta: String::new(),
            done: true,
            finish_reason: Some(FinishReason::ToolCalls),
            tool_calls: Some(vec![ToolCall {
                id: "t1".to_string(),
                call_type: "function".to_string(),
                function: FunctionCall {
                    name: "f".to_string(),
                    arguments: "{}".to_string(),
                },
            }]),
            ..Default::default()
        });
        let mut events = e1;
        events.extend(e2);
        assert_eq!(block_start_indices(&events), vec![0, 1]);
    }

    #[test]
    fn text_tool_text_block_indices_no_reuse() {
        let empty_stream = Box::pin(futures::stream::empty::<Result<StreamChunk, LlmError>>());
        let mut state = AnthropicStreamState::new(
            empty_stream,
            "msg_ttt".to_string(),
            "test-model".to_string(),
        );
        let e1 = state.process_chunk(StreamChunk {
            delta: "A".to_string(),
            ..Default::default()
        });
        let e2 = state.process_chunk(StreamChunk {
            delta: String::new(),
            tool_calls: Some(vec![ToolCall {
                id: "t1".to_string(),
                call_type: "function".to_string(),
                function: FunctionCall {
                    name: "f".to_string(),
                    arguments: "{}".to_string(),
                },
            }]),
            ..Default::default()
        });
        let e3 = state.process_chunk(StreamChunk {
            delta: "B".to_string(),
            done: true,
            finish_reason: Some(FinishReason::Stop),
            ..Default::default()
        });
        let mut events = e1;
        events.extend(e2);
        events.extend(e3);
        assert_eq!(block_start_indices(&events), vec![0, 1, 2]);
    }

    #[test]
    fn two_tools_block_indices() {
        let empty_stream = Box::pin(futures::stream::empty::<Result<StreamChunk, LlmError>>());
        let mut state =
            AnthropicStreamState::new(empty_stream, "msg_2t".to_string(), "test-model".to_string());
        let chunk = StreamChunk {
            delta: String::new(),
            done: true,
            finish_reason: Some(FinishReason::ToolCalls),
            tool_calls: Some(vec![
                ToolCall {
                    id: "t1".to_string(),
                    call_type: "function".to_string(),
                    function: FunctionCall {
                        name: "f".to_string(),
                        arguments: "{}".to_string(),
                    },
                },
                ToolCall {
                    id: "t2".to_string(),
                    call_type: "function".to_string(),
                    function: FunctionCall {
                        name: "g".to_string(),
                        arguments: "[]".to_string(),
                    },
                },
            ]),
            ..Default::default()
        };
        let events = state.process_chunk(chunk);
        assert_eq!(block_start_indices(&events), vec![0, 1]);
    }

    #[test]
    fn same_chunk_text_and_tool_closes_text_before_tool_start() {
        let empty_stream = Box::pin(futures::stream::empty::<Result<StreamChunk, LlmError>>());
        let mut state = AnthropicStreamState::new(
            empty_stream,
            "msg_mix".to_string(),
            "test-model".to_string(),
        );
        let chunk = StreamChunk {
            delta: "text".to_string(),
            done: true,
            finish_reason: Some(FinishReason::ToolCalls),
            tool_calls: Some(vec![ToolCall {
                id: "t1".to_string(),
                call_type: "function".to_string(),
                function: FunctionCall {
                    name: "f".to_string(),
                    arguments: "{}".to_string(),
                },
            }]),
            ..Default::default()
        };
        let events = state.process_chunk(chunk);

        // Extract SSE event names and block indices from content_block_* events.
        let seq: Vec<(String, usize)> = events
            .iter()
            .filter(|e| {
                e.starts_with("event: content_block_start")
                    || e.starts_with("event: content_block_delta")
                    || e.starts_with("event: content_block_stop")
            })
            .map(|e| {
                let ev_type = e
                    .lines()
                    .next()
                    .unwrap()
                    .trim_start_matches("event: ")
                    .to_string();
                let idx_start = e.find("\"index\":").unwrap() + 8;
                let idx_end = e[idx_start..]
                    .find(|c: char| !c.is_ascii_digit())
                    .unwrap_or(e.len() - idx_start);
                let index: usize = e[idx_start..idx_start + idx_end].parse().unwrap();
                (ev_type, index)
            })
            .collect();

        // Text block must be fully closed (start→delta→stop) at index 0
        // before the tool block (start→delta→stop) at index 1.
        assert_eq!(
            seq,
            vec![
                ("content_block_start".to_string(), 0),
                ("content_block_delta".to_string(), 0),
                ("content_block_stop".to_string(), 0),
                ("content_block_start".to_string(), 1),
                ("content_block_delta".to_string(), 1),
                ("content_block_stop".to_string(), 1),
            ],
            "same-chunk text+tool must close text before starting tool"
        );
    }

    #[tokio::test]
    async fn stream_error_does_not_flush_normal_message_stop() {
        // inner stream: Ok chunk then Err — error must NOT be followed by
        // normal message_delta / message_stop from flush().
        let inner = futures::stream::iter([
            Ok(StreamChunk {
                delta: "hello".to_string(),
                ..Default::default()
            }),
            Err(LlmError::Stream("upstream disconnected".to_string())),
        ]);
        let resp = super::build_stream_response(
            Box::pin(inner),
            "msg_err".to_string(),
            "test-model".to_string(),
        );
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("body read");
        let text = String::from_utf8(body.to_vec()).expect("valid UTF-8");

        assert!(
            text.contains("event: error"),
            "must emit error event: {text}"
        );
        assert!(
            !text.contains("event: message_stop") || {
                // message_stop must not appear AFTER the error
                let err_pos = text.find("event: error").unwrap();
                !text[err_pos..].contains("event: message_stop")
            },
            "error event must not be followed by normal message_stop: {text}"
        );
    }
}
