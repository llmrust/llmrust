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

use crate::providers::stream_util::line_stream;
use crate::providers::{LlmError, Provider, ProviderConfig, Result};
use crate::types::{
    ChatRequest, ChatResponse, Content, Message, StreamChunk, Tool, ToolCall, ToolChoice, Usage,
};

// ── Defaults ────────────────────────────────────────────

const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

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
}

#[derive(Deserialize)]
struct CompUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
}

#[derive(Deserialize)]
struct CompStreamChunk {
    #[serde(default)]
    choices: Vec<CompStreamChoice>,
    #[serde(default)]
    usage: Option<CompUsage>,
}

#[derive(Deserialize)]
struct CompStreamChoice {
    delta: CompDelta,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct CompDelta {
    content: Option<String>,
}

#[derive(Deserialize)]
struct CompErrorBody {
    error: CompErrorDetail,
}

#[derive(Deserialize)]
struct CompErrorDetail {
    message: String,
}

// ── HTTP client construction ──────────────────────────

fn build_http_client() -> Client {
    Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .pool_max_idle_per_host(32)
        .tcp_keepalive(Duration::from_secs(30))
        .build()
        .expect("reqwest::Client::builder() with valid options")
}

/// Parse a single SSE line from an OpenAI-compatible stream into zero or more
/// [`StreamChunk`]s. Lines are guaranteed complete by [`line_stream`], so a
/// failed JSON parse here indicates a genuinely malformed payload rather than
/// a chunk-boundary artifact.
///
/// A streamed response ends with a chunk carrying a `finish_reason`, optionally
/// followed by a choices-less chunk carrying only `usage` (when
/// `stream_options.include_usage` was requested), then a literal `[DONE]`.
fn parse_sse_line(line: &str) -> Vec<Result<StreamChunk>> {
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
    let Ok(parsed) = serde_json::from_str::<CompStreamChunk>(data) else {
        return Vec::new();
    };
    let usage = parsed.usage.map(|u| Usage {
        prompt_tokens: u.prompt_tokens,
        completion_tokens: u.completion_tokens,
        total_tokens: u.total_tokens,
    });
    match parsed.choices.first() {
        Some(choice) => {
            let finish_reason = choice.finish_reason.clone();
            vec![Ok(StreamChunk {
                delta: choice.delta.content.clone().unwrap_or_default(),
                done: finish_reason.is_some(),
                finish_reason,
                usage,
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

// ── The unified OpenAI-compatible provider ────────────────────

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
}

impl OpenAiCompatibleProvider {
    /// Create a new provider. Extra headers are appended after the standard
    /// `Authorization` header — they can override it if needed.
    pub fn new(
        config: ProviderConfig,
        extra_headers: impl IntoIterator<Item = (String, String)>,
    ) -> Self {
        let base_url = config.base_url.unwrap_or_default();
        Self {
            client: build_http_client(),
            api_key: config.api_key,
            base_url,
            extra_headers: extra_headers.into_iter().collect(),
        }
    }

    /// Send a fully-built request body and return the raw HTTP response.
    async fn send(&self, body: &CompChatRequest<'_>) -> Result<reqwest::Response> {
        let mut rb = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key));

        for (k, v) in &self.extra_headers {
            rb = rb.header(k.as_str(), v.as_str());
        }

        let resp = rb.json(body).send().await?;
        Ok(resp)
    }

    /// Common HTTP error -> LlmError conversion.
    async fn parse_error(resp: reqwest::Response) -> LlmError {
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        let msg = serde_json::from_str::<CompErrorBody>(&text)
            .map(|e| e.error.message)
            .unwrap_or(text);
        LlmError::Api {
            status,
            message: msg,
        }
    }

    /// Parse a non-streaming response into [`ChatResponse`].
    fn parse_response(parsed: CompResponse) -> ChatResponse {
        let usage = parsed.usage.map(|u| Usage {
            prompt_tokens: u.prompt_tokens,
            completion_tokens: u.completion_tokens,
            total_tokens: u.total_tokens,
        });

        let (content, tool_calls, finish_reason) = match parsed.choices.into_iter().next() {
            Some(choice) => {
                let content = match choice.message.content {
                    Some(c) => c.as_text(),
                    None => String::new(),
                };
                (content, choice.message.tool_calls, choice.finish_reason)
            }
            None => (String::new(), None, None),
        };

        ChatResponse {
            content,
            model: parsed.model,
            usage,
            tool_calls,
            finish_reason,
        }
    }
}

#[async_trait]
impl Provider for OpenAiCompatibleProvider {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse> {
        let messages: Vec<CompMessage> = req.messages.iter().map(CompMessage::from).collect();

        let body = CompChatRequest {
            model: &req.model,
            messages: &messages,
            temperature: req.temperature,
            max_tokens: req.max_tokens,
            top_p: req.top_p,
            stream: false,
            stream_options: None,
            tools: req.tools.as_deref(),
            tool_choice: req.tool_choice.as_ref(),
        };

        let resp = self.send(&body).await?;

        if !resp.status().is_success() {
            return Err(Self::parse_error(resp).await);
        }

        let parsed: CompResponse = resp
            .json()
            .await
            .map_err(|e| LlmError::Parse(format!("OpenAI-compatible parse: {e}")))?;

        Ok(Self::parse_response(parsed))
    }

    async fn stream(&self, req: &ChatRequest) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        let messages: Vec<CompMessage> = req.messages.iter().map(CompMessage::from).collect();

        let body = CompChatRequest {
            model: &req.model,
            messages: &messages,
            temperature: req.temperature,
            max_tokens: req.max_tokens,
            top_p: req.top_p,
            stream: true,
            stream_options: Some(StreamOptions {
                include_usage: true,
            }),
            tools: req.tools.as_deref(),
            tool_choice: req.tool_choice.as_ref(),
        };

        let resp = self.send(&body).await?;

        if !resp.status().is_success() {
            return Err(Self::parse_error(resp).await);
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
        assert_eq!(resp.finish_reason.as_deref(), Some("tool_calls"));
        let calls = resp.tool_calls.expect("tool_calls present");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "get_weather");
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
}
