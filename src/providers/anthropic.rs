//! Anthropic Claude API provider.

use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::providers::stream_util::line_stream;
use crate::providers::{LlmError, Provider, ProviderConfig, Result};
use crate::types::{
    ChatRequest, ChatResponse, Content, ContentPart, FunctionCall, StreamChunk, Tool, ToolCall,
    ToolChoice, Usage,
};

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com/v1";

/// Build an HTTP client with explicit request/connect timeouts so a stalled
/// connection can never block a caller indefinitely. Falls back to the default
/// client if the builder fails (it will not in practice).
fn build_http_client() -> Client {
    Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_else(|_| Client::new())
}

pub struct AnthropicProvider {
    client: Client,
    api_key: String,
    base_url: String,
}

impl AnthropicProvider {
    pub fn new(config: ProviderConfig) -> Self {
        Self {
            client: build_http_client(),
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
    tools: Option<Vec<AnthropicTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<AnthropicToolChoice>,
    stream: bool,
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
/// to the cross-provider `tool_calls` value so callers can branch uniformly.
fn normalize_stop_reason(reason: String) -> String {
    if reason == "tool_use" {
        "tool_calls".to_string()
    } else {
        reason
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
                system = Some(msg.content.as_text());
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
    input_tokens: u64,
    output_tokens: u64,
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
    delta: Option<AnthropicDelta>,
}

#[derive(Deserialize)]
struct AnthropicDelta {
    text: Option<String>,
    stop_reason: Option<String>,
}

/// Parse a single SSE line from an Anthropic stream into zero or more
/// [`StreamChunk`]s. Lines are guaranteed complete by [`line_stream`].
///
/// Anthropic delivers the stop reason on the `message_delta` event rather than
/// `message_stop`, so completion is keyed off that event's `stop_reason`.
/// Streaming surfaces text deltas only; reconstructing tool calls from the
/// stream is not yet supported, so callers that need tool calls should use the
/// non-streaming `chat` path.
fn parse_sse_line(line: &str) -> Vec<Result<StreamChunk>> {
    let line = line.trim();
    let Some(data) = line.strip_prefix("data: ") else {
        return Vec::new();
    };
    let Ok(event) = serde_json::from_str::<AnthropicStreamEvent>(data) else {
        return Vec::new();
    };
    match event.event_type.as_str() {
        "content_block_delta" => {
            let text = event.delta.and_then(|d| d.text).unwrap_or_default();
            vec![Ok(StreamChunk {
                delta: text,
                ..Default::default()
            })]
        }
        "message_delta" => {
            let finish_reason = event
                .delta
                .and_then(|d| d.stop_reason)
                .map(normalize_stop_reason);
            vec![Ok(StreamChunk {
                done: finish_reason.is_some(),
                finish_reason,
                ..Default::default()
            })]
        }
        _ => Vec::new(),
    }
}

impl AnthropicProvider {
    /// Build the Claude request body shared by `chat` and `stream`.
    fn build_body<'a>(&self, req: &'a ChatRequest, stream: bool) -> AnthropicRequest<'a> {
        let (system, messages) = split_messages(req);
        AnthropicRequest {
            model: &req.model,
            messages,
            system,
            temperature: req.temperature,
            max_tokens: req.max_tokens.or(Some(4096)), // Anthropic requires max_tokens
            top_p: req.top_p,
            tools: req.tools.as_deref().map(to_anthropic_tools),
            tool_choice: req.tool_choice.as_ref().map(to_anthropic_tool_choice),
            stream,
        }
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse> {
        let body = self.build_body(req, false);

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
            return Err(LlmError::Api {
                status: status.as_u16(),
                message: msg,
            });
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

        Ok(ChatResponse {
            content,
            model: parsed.model,
            usage: parsed.usage.map(|u| Usage {
                prompt_tokens: u.input_tokens,
                completion_tokens: u.output_tokens,
                total_tokens: u.input_tokens.saturating_add(u.output_tokens),
            }),
            tool_calls,
            finish_reason: parsed.stop_reason.map(normalize_stop_reason),
        })
    }

    async fn stream(&self, req: &ChatRequest) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        let body = self.build_body(req, true);

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
            return Err(LlmError::Api {
                status: status.as_u16(),
                message: msg,
            });
        }

        let byte_stream = resp
            .bytes_stream()
            .map(|r| r.map_err(|e| LlmError::Stream(e.to_string())));

        let stream = line_stream(byte_stream).flat_map(|line_result| {
            let chunks = match line_result {
                Ok(line) => parse_sse_line(&line),
                Err(e) => vec![Err(e)],
            };
            futures::stream::iter(chunks)
        });

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
            "tool_calls"
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
}
