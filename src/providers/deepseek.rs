//! DeepSeek API provider (OpenAI-compatible).

use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::providers::{LlmError, Provider, ProviderConfig, Result};
use crate::types::{ChatRequest, ChatResponse, Message, StreamChunk, Usage};

const DEFAULT_BASE_URL: &str = "https://api.deepseek.com/v1";

pub struct DeepSeekProvider {
    client: Client,
    api_key: String,
    base_url: String,
}

impl DeepSeekProvider {
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

// DeepSeek uses OpenAI-compatible API, so types are similar.

#[derive(Serialize)]
struct DeepSeekRequest<'a> {
    model: &'a str,
    messages: &'a [DeepSeekMessage],
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f64>,
    stream: bool,
}

#[derive(Serialize, Deserialize)]
struct DeepSeekMessage {
    role: String,
    content: String,
}

impl From<&Message> for DeepSeekMessage {
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
struct DeepSeekResponse {
    choices: Vec<DeepSeekChoice>,
    model: String,
    usage: Option<DeepSeekUsage>,
}

#[derive(Deserialize)]
struct DeepSeekChoice {
    message: DeepSeekMessage,
}

#[derive(Deserialize)]
struct DeepSeekUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
}

#[derive(Deserialize)]
struct DeepSeekStreamChunk {
    choices: Vec<DeepSeekStreamChoice>,
}

#[derive(Deserialize)]
struct DeepSeekStreamChoice {
    delta: DeepSeekDelta,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct DeepSeekDelta {
    content: Option<String>,
}

#[derive(Deserialize)]
struct DeepSeekErrorBody {
    error: DeepSeekErrorDetail,
}

#[derive(Deserialize)]
struct DeepSeekErrorDetail {
    message: String,
}

#[async_trait]
impl Provider for DeepSeekProvider {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse> {
        let messages: Vec<DeepSeekMessage> =
            req.messages.iter().map(DeepSeekMessage::from).collect();

        let body = DeepSeekRequest {
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
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            let msg = serde_json::from_str::<DeepSeekErrorBody>(&text)
                .map(|e| e.error.message)
                .unwrap_or(text);
            return Err(LlmError::Api {
                status: status.as_u16(),
                message: msg,
            });
        }

        let parsed: DeepSeekResponse = resp
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
        let messages: Vec<DeepSeekMessage> =
            req.messages.iter().map(DeepSeekMessage::from).collect();

        let body = DeepSeekRequest {
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
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            let msg = serde_json::from_str::<DeepSeekErrorBody>(&text)
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
                    if let Ok(parsed) = serde_json::from_str::<DeepSeekStreamChunk>(data) {
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
