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
use crate::types::{ChatRequest, ChatResponse, Message, StreamChunk, Usage};

// ── Defaults ────────────────────────────────────────────────────

const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

// ── Shared request / response types ───────────────────────────────

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
}

#[derive(Serialize, Deserialize)]
struct CompMessage {
    role: String,
    content: String,
}

impl From<&Message> for CompMessage {
    fn from(msg: &Message) -> Self {
        Self {
            role: match msg.role {
                crate::types::Role::System => "system",
                crate::types::Role::User => "user",
                crate::types::Role::Assistant => "assistant",
            }
            .to_string(),
            content: msg.content.clone(),
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
}

#[derive(Deserialize)]
struct CompUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
}

#[derive(Deserialize)]
struct CompStreamChunk {
    choices: Vec<CompStreamChoice>,
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

// ── HTTP client construction ─────────────────────────────────────

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
fn parse_sse_line(line: &str) -> Vec<Result<StreamChunk>> {
    let line = line.trim();
    let Some(data) = line.strip_prefix("data: ") else {
        return Vec::new();
    };
    if data == "[DONE]" {
        return vec![Ok(StreamChunk {
            delta: String::new(),
            done: true,
        })];
    }
    let Ok(parsed) = serde_json::from_str::<CompStreamChunk>(data) else {
        return Vec::new();
    };
    let Some(choice) = parsed.choices.first() else {
        return Vec::new();
    };
    vec![Ok(StreamChunk {
        delta: choice.delta.content.clone().unwrap_or_default(),
        done: choice.finish_reason.is_some(),
    })]
}

// ── The unified OpenAI-compatible provider ───────────────────────────

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

    /// Send the request and parse the raw JSON response.
    async fn send_request(
        &self,
        messages: &[CompMessage],
        model: &str,
        temperature: Option<f64>,
        max_tokens: Option<u64>,
        top_p: Option<f64>,
        stream: bool,
    ) -> Result<reqwest::Response> {
        let body = CompChatRequest {
            model,
            messages,
            temperature,
            max_tokens,
            top_p,
            stream,
        };

        let mut rb = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key));

        for (k, v) in &self.extra_headers {
            rb = rb.header(k.as_str(), v.as_str());
        }

        let resp = rb.json(&body).send().await?;
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
        let content = parsed
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .unwrap_or_default();

        ChatResponse {
            content,
            model: parsed.model,
            usage: parsed.usage.map(|u| Usage {
                prompt_tokens: u.prompt_tokens,
                completion_tokens: u.completion_tokens,
                total_tokens: u.total_tokens,
            }),
        }
    }
}

#[async_trait]
impl Provider for OpenAiCompatibleProvider {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse> {
        let messages: Vec<CompMessage> = req.messages.iter().map(CompMessage::from).collect();

        let resp = self
            .send_request(
                &messages,
                &req.model,
                req.temperature,
                req.max_tokens,
                req.top_p,
                false,
            )
            .await?;

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

        let resp = self
            .send_request(
                &messages,
                &req.model,
                req.temperature,
                req.max_tokens,
                req.top_p,
                true,
            )
            .await?;

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
