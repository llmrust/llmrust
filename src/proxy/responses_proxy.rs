//! OpenAI Responses API protocol handler for the proxy server.
//!
//! Provides a `/v1/responses` endpoint that speaks OpenAI's native Responses
//! API wire protocol, allowing Responses-API clients (Codex CLI, SDKs that
//! target `POST /v1/responses`) to reach any registered llmrust backend
//! through automatic format conversion.
//!
//! The handler translates:
//! - request: `input`/`instructions`/`tools` (Responses shape) → `ChatRequest`
//! - response: `ChatResponse` / `StreamChunk` stream → Responses object / SSE

use std::convert::Infallible;

use axum::{
    extract::State,
    http::StatusCode,
    response::{
        sse::{Event, Sse},
        IntoResponse, Response,
    },
    Json,
};
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};

use crate::types::{
    ChatRequest, ChatResponse, Content, ContentPart, FunctionCall, Message, Role, StreamChunk,
    ThinkingConfig, Tool, ToolCall, ToolChoice, Usage,
};
use crate::LlmError;

use super::{generate_id, split_model, unix_timestamp, AppState};

// ── Responses API request types ──────────────

/// Role strings accepted by the Responses API.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ResponsesRole {
    #[default]
    User,
    Assistant,
    Developer,
    System,
}

impl ResponsesRole {
    fn to_llmrust(&self) -> Role {
        match self {
            ResponsesRole::User => Role::User,
            ResponsesRole::Assistant => Role::Assistant,
            ResponsesRole::Developer | ResponsesRole::System => Role::System,
        }
    }
}

/// A single content part inside a Responses input message.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponsesContentPart {
    #[serde(rename = "input_text")]
    InputText { text: String },
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "output_text")]
    OutputText { text: String },
    #[serde(rename = "input_image")]
    InputImage { image_url: ResponsesImageUrl },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: ResponsesImageUrl },
    #[serde(other)]
    Unknown,
}

/// Image reference inside a Responses content part.
#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
pub enum ResponsesImageUrl {
    Str(String),
    Obj { url: String },
}

/// A single input item in a Responses request's `input` array.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponsesInputItem {
    Message {
        #[serde(default)]
        role: ResponsesRole,
        #[serde(default)]
        content: Vec<ResponsesContentPart>,
    },
    #[serde(rename = "function_call")]
    FunctionCall {
        #[serde(default)]
        #[allow(dead_code)]
        id: Option<String>,
        #[serde(default)]
        call_id: Option<String>,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        arguments: Option<String>,
    },
    #[serde(rename = "function_call_output")]
    FunctionCallOutput {
        #[serde(default)]
        call_id: Option<String>,
        #[serde(default)]
        output: Option<serde_json::Value>,
    },
}

/// The `input` field of a Responses request: either a string or an array.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ResponsesInput {
    Text(String),
    Items(Vec<ResponsesInputItem>),
}

/// A Responses-shaped tool definition (flat `name`/`description`/`parameters`).
#[derive(Debug, Deserialize, Clone)]
pub struct ResponsesTool {
    #[serde(default)]
    #[serde(rename = "type")]
    #[allow(dead_code)]
    pub tool_type: Option<String>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub parameters: Option<serde_json::Value>,
}

/// A Responses API request (`POST /v1/responses`).
#[derive(Debug, Deserialize)]
pub struct ResponsesRequest {
    pub model: String,
    #[serde(default)]
    pub input: Option<ResponsesInput>,
    #[serde(default)]
    pub instructions: Option<String>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub max_output_tokens: Option<u64>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub top_p: Option<f64>,
    #[serde(default)]
    pub tools: Option<Vec<ResponsesTool>>,
    #[serde(default)]
    pub tool_choice: Option<serde_json::Value>,
    #[serde(default)]
    pub reasoning: Option<serde_json::Value>,
    #[serde(default)]
    pub store: Option<bool>,
    #[serde(default)]
    #[allow(dead_code)]
    pub include: Option<Vec<String>>,
    #[serde(default)]
    #[allow(dead_code)]
    pub parallel_tool_calls: Option<bool>,
    #[serde(default)]
    #[allow(dead_code)]
    pub text: Option<serde_json::Value>,
    #[serde(default)]
    #[allow(dead_code)]
    pub prompt_cache_key: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub previous_response_id: Option<String>,
}

// ── Responses API response types ─────────────

/// The Responses API response object (non-streaming).
#[derive(Debug, Serialize)]
pub struct ResponsesResponse {
    pub id: String,
    #[serde(rename = "object")]
    pub object: String,
    pub created_at: u64,
    pub status: String,
    pub model: String,
    pub output: Vec<ResponsesOutputItem>,
    pub usage: Option<ResponsesUsage>,
}

/// A single output item in the `output` array.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponsesOutputItem {
    Message {
        id: String,
        role: String,
        status: String,
        content: Vec<ResponsesOutputContent>,
    },
    #[serde(rename = "function_call")]
    FunctionCall {
        id: String,
        call_id: String,
        name: String,
        arguments: String,
        status: String,
    },
}

/// Output content part inside a message item.
#[derive(Debug, Serialize)]
pub struct ResponsesOutputContent {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: String,
    pub annotations: Vec<serde_json::Value>,
}

/// Responses usage object (subset of the OpenAI shape).
#[derive(Debug, Serialize)]
pub struct ResponsesUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens_details: Option<ResponsesUsageDetails>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens_details: Option<ResponsesOutputTokensDetails>,
}

/// Input token details (cached-token breakdown).
#[derive(Debug, Serialize)]
pub struct ResponsesUsageDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_tokens: Option<u64>,
}

/// Output token details (reasoning-token breakdown).
#[derive(Debug, Serialize)]
pub struct ResponsesOutputTokensDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
}

// ── Request conversion: Responses → ChatRequest ──

/// Convert a Responses request into an internal [`ChatRequest`].
fn convert_request(req: &ResponsesRequest) -> Result<ChatRequest, String> {
    let mut messages: Vec<Message> = Vec::new();

    if let Some(instructions) = &req.instructions {
        if !instructions.is_empty() {
            messages.push(Message::system(instructions.clone()));
        }
    }

    match &req.input {
        Some(ResponsesInput::Text(text)) => {
            if !text.is_empty() {
                messages.push(Message::user(text.clone()));
            }
        }
        Some(ResponsesInput::Items(items)) => {
            for item in items {
                match item {
                    ResponsesInputItem::Message { role, content } => {
                        let parts = content_parts_to_llmrust(content);
                        if parts.is_empty() {
                            continue;
                        }
                        let content = if parts.len() == 1 {
                            match &parts[0] {
                                ContentPart::Text { text } => Content::Text(text.clone()),
                                other => Content::Parts(vec![other.clone()]),
                            }
                        } else {
                            Content::Parts(parts)
                        };
                        messages.push(Message {
                            role: role.to_llmrust(),
                            content,
                            tool_calls: None,
                            tool_call_id: None,
                            name: None,
                        });
                    }
                    ResponsesInputItem::FunctionCall {
                        name,
                        arguments,
                        call_id,
                        ..
                    } => {
                        let call_id = call_id.clone().unwrap_or_default();
                        let name = name.clone().unwrap_or_default();
                        let arguments = arguments.clone().unwrap_or_default();
                        // Preserve the structured tool call (assistant turn with
                        // tool_calls) so multi-turn function calling stays real,
                        // instead of degrading it to fake text.
                        messages.push(Message {
                            role: Role::Assistant,
                            content: Content::Text(String::new()),
                            tool_calls: Some(vec![ToolCall {
                                id: call_id.clone(),
                                call_type: "function".to_string(),
                                function: FunctionCall { name, arguments },
                            }]),
                            tool_call_id: None,
                            name: None,
                        });
                    }
                    ResponsesInputItem::FunctionCallOutput { call_id, output } => {
                        let call_id = call_id.clone().unwrap_or_default();
                        let out = output
                            .as_ref()
                            .map(|v| {
                                if let Some(s) = v.as_str() {
                                    s.to_string()
                                } else {
                                    v.to_string()
                                }
                            })
                            .unwrap_or_default();
                        // Tool result message carries the call id (tool role).
                        messages.push(Message {
                            role: Role::Tool,
                            content: Content::Text(out),
                            tool_calls: None,
                            tool_call_id: Some(call_id),
                            name: None,
                        });
                    }
                }
            }
        }
        None => {}
    }

    if messages.is_empty() {
        messages.push(Message::user(""));
    }

    let mut chat = ChatRequest::from_messages(req.model.clone(), messages);
    chat.temperature = req.temperature;
    chat.top_p = req.top_p;
    chat.max_tokens = req.max_output_tokens;
    if req.store == Some(true) {
        chat.store = Some(true);
    }
    if let Some(tools) = &req.tools {
        let converted: Vec<Tool> = tools.iter().filter_map(responses_tool_to_llmrust).collect();
        if !converted.is_empty() {
            chat.tools = Some(converted);
        }
    }
    if let Some(choice) = &req.tool_choice {
        chat.tool_choice = Some(tool_choice_from_value(choice));
    }
    if let Some(reasoning) = &req.reasoning {
        if let Some(effort) = reasoning.get("effort").and_then(|v| v.as_str()) {
            if effort != "none" && effort != "disabled" {
                chat.thinking = Some(ThinkingConfig::Enabled {
                    budget_tokens: None,
                });
            }
        }
    }
    Ok(chat)
}

/// Convert Responses content parts to llmrust [`ContentPart`]s.
fn content_parts_to_llmrust(parts: &[ResponsesContentPart]) -> Vec<ContentPart> {
    let mut out = Vec::new();
    for part in parts {
        match part {
            ResponsesContentPart::InputText { text }
            | ResponsesContentPart::Text { text }
            | ResponsesContentPart::OutputText { text } => {
                if !text.is_empty() {
                    out.push(ContentPart::Text { text: text.clone() });
                }
            }
            ResponsesContentPart::InputImage { image_url }
            | ResponsesContentPart::ImageUrl { image_url } => {
                let url = match image_url {
                    ResponsesImageUrl::Str(s) => s.clone(),
                    ResponsesImageUrl::Obj { url } => url.clone(),
                };
                out.push(ContentPart::image_url(url));
            }
            ResponsesContentPart::Unknown => {
                tracing::warn!("responses proxy: skipping unknown content part type");
            }
        }
    }
    out
}

/// Convert a Responses-shaped tool to llmrust's nested [`Tool`] shape.
fn responses_tool_to_llmrust(tool: &ResponsesTool) -> Option<Tool> {
    if tool.name.is_empty() {
        return None;
    }
    let parameters = tool
        .parameters
        .clone()
        .unwrap_or_else(|| serde_json::json!({"type": "object", "properties": {}}));
    Some(Tool::function(
        tool.name.clone(),
        tool.description.clone(),
        parameters,
    ))
}

/// Parse a `tool_choice` value into llmrust [`ToolChoice`].
fn tool_choice_from_value(value: &serde_json::Value) -> ToolChoice {
    if let Some(s) = value.as_str() {
        return ToolChoice::Mode(s.to_string());
    }
    if let Some(obj) = value.as_object() {
        if let Some(fn_obj) = obj.get("function").and_then(|v| v.as_object()) {
            if let Some(name) = fn_obj.get("name").and_then(|v| v.as_str()) {
                return ToolChoice::Function {
                    choice_type: "function".to_string(),
                    function: crate::types::ToolChoiceFunction {
                        name: name.to_string(),
                    },
                };
            }
        }
    }
    ToolChoice::auto()
}

// ── Response building: ChatResponse → Responses object ──

/// Build a non-streaming Responses response object from a [`ChatResponse`].
fn build_response(resp: ChatResponse, id: &str, model: &str) -> ResponsesResponse {
    let mut output: Vec<ResponsesOutputItem> = Vec::new();

    if !resp.content.is_empty() {
        output.push(ResponsesOutputItem::Message {
            id: format!("msg_{id}"),
            role: "assistant".to_string(),
            status: "completed".to_string(),
            content: vec![ResponsesOutputContent {
                content_type: "output_text".to_string(),
                text: resp.content.clone(),
                annotations: Vec::new(),
            }],
        });
    }

    if let Some(tool_calls) = &resp.tool_calls {
        for tc in tool_calls {
            output.push(ResponsesOutputItem::FunctionCall {
                id: tc.id.clone(),
                call_id: tc.id.clone(),
                name: tc.function.name.clone(),
                arguments: tc.function.arguments.clone(),
                status: "completed".to_string(),
            });
        }
    }

    ResponsesResponse {
        id: id.to_string(),
        object: "response".to_string(),
        created_at: unix_timestamp(),
        status: "completed".to_string(),
        model: model.to_string(),
        output,
        usage: resp.usage.map(responses_usage_from_llmrust),
    }
}

/// Map llmrust [`Usage`] onto the Responses usage shape.
fn responses_usage_from_llmrust(u: Usage) -> ResponsesUsage {
    ResponsesUsage {
        input_tokens: u.prompt_tokens,
        output_tokens: u.completion_tokens,
        total_tokens: u.total_tokens,
        input_tokens_details: Some(ResponsesUsageDetails {
            cached_tokens: u.cache_read_tokens,
        }),
        output_tokens_details: Some(ResponsesOutputTokensDetails {
            reasoning_tokens: u.reasoning_tokens,
        }),
    }
}

// ── Streaming: StreamChunk stream → Responses SSE ──

/// Streaming state for the SSE unfold: carries the upstream stream plus a
/// FIFO queue of already-serialized SSE events to emit.
struct SseState {
    inner: futures::stream::BoxStream<'static, Result<StreamChunk, LlmError>>,
    id: String,
    model: String,
    msg_id: String,
    tool_item_ids: Vec<String>,
    tool_args_by_item: std::collections::HashMap<String, String>,
    tool_names_by_item: std::collections::HashMap<String, String>,
    full_text: String,
    terminal_usage: Option<Usage>,
    item_sent: bool,
    part_sent: bool,
    tool_items_sent: usize,
    done_seen: bool,
    awaiting_terminal: bool,
    terminated: bool,
    queue: std::collections::VecDeque<String>,
}

impl SseState {
    fn push(&mut self, payload: serde_json::Value) {
        self.queue.push_back(payload.to_string());
    }
}

/// Build a streaming Responses SSE response from an llmrust chunk stream.
///
/// Emits the Responses event sequence:
/// `response.created` → `response.output_item.added` →
/// `response.content_part.added` → `response.output_text.delta`* →
/// `response.output_item.done` → `response.completed` → `data: [DONE]`.
/// Tool calls surface as `response.output_item.added` (function_call) with
/// `response.function_call_arguments.delta` events.
fn build_stream_response(
    inner_stream: futures::stream::BoxStream<'static, Result<StreamChunk, LlmError>>,
    id: String,
    model: String,
) -> Response {
    let msg_id = format!("msg_{id}");
    let mut state = SseState {
        inner: inner_stream,
        id,
        model,
        msg_id,
        tool_item_ids: Vec::new(),
        tool_args_by_item: std::collections::HashMap::new(),
        tool_names_by_item: std::collections::HashMap::new(),
        full_text: String::new(),
        terminal_usage: None,
        item_sent: false,
        part_sent: false,
        tool_items_sent: 0,
        done_seen: false,
        awaiting_terminal: false,
        terminated: false,
        queue: std::collections::VecDeque::new(),
    };

    state.push(serde_json::json!({
        "type": "response.created",
        "response": {
            "id": state.id,
            "object": "response",
            "status": "in_progress",
            "model": state.model,
            "output": [],
        }
    }));

    let sse_stream = stream::unfold(state, |mut st| async move {
        loop {
            // Drain the queue first.
            if let Some(payload) = st.queue.pop_front() {
                return Some((Ok::<_, Infallible>(Event::default().data(payload)), st));
            }
            if st.terminated {
                return None;
            }

            // After `done` we keep polling to harvest the trailing usage
            // chunk that OpenAI-compatible providers emit (usage arrives on a
            // chunk *after* the terminal `done: true` chunk). Once the stream
            // ends, emit the final `response.completed` carrying the full
            // output array and the harvested usage.
            if st.awaiting_terminal {
                match st.inner.next().await {
                    Some(Ok(chunk)) => {
                        if let Some(u) = chunk.usage {
                            st.terminal_usage = Some(u);
                        }
                        // Drain any remaining tool-argument fragments too.
                        if let Some(tool_calls) = &chunk.tool_calls {
                            for tc in tool_calls {
                                if !tc.function.arguments.is_empty() {
                                    st.tool_args_by_item
                                        .entry(tc.id.clone())
                                        .and_modify(|a| a.push_str(&tc.function.arguments))
                                        .or_insert_with(|| tc.function.arguments.clone());
                                }
                            }
                        }
                        continue;
                    }
                    Some(Err(e)) => {
                        st.terminated = true;
                        st.push(serde_json::json!({
                            "type": "response.failed",
                            "response": {
                                "id": st.id, "object": "response", "status": "failed",
                                "model": st.model,
                                "error": {"message": e.to_string(), "code": "upstream_error"},
                            }
                        }));
                        continue;
                    }
                    None => {
                        st.terminated = true;
                        emit_completed(&mut st);
                        continue;
                    }
                }
            }

            match st.inner.next().await {
                Some(Ok(chunk)) => {
                    let has_tool = chunk.tool_calls.as_ref().is_some_and(|c| !c.is_empty());

                    // Accumulate text for the completed output snapshot.
                    if !chunk.delta.is_empty() {
                        st.full_text.push_str(&chunk.delta);
                    }

                    if !st.item_sent && (!chunk.delta.is_empty() || chunk.done) {
                        st.item_sent = true;
                        st.push(serde_json::json!({
                            "type": "response.output_item.added",
                            "output_index": 0,
                            "item": {
                                "id": st.msg_id, "type": "message", "role": "assistant",
                                "status": "in_progress", "content": [],
                            }
                        }));
                    }

                    if !st.part_sent && !chunk.delta.is_empty() {
                        st.part_sent = true;
                        st.push(serde_json::json!({
                            "type": "response.content_part.added",
                            "item_id": st.msg_id, "output_index": 0, "content_index": 0,
                            "part": {"type": "output_text", "text": "", "annotations": []},
                        }));
                    }

                    // Emit one function_call output item per tool call (not
                    // just the first), with distinct output_index.
                    if has_tool {
                        if let Some(tool_calls) = &chunk.tool_calls {
                            for tc in tool_calls {
                                let already_sent = st.tool_item_ids.contains(&tc.id);
                                let idx = if already_sent {
                                    // Real index of this tool among all tool
                                    // items (arguments deltas on later chunks
                                    // must keep pointing at the item).
                                    st.tool_item_ids
                                        .iter()
                                        .position(|x| x == &tc.id)
                                        .map(|p| p + 1)
                                        .unwrap_or(st.tool_items_sent + 1)
                                } else {
                                    st.tool_item_ids.push(tc.id.clone());
                                    st.tool_items_sent += 1;
                                    st.tool_items_sent
                                };
                                if !already_sent {
                                    st.push(serde_json::json!({
                                        "type": "response.output_item.added",
                                        "output_index": idx,
                                        "item": {
                                            "id": tc.id, "type": "function_call",
                                            "status": "in_progress", "name": tc.function.name,
                                            "call_id": tc.id, "arguments": "",
                                        }
                                    }));
                                }
                                // Remember the tool name on first sighting with
                                // a non-empty name (real streaming sends name on
                                // the first chunk; later chunks may omit it).
                                if !tc.function.name.is_empty() {
                                    st.tool_names_by_item
                                        .entry(tc.id.clone())
                                        .or_insert_with(|| tc.function.name.clone());
                                }
                                if !tc.function.arguments.is_empty() {
                                    st.tool_args_by_item
                                        .entry(tc.id.clone())
                                        .and_modify(|a| a.push_str(&tc.function.arguments))
                                        .or_insert_with(|| tc.function.arguments.clone());
                                    st.push(serde_json::json!({
                                        "type": "response.function_call_arguments.delta",
                                        "item_id": tc.id, "output_index": idx,
                                        "delta": tc.function.arguments,
                                    }));
                                }
                            }
                        }
                    }

                    if !chunk.delta.is_empty() {
                        st.push(serde_json::json!({
                            "type": "response.output_text.delta",
                            "item_id": st.msg_id, "output_index": 0, "content_index": 0,
                            "delta": chunk.delta,
                        }));
                    }

                    if chunk.done {
                        // Close the message item now; keep the stream open to
                        // harvest usage, then emit completed when the stream
                        // ends.
                        st.done_seen = true;
                        st.push(serde_json::json!({
                            "type": "response.output_item.done",
                            "output_index": 0,
                            "item": {
                                "id": st.msg_id, "type": "message", "role": "assistant",
                                "status": "completed",
                                "content": [{
                                    "type": "output_text",
                                    "text": st.full_text,
                                    "annotations": [],
                                }],
                            }
                        }));
                        let tool_ids: Vec<String> = st.tool_item_ids.clone();
                        for (i, tc_id) in tool_ids.iter().enumerate() {
                            let idx = i + 1;
                            let args = st.tool_args_by_item.get(tc_id).cloned().unwrap_or_default();
                            let name = st
                                .tool_names_by_item
                                .get(tc_id)
                                .cloned()
                                .unwrap_or_default();
                            st.push(serde_json::json!({
                                "type": "response.output_item.done",
                                "output_index": idx,
                                "item": {
                                    "id": tc_id, "type": "function_call",
                                    "status": "completed", "name": name,
                                    "call_id": tc_id, "arguments": args,
                                }
                            }));
                        }
                        if let Some(u) = chunk.usage {
                            st.terminal_usage = Some(u);
                        }
                        st.awaiting_terminal = true;
                        continue;
                    }

                    // Non-terminal chunk with no events to emit: keep polling.
                    continue;
                }
                Some(Err(e)) => {
                    st.terminated = true;
                    st.push(serde_json::json!({
                        "type": "response.failed",
                        "response": {
                            "id": st.id, "object": "response", "status": "failed",
                            "model": st.model,
                            "error": {"message": e.to_string(), "code": "upstream_error"},
                        }
                    }));
                    continue;
                }
                None => {
                    st.terminated = true;
                    emit_completed(&mut st);
                    continue;
                }
            }
        }
    });

    let done: futures::stream::BoxStream<'static, Result<Event, Infallible>> =
        Box::pin(stream::once(async {
            std::result::Result::<_, Infallible>::Ok(Event::default().data("[DONE]"))
        }));
    let sse_stream: futures::stream::BoxStream<'static, Result<Event, Infallible>> =
        Box::pin(sse_stream);
    Sse::new(sse_stream.chain(done))
        .keep_alive(
            axum::response::sse::KeepAlive::new()
                .interval(std::time::Duration::from_secs(15))
                .text("keep-alive"),
        )
        .into_response()
}

/// Emit the terminal `response.completed` event with the full output array
/// (message content plus any function calls) and the harvested usage.
fn emit_completed(st: &mut SseState) {
    let mut output: Vec<serde_json::Value> = Vec::new();
    output.push(serde_json::json!({
        "id": st.msg_id,
        "type": "message",
        "role": "assistant",
        "status": "completed",
        "content": [{
            "type": "output_text",
            "text": st.full_text,
            "annotations": [],
        }],
    }));
    for tc_id in st.tool_item_ids.clone() {
        let args = st
            .tool_args_by_item
            .get(&tc_id)
            .cloned()
            .unwrap_or_default();
        let name = st
            .tool_names_by_item
            .get(&tc_id)
            .cloned()
            .unwrap_or_default();
        output.push(serde_json::json!({
            "id": tc_id,
            "type": "function_call",
            "status": "completed",
            "call_id": tc_id,
            "name": name,
            "arguments": args,
        }));
    }
    let usage = st.terminal_usage.take().map(|u| {
        serde_json::json!({
            "input_tokens": u.prompt_tokens,
            "output_tokens": u.completion_tokens,
            "total_tokens": u.total_tokens,
            "input_tokens_details": {
                "cached_tokens": u.cache_read_tokens.unwrap_or(0),
            },
            "output_tokens_details": {
                "reasoning_tokens": u.reasoning_tokens.unwrap_or(0),
            },
        })
    });
    st.push(serde_json::json!({
        "type": "response.completed",
        "response": {
            "id": st.id,
            "object": "response",
            "status": "completed",
            "model": st.model,
            "output": output,
            "usage": usage,
        }
    }));
}

// ── Handler ──────────────────────────────

/// Handle a `POST /v1/responses` request.
pub async fn handle_responses(State(state): State<AppState>, body: String) -> Response {
    let raw: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("responses proxy: request JSON extraction failed");
            return invalid_json_response(
                StatusCode::BAD_REQUEST,
                &format!("Invalid JSON request body: {e}"),
            );
        }
    };

    let req: ResponsesRequest = match serde_json::from_value(raw) {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("responses proxy: request deserialization failed");
            return invalid_json_response(
                StatusCode::BAD_REQUEST,
                &format!("Invalid Responses request: {e}"),
            );
        }
    };

    let chat = match convert_request(&req) {
        Ok(c) => c,
        Err(e) => {
            return invalid_json_response(StatusCode::BAD_REQUEST, &e);
        }
    };

    if req.stream {
        handle_stream(state, req.model.as_str(), chat).await
    } else {
        handle_non_stream(state, req.model.as_str(), chat).await
    }
}

/// Non-streaming branch: `ChatResponse` → Responses response object.
async fn handle_non_stream(state: AppState, model: &str, req: ChatRequest) -> Response {
    let (_, model_name) = match split_model(model) {
        Ok(pair) => pair,
        Err(e) => {
            return invalid_json_response(StatusCode::BAD_REQUEST, e);
        }
    };
    let id = generate_id();
    match state.llm.chat_with(model, req).await {
        Ok(resp) => {
            let responses = build_response(resp, &id, model_name);
            Json(responses).into_response()
        }
        Err(e) => responses_error_from_llm_error(e),
    }
}

/// Streaming branch: `StreamChunk` stream → Responses SSE.
async fn handle_stream(state: AppState, model: &str, mut req: ChatRequest) -> Response {
    let (provider_name, model_name) = match split_model(model) {
        Ok(pair) => pair,
        Err(e) => {
            return invalid_json_response(StatusCode::BAD_REQUEST, e);
        }
    };

    req.model = model_name.to_string();
    req.stream = true;

    let provider = match state.llm.get_provider(provider_name).await {
        Ok(p) => p,
        Err(e) => return responses_error_from_llm_error(e),
    };

    let id = generate_id();
    match provider.stream(&req).await {
        Ok(inner_stream) => build_stream_response(inner_stream, id, model_name.to_string()),
        Err(e) => responses_error_from_llm_error(e),
    }
}

// ── Error helpers ─────────────────────────

fn invalid_json_response(status: StatusCode, message: &str) -> Response {
    let payload = serde_json::json!({
        "type": "error",
        "error": {"message": message, "type": "invalid_request_error"},
    });
    (status, Json(payload)).into_response()
}

/// Map an llmrust error onto a Responses-shaped error body.
fn responses_error_from_llm_error(e: LlmError) -> Response {
    let (status, err_type) = match &e {
        LlmError::Api { status, .. } => (
            StatusCode::from_u16(*status).unwrap_or(StatusCode::BAD_GATEWAY),
            "api_error",
        ),
        LlmError::UnknownProvider(_) => (StatusCode::BAD_REQUEST, "invalid_request_error"),
        _ => (StatusCode::BAD_GATEWAY, "upstream_error"),
    };
    let payload = serde_json::json!({
        "type": "error",
        "error": {"message": e.to_string(), "type": err_type},
    });
    (status, Json(payload)).into_response()
}
