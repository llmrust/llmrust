//! Shared OpenAI-compatible chat completions implementation.
//!
//! Many providers (OpenAI, DeepSeek, Moonshot, OpenRouter, ...) speak the same
//! `/chat/completions` wire format. This module centralizes request building,
//! response parsing, and SSE stream handling so each provider only needs to
//! supply its endpoint, auth, and model defaults.

use futures::{stream::BoxStream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::providers::stream_util::line_stream;
use crate::providers::{LlmError, Result};
use crate::types::{
    ChatRequest, ChatResponse, Content, FunctionCall, Message, ResponseFormat, StreamChunk, Tool,
    ToolCall, ToolChoice, Usage,
};

// --- Request types ---

#[derive(Serialize)]
struct CompRequest<'a> {
    model: &'a str,
    messages: &'a [Message],
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frequency_penalty: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    presence_penalty: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop: Option<&'a [String]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    n: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    logprobs: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_logprobs: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<&'a [Tool]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'a ToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<&'a ResponseFormat>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    stream: bool,
}

impl<'a> CompRequest<'a> {
    fn from_chat(req: &'a ChatRequest, stream: bool) -> Self {
        Self {
            model: &req.model,
            messages: &req.messages,
            temperature: req.temperature,
            max_tokens: req.max_tokens,
            top_p: req.top_p,
            frequency_penalty: req.frequency_penalty,
            presence_penalty: req.presence_penalty,
            stop: req.stop.as_deref(),
            seed: req.seed,
            n: req.n,
            logprobs: req.logprobs,
            top_logprobs: req.top_logprobs,
            tools: req.tools.as_deref(),
            tool_choice: req.tool_choice.as_ref(),
            response_format: req.response_format.as_ref(),
            stream,
        }
    }
}

// --- Response types ---

#[derive(Deserialize)]
struct CompResponse {
    choices: Vec<CompChoice>,
    #[serde(default)]
    model: String,
    #[serde(default)]
    usage: Option<CompUsage>,
}

#[derive(Deserialize)]
struct CompChoice {
    message: CompMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct CompMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Deserialize)]
struct CompUsage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
    #[serde(default)]
    total_tokens: u64,
}

#[derive(Deserialize)]
struct CompErrorBody {
    error: CompErrorDetail,
}

#[derive(Deserialize)]
struct CompErrorDetail {
    message: String,
}

// --- Stream types ---

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
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct CompDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<CompToolCallDelta>>,
}

/// A streamed tool-call fragment. OpenAI streams tool calls incrementally:
/// the first fragment for a given `index` carries the `id` and function
/// `name`, and subsequent fragments append `arguments` characters. Fragments
/// are correlated across chunks by their `index`.
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

/// Accumulates streamed tool-call fragments into complete [`ToolCall`]s.
/// Builders are stored positionally by their stream `index`; the vector is
/// grown as new indices appear so fragments can arrive in any order.
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
            if delta.index >= self.builders.len() {
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
        let calls: Vec<ToolCall> = std::mem::take(&mut self.builders)
            .into_iter()
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

/// Parse one SSE line from an OpenAI-compatible stream into zero or more
/// [`StreamChunk`]s, threading a [`ToolCallAccumulator`] so streamed tool calls
/// can be reconstructed across chunks. Tool calls are surfaced on the terminal
/// chunk (the one carrying `finish_reason`). Lines are guaranteed complete by
/// [`line_stream`].
fn parse_sse_line(tools: &mut ToolCallAccumulator, line: &str) -> Vec<Result<StreamChunk>> {
    let line = line.trim();
    let Some(data) = line.strip_prefix("data: ") else {
        return Vec::new();
    };
    if data == "[DONE]" {
        return Vec::new();
    }
    let Ok(chunk) = serde_json::from_str::<CompStreamChunk>(data) else {
        return Vec::new();
    };
    let usage = chunk.usage.map(|u| Usage {
        prompt_tokens: u.prompt_tokens,
        completion_tokens: u.completion_tokens,
        total_tokens: u.total_tokens,
    });
    match chunk.choices.into_iter().next() {
        Some(choice) => {
            if let Some(deltas) = &choice.delta.tool_calls {
                tools.ingest(deltas);
            }
            let tool_calls = if choice.finish_reason.is_some() {
                tools.take()
            } else {
                None
            };
            vec![Ok(StreamChunk {
                delta: choice.delta.content.unwrap_or_default(),
                done: choice.finish_reason.is_some(),
                finish_reason: choice.finish_reason,
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

/// Issue a non-streaming chat completion against an OpenAI-compatible endpoint.
pub async fn chat(
    client: &Client,
    url: &str,
    api_key: &str,
    req: &ChatRequest,
) -> Result<ChatResponse> {
    let body = CompRequest::from_chat(req, false);
    let resp = client
        .post(url)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        let msg = serde_json::from_str::<CompErrorBody>(&text)
            .map(|e| e.error.message)
            .unwrap_or(text);
        return Err(LlmError::Api {
            status: status.as_u16(),
            message: msg,
        });
    }

    let parsed: CompResponse = resp
        .json()
        .await
        .map_err(|e| LlmError::Parse(e.to_string()))?;

    let choice = parsed
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| LlmError::Parse("no choices in response".to_string()))?;

    Ok(ChatResponse {
        content: choice.message.content.unwrap_or_default(),
        model: parsed.model,
        usage: parsed.usage.map(|u| Usage {
            prompt_tokens: u.prompt_tokens,
            completion_tokens: u.completion_tokens,
            total_tokens: u.total_tokens,
        }),
        tool_calls: choice.message.tool_calls,
        finish_reason: choice.finish_reason,
    })
}

/// Issue a streaming chat completion against an OpenAI-compatible endpoint.
pub async fn stream(
    client: &Client,
    url: &str,
    api_key: &str,
    req: &ChatRequest,
) -> Result<BoxStream<'static, Result<StreamChunk>>> {
    let body = CompRequest::from_chat(req, true);
    let resp = client
        .post(url)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        let msg = serde_json::from_str::<CompErrorBody>(&text)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ContentPart, FunctionCall};

    #[test]
    fn serializes_tools_and_tool_choice() {
        let mut req = ChatRequest::from_messages("gpt-4o", vec![Message::user("hi")]);
        req.tools = Some(vec![Tool::function(
            "get_weather",
            Some("Get weather".to_string()),
            serde_json::json!({"type": "object"}),
        )]);
        req.tool_choice = Some(ToolChoice::auto());
        let body = CompRequest::from_chat(&req, false);
        let v = serde_json::to_value(&body).unwrap();
        assert_eq!(v["tools"][0]["function"]["name"], "get_weather");
        assert_eq!(v["tool_choice"], "auto");
    }

    #[test]
    fn parses_tool_calls_from_response() {
        let raw = serde_json::json!({
            "choices": [{
                "message": {
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {"name": "get_weather", "arguments": "{\"city\":\"SF\"}"}
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "model": "gpt-4o"
        })
        .to_string();
        let parsed: CompResponse = serde_json::from_str(&raw).unwrap();
        let choice = parsed.choices.into_iter().next().unwrap();
        let calls = choice.message.tool_calls.unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].function.name, "get_weather");
    }

    #[test]
    fn tool_message_serializes_with_id() {
        let messages = vec![Message::tool("call_1", "sunny")];
        let v = serde_json::to_value(&messages).unwrap();
        assert_eq!(v[0]["role"], "tool");
        assert_eq!(v[0]["tool_call_id"], "call_1");
        assert_eq!(v[0]["content"], "sunny");
    }

    #[test]
    fn assistant_tool_call_message_omits_empty_content() {
        let messages = vec![Message::assistant_tool_calls(vec![ToolCall {
            id: "call_1".to_string(),
            call_type: "function".to_string(),
            function: FunctionCall {
                name: "f".to_string(),
                arguments: "{}".to_string(),
            },
        }])];
        let v = serde_json::to_value(&messages).unwrap();
        assert_eq!(v[0]["role"], "assistant");
        assert!(v[0].get("content").is_none() || v[0]["content"].is_null());
        assert_eq!(v[0]["tool_calls"][0]["id"], "call_1");
    }

    #[test]
    fn user_image_message_serializes_as_content_parts() {
        let messages = vec![Message::user_parts(vec![
            ContentPart::text("describe"),
            ContentPart::image_url("https://example.com/cat.png"),
        ])];
        let v = serde_json::to_value(&messages).unwrap();
        assert_eq!(v[0]["content"][0]["type"], "text");
        assert_eq!(v[0]["content"][1]["type"], "image_url");
        assert_eq!(
            v[0]["content"][1]["image_url"]["url"],
            "https://example.com/cat.png"
        );
    }

    #[test]
    fn serializes_sampling_params_and_response_format() {
        let mut req = ChatRequest::from_messages("gpt-4o", vec![Message::user("hi")]);
        req.temperature = Some(0.5);
        req.max_tokens = Some(256);
        req.top_p = Some(0.9);
        req.frequency_penalty = Some(0.1);
        req.presence_penalty = Some(0.2);
        req.stop = Some(vec!["\n".to_string()]);
        req.seed = Some(42);
        req.n = Some(2);
        req.response_format = Some(ResponseFormat::json_object());
        let body = CompRequest::from_chat(&req, false);
        let v = serde_json::to_value(&body).unwrap();
        assert_eq!(v["temperature"], 0.5);
        assert_eq!(v["max_tokens"], 256);
        assert_eq!(v["top_p"], 0.9);
        assert_eq!(v["frequency_penalty"], 0.1);
        assert_eq!(v["presence_penalty"], 0.2);
        assert_eq!(v["stop"][0], "\n");
        assert_eq!(v["seed"], 42);
        assert_eq!(v["n"], 2);
        assert_eq!(v["response_format"]["type"], "json_object");
    }

    #[test]
    fn omits_unset_sampling_params() {
        let req = ChatRequest::from_messages("gpt-4o", vec![Message::user("hi")]);
        let body = CompRequest::from_chat(&req, false);
        let v = serde_json::to_value(&body).unwrap();
        assert!(v.get("temperature").is_none());
        assert!(v.get("stop").is_none());
        assert!(v.get("seed").is_none());
        assert!(v.get("response_format").is_none());
        assert!(v.get("stream").is_none());
    }

    #[test]
    fn stream_accumulates_tool_call_fragments() {
        let mut tools = ToolCallAccumulator::default();
        let lines = [
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"get_weather","arguments":""}}]},"finish_reason":null}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"city\":"}}]},"finish_reason":null}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"SF\"}"}}]},"finish_reason":null}]}"#,
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
        let mut chunks = Vec::new();
        for line in [
            r#"data: {"choices":[{"delta":{"content":"Hello"},"finish_reason":null}]}"#,
            r#"data: {"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
        ] {
            for chunk in parse_sse_line(&mut tools, line) {
                chunks.push(chunk.unwrap());
            }
        }
        assert_eq!(chunks[0].delta, "Hello");
        let last = chunks.last().unwrap();
        assert!(last.done);
        assert_eq!(last.finish_reason.as_deref(), Some("stop"));
        assert!(last.tool_calls.is_none());
    }
}
