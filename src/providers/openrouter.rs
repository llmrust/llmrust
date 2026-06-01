//! OpenRouter API provider (OpenAI-compatible).

use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::providers::{LlmError, Provider, ProviderConfig, Result};
use crate::types::{ChatRequest, ChatResponse, Message, StreamChunk, Usage};

const DEFAULT_BASE_URL: &str = "https://openrouter.ai/api/v1";

pub struct OpenRouterProvider {
    client: Client,
    api_key: String,
    base_url: String,
}

impl OpenRouterProvider {
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

// --- OpenAI-compatible types for OpenRouter ---

#[derive(Serialize)]
struct OpenRouterRequest<'a> {
    model: &'a str,
    messages: &'a [OpenRouterMessage],
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f64>,
    stream: bool,
}

#[derive(Serialize, Deserialize)]
struct OpenRouterMessage {
    role: String,
    content: String,
}

impl From<&Message> for OpenRouterMessage {
    fn from(msg: &Message) -> Self {
        Self {
            role: match msg.role {
                crate::types::Role::System => "system".to_string(),
                crate::types::Role::User => "user".to_string(),
                crate::types::Role::Assistant => "assistant".to_string(),
            },
            content: msg.content.clone(),
        }
    }
}

#[derive(Deserialize)]
struct OpenRouterResponse {
    choices: Vec<OpenRouterChoice>,
    model: String,
    usage: Option<OpenRouterUsage>,
}

#[derive(Deserialize)]
struct OpenRouterChoice {
    message: OpenRouterMessage,
}

#[derive(Deserialize)]
struct OpenRouterUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
}

#[derive(Deserialize)]
struct OpenRouterStreamChunk {
    choices: Vec<OpenRouterStreamChoice>,
}

#[derive(Deserialize)]
struct OpenRouterStreamChoice {
    delta: OpenRouterDelta,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct OpenRouterDelta {
    content: Option<String>,
}

#[derive(Deserialize)]
struct OpenRouterErrorBody {
    error: OpenRouterErrorDetail,
}

#[derive(Deserialize)]
struct OpenRouterErrorDetail {
    message: String,
}

#[async_trait]
impl Provider for OpenRouterProvider {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse> {
        let messages: Vec<OpenRouterMessage> =
            req.messages.iter().map(OpenRouterMessage::from).collect();

        let body = OpenRouterRequest {
            model: &req.model,
            messages: &messages,
            temperature: req.temperature,
            max_tokens: req.max_tokens,
            top_p: req.top_p,
            stream: false,
        };

        let resp = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("HTTP-Referer", "https://github.com/llmrust/llmrust")
            .header("X-Title", "llmrust")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            let msg = serde_json::from_str::<OpenRouterErrorBody>(&text)
                .map(|e| e.error.message)
                .unwrap_or(text);
            return Err(LlmError::Api {
                status: status.as_u16(),
                message: msg,
            });
        }

        let parsed: OpenRouterResponse = resp
            .json()
            .await
            .map_err(|e| LlmError::Parse(e.to_string()))?;

        let content = parsed
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .unwrap_or_default();

        Ok(ChatResponse {
            content,
            model: parsed.model,
            usage: parsed.usage.map(|u| Usage {
                prompt_tokens: u.prompt_tokens,
                completion_tokens: u.completion_tokens,
                total_tokens: u.total_tokens,
            }),
        })
    }

    async fn stream(&self, req: &ChatRequest) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        let messages: Vec<OpenRouterMessage> =
            req.messages.iter().map(OpenRouterMessage::from).collect();

        let body = OpenRouterRequest {
            model: &req.model,
            messages: &messages,
            temperature: req.temperature,
            max_tokens: req.max_tokens,
            top_p: req.top_p,
            stream: true,
        };

        let resp = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("HTTP-Referer", "https://github.com/llmrust/llmrust")
            .header("X-Title", "llmrust")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            let msg = serde_json::from_str::<OpenRouterErrorBody>(&text)
                .map(|e| e.error.message)
                .unwrap_or(text);
            return Err(LlmError::Api {
                status: status.as_u16(),
                message: msg,
            });
        }

        let stream = resp
            .bytes_stream()
            .map(|chunk_result| {
                let bytes = chunk_result.map_err(|e| LlmError::Stream(e.to_string()))?;
                let text = String::from_utf8_lossy(&bytes);

                let mut chunks = Vec::new();
                for line in text.lines() {
                    let line = line.trim();
                    if !line.starts_with("data: ") {
                        continue;
                    }
                    let data = &line[6..];
                    if data == "[DONE]" {
                        chunks.push(Ok(StreamChunk {
                            delta: String::new(),
                            done: true,
                        }));
                        continue;
                    }
                    if let Ok(parsed) = serde_json::from_str::<OpenRouterStreamChunk>(data) {
                        if let Some(choice) = parsed.choices.first() {
                            let delta = choice.delta.content.clone().unwrap_or_default();
                            let done = choice.finish_reason.is_some();
                            chunks.push(Ok(StreamChunk { delta, done }));
                        }
                    }
                }
                Ok(chunks)
            })
            .flat_map(|result| match result {
                Ok(chunks) => futures::stream::iter(chunks).boxed(),
                Err(e) => futures::stream::once(async move { Err(e) }).boxed(),
            });

        Ok(stream.boxed())
    }
}
