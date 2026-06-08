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
    ChatRequest, ChatResponse, Content, FunctionCall, Message, ResponseFormat, StreamChunk, Tool,
    ToolCall, ToolChoice, Usage,
};

// ── Defaults ────────────────────────────────

const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

// ── Shared request / response types ─────────────────────

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
    #[serde(default)]
    tool_calls: Option<Vec<CompToolCallDelta>>,
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

/// Accumulates streamed tool-call fragments across SSE lines. OpenAI-style
/// streams deliver tool calls as a series of deltas keyed by `index`: the
/// first delta for an index carries `id` and `name`, later deltas append
/// `arguments` text. [`ToolCallAccumulator::ingest`] folds each delta in and
/// [`ToolCallAccumulator::take`] drains the completed calls on the terminal
/// chunk.
#[derive(Default)]
struct ToolCallAccumulator {
    builders: Vec<ToolCallBuilder>,
}

#[derive(Default)]
struct ToolCallBuilder {
    id: String,
    name: String,
    arguments: String,
}

impl ToolCallAccumulator {
    fn ingest(&mut self, deltas: &[CompToolCallDelta]) {
        for delta in deltas {
            if self.builders.len() <= delta.index {
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

// ── HTTP client construction ──────────────────────

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
            if let Some(deltas) = &choice.delta.tool_calls {
                tools.ingest(deltas);
            }
            let finish_reason = choice.finish_reason.clone();
            let tool_calls = if finish_reason.is_some() {
                tools.take()
            } else {
                None
            };
            vec![Ok(StreamChunk {
                delta: choice.delta.content.clone().unwrap_or_default(),
                done: finish_reason.is_some(),
                finish_reason,
                usage,
                tool_calls,
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

// ── The unified OpenAI-compatible provider ────────────────

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
            logprobs: None,
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
            response_format: req.response_format.as_ref(),
            stop: req.stop.as_deref(),
            n: req.n,
            seed: req.seed,
            presence_penalty: req.presence_penalty,
            frequency_penalty: req.frequency_penalty,
            logprobs: req.logprobs,
            top_logprobs: req.top_logprobs,
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
            response_format: req.response_format.as_ref(),
            stop: req.stop.as_deref(),
            n: req.n,
            seed: req.seed,
            presence_penalty: req.presence_penalty,
            frequency_penalty: req.frequency_penalty,
            logprobs: req.logprobs,
            top_logprobs: req.top_logprobs,
        };

        let resp = self.send(&body).await?;

        if !resp.status().is_success() {
            return Err(Self::parse_error(resp).await);
        }

        let byte_stream = resp
            .bytes_stream()
            .map(|r| r.map_err(|e| LlmError::Stream(e.to_string())));

        let stream = line_stream(byte_stream)
            .scan(ToolCallAccumulator::default(), |tools, line_result| {
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

    #[test]
    fn serializes_sampling_params_and_response_format() {
        let messages = vec![CompMessage::from(&Message::user("hi"))];
        let rf = ResponseFormat::json_object();
        let stop = vec!["STOP".to_string()];
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
        assert_eq!(chunk.finish_reason.as_deref(), Some("tool_calls"));
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
}
