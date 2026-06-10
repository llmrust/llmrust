//! Ollama local LLM provider.

use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::providers::http::build_http_client;
use crate::providers::stream_util::line_stream;
use crate::providers::{LlmError, Provider, ProviderConfig, Result};
use crate::types::{ChatRequest, ChatResponse, Message, StreamChunk, Usage};

const DEFAULT_BASE_URL: &str = "http://localhost:11434";

pub struct OllamaProvider {
    client: Client,
    base_url: String,
}

impl OllamaProvider {
    pub fn new(config: ProviderConfig) -> Self {
        Self {
            // Local generation can run for a long time on large models, so opt
            // out of the overall request timeout; only the connect timeout from
            // the shared builder applies (to fail fast when unreachable).
            client: build_http_client(None),
            base_url: config
                .base_url
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
        }
    }
}

// --- Ollama API types ---

#[derive(Serialize)]
struct OllamaRequest<'a> {
    model: &'a str,
    messages: &'a [OllamaMessage],
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<OllamaOptions>,
}

/// Build `OllamaOptions` only when at least one field is set, so the request
/// body stays clean when the caller doesn't override any sampling parameter.
fn build_ollama_options(req: &ChatRequest) -> Option<OllamaOptions> {
    if req.temperature.is_some() || req.max_tokens.is_some() || req.top_p.is_some() {
        Some(OllamaOptions {
            temperature: req.temperature,
            num_predict: req.max_tokens,
            top_p: req.top_p,
        })
    } else {
        None
    }
}

#[derive(Serialize)]
struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    num_predict: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f64>,
}

#[derive(Serialize, Deserialize)]
struct OllamaMessage {
    role: String,
    content: String,
}

impl From<&Message> for OllamaMessage {
    fn from(msg: &Message) -> Self {
        Self {
            role: match msg.role {
                crate::types::Role::System => "system".to_string(),
                crate::types::Role::User => "user".to_string(),
                crate::types::Role::Assistant => "assistant".to_string(),
                crate::types::Role::Tool => "tool".to_string(),
            },
            // Ollama's /api/chat expects a plain string body, so multimodal
            // content parts are flattened to their concatenated text.
            content: msg.content.as_text(),
        }
    }
}

#[derive(Deserialize)]
struct OllamaResponse {
    message: OllamaMessageResponse,
    model: String,
    #[serde(default)]
    eval_count: u64,
    #[serde(default)]
    prompt_eval_count: u64,
}

#[derive(Deserialize)]
struct OllamaMessageResponse {
    #[serde(default)]
    content: String,
}

#[derive(Deserialize)]
struct OllamaStreamChunk {
    message: Option<OllamaMessageResponse>,
    done: bool,
    #[serde(default)]
    done_reason: Option<String>,
    #[serde(default)]
    eval_count: u64,
    #[serde(default)]
    prompt_eval_count: u64,
}

#[derive(Deserialize)]
struct OllamaErrorBody {
    error: String,
}

/// Parse a single newline-delimited JSON line from an Ollama stream into zero
/// or more [`StreamChunk`]s. Lines are guaranteed complete by [`line_stream`],
/// so a JSON object split across network chunks is reassembled before parsing
/// and multi-byte UTF-8 (e.g. CJK / emoji) is never corrupted.
fn parse_ndjson_line(line: &str) -> Vec<Result<StreamChunk>> {
    let line = line.trim();
    if line.is_empty() {
        return Vec::new();
    }
    let Ok(parsed) = serde_json::from_str::<OllamaStreamChunk>(line) else {
        return Vec::new();
    };
    if parsed.done {
        vec![Ok(StreamChunk {
            done: true,
            finish_reason: Some(parsed.done_reason.unwrap_or_else(|| "stop".to_string())),
            usage: Some(Usage {
                prompt_tokens: parsed.prompt_eval_count,
                completion_tokens: parsed.eval_count,
                total_tokens: parsed.prompt_eval_count.saturating_add(parsed.eval_count),
            }),
            ..Default::default()
        })]
    } else if let Some(msg) = parsed.message {
        vec![Ok(StreamChunk {
            delta: msg.content,
            ..Default::default()
        })]
    } else {
        Vec::new()
    }
}

#[async_trait]
impl Provider for OllamaProvider {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse> {
        let messages: Vec<OllamaMessage> = req.messages.iter().map(OllamaMessage::from).collect();

        let body = OllamaRequest {
            model: &req.model,
            messages: &messages,
            stream: false,
            options: build_ollama_options(req),
        };

        tracing::debug!(
            provider = "ollama",
            model = &req.model,
            "sending chat request"
        );
        let resp = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            let msg = serde_json::from_str::<OllamaErrorBody>(&text)
                .map(|e| e.error)
                .unwrap_or(text);
            let err = LlmError::Api {
                status: status.as_u16(),
                message: msg,
            };
            tracing::error!(provider = "ollama", status = status.as_u16(), error = %err, "API error");
            return Err(err);
        }

        let parsed: OllamaResponse = resp
            .json()
            .await
            .map_err(|e| LlmError::Parse(e.to_string()))?;

        let result = ChatResponse {
            content: parsed.message.content,
            model: parsed.model,
            usage: Some(Usage {
                prompt_tokens: parsed.prompt_eval_count,
                completion_tokens: parsed.eval_count,
                total_tokens: parsed.prompt_eval_count.saturating_add(parsed.eval_count),
            }),
            ..Default::default()
        };
        tracing::debug!(
            provider = "ollama",
            model = &result.model,
            "chat response received"
        );
        Ok(result)
    }

    async fn stream(&self, req: &ChatRequest) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        let messages: Vec<OllamaMessage> = req.messages.iter().map(OllamaMessage::from).collect();

        let body = OllamaRequest {
            model: &req.model,
            messages: &messages,
            stream: true,
            options: build_ollama_options(req),
        };

        tracing::debug!(
            provider = "ollama",
            model = &req.model,
            "sending stream request"
        );
        let resp = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            let msg = serde_json::from_str::<OllamaErrorBody>(&text)
                .map(|e| e.error)
                .unwrap_or(text);
            let err = LlmError::Api {
                status: status.as_u16(),
                message: msg,
            };
            tracing::error!(provider = "ollama", status = status.as_u16(), error = %err, "API error");
            return Err(err);
        }

        let byte_stream = resp
            .bytes_stream()
            .map(|r| r.map_err(|e| LlmError::Stream(e.to_string())));

        let stream = line_stream(byte_stream).flat_map(|line_result| {
            let chunks = match line_result {
                Ok(line) => parse_ndjson_line(&line),
                Err(e) => vec![Err(e)],
            };
            futures::stream::iter(chunks)
        });

        Ok(stream.boxed())
    }
}
