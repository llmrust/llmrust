//! Anthropic Claude API provider.

use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::providers::stream_util::line_stream;
use crate::providers::{LlmError, Provider, ProviderConfig, Result};
use crate::types::{ChatRequest, ChatResponse, StreamChunk, Usage};

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com/v1";

pub struct AnthropicProvider {
    client: Client,
    api_key: String,
    base_url: String,
}

impl AnthropicProvider {
    pub fn new(config: ProviderConfig) -> Self {
        Self {
            client: Client::new(),
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
    stream: bool,
}

#[derive(Serialize, Deserialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContent>,
    model: String,
    usage: Option<AnthropicUsage>,
}

#[derive(Deserialize)]
struct AnthropicContent {
    text: String,
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
}

/// Parse a single SSE line from an Anthropic stream into zero or more
/// [`StreamChunk`]s. Lines are guaranteed complete by [`line_stream`].
fn parse_sse_line(line: &str) -> Vec<Result<StreamChunk>> {
    let line = line.trim();
    let Some(data) = line.strip_prefix("data: ") else {
        return Vec::new();
    };
    let Ok(event) = serde_json::from_str::<AnthropicStreamEvent>(data) else {
        return Vec::new();
    };
    match event.event_type.as_str() {
        "content_block_delta" => event
            .delta
            .and_then(|d| d.text)
            .map(|text| {
                vec![Ok(StreamChunk {
                    delta: text,
                    done: false,
                })]
            })
            .unwrap_or_default(),
        "message_stop" => vec![Ok(StreamChunk {
            delta: String::new(),
            done: true,
        })],
        _ => Vec::new(),
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse> {
        // Separate system message from conversation messages
        let (system, messages): (Option<String>, Vec<AnthropicMessage>) = {
            let mut sys = None;
            let mut msgs = Vec::new();
            for msg in &req.messages {
                match msg.role {
                    crate::types::Role::System => sys = Some(msg.content.clone()),
                    crate::types::Role::User => msgs.push(AnthropicMessage {
                        role: "user".to_string(),
                        content: msg.content.clone(),
                    }),
                    crate::types::Role::Assistant => msgs.push(AnthropicMessage {
                        role: "assistant".to_string(),
                        content: msg.content.clone(),
                    }),
                }
            }
            (sys, msgs)
        };

        let body = AnthropicRequest {
            model: &req.model,
            messages,
            system,
            temperature: req.temperature,
            max_tokens: req.max_tokens.or(Some(4096)), // Anthropic requires max_tokens
            top_p: req.top_p,
            stream: false,
        };

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
            .first()
            .map(|c| c.text.clone())
            .unwrap_or_default();

        Ok(ChatResponse {
            content,
            model: parsed.model,
            usage: parsed.usage.map(|u| Usage {
                prompt_tokens: u.input_tokens,
                completion_tokens: u.output_tokens,
                total_tokens: u.input_tokens.saturating_add(u.output_tokens),
            }),
        })
    }

    async fn stream(&self, req: &ChatRequest) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        let (system, messages): (Option<String>, Vec<AnthropicMessage>) = {
            let mut sys = None;
            let mut msgs = Vec::new();
            for msg in &req.messages {
                match msg.role {
                    crate::types::Role::System => sys = Some(msg.content.clone()),
                    crate::types::Role::User => msgs.push(AnthropicMessage {
                        role: "user".to_string(),
                        content: msg.content.clone(),
                    }),
                    crate::types::Role::Assistant => msgs.push(AnthropicMessage {
                        role: "assistant".to_string(),
                        content: msg.content.clone(),
                    }),
                }
            }
            (sys, msgs)
        };

        let body = AnthropicRequest {
            model: &req.model,
            messages,
            system,
            temperature: req.temperature,
            max_tokens: req.max_tokens.or(Some(4096)),
            top_p: req.top_p,
            stream: true,
        };

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
