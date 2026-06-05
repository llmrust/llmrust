//! Ollama local LLM provider.

use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};

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
            client: Client::new(),
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
            content: msg.content.clone(),
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
    eval_count: u64,
    #[serde(default)]
    prompt_eval_count: u64,
}

#[derive(Deserialize)]
struct OllamaErrorBody {
    error: String,
}

#[async_trait]
impl Provider for OllamaProvider {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse> {
        let messages: Vec<OllamaMessage> = req.messages.iter().map(OllamaMessage::from).collect();

        let body = OllamaRequest {
            model: &req.model,
            messages: &messages,
            stream: false,
            options: Some(OllamaOptions {
                temperature: req.temperature,
                num_predict: req.max_tokens,
                top_p: req.top_p,
            }),
        };

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
            return Err(LlmError::Api {
                status: status.as_u16(),
                message: msg,
            });
        }

        let parsed: OllamaResponse = resp
            .json()
            .await
            .map_err(|e| LlmError::Parse(e.to_string()))?;

        Ok(ChatResponse {
            content: parsed.message.content,
            model: parsed.model,
            usage: Some(Usage {
                prompt_tokens: parsed.prompt_eval_count,
                completion_tokens: parsed.eval_count,
                total_tokens: parsed.prompt_eval_count + parsed.eval_count,
            }),
            ..Default::default()
        })
    }

    async fn stream(&self, req: &ChatRequest) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        let messages: Vec<OllamaMessage> = req.messages.iter().map(OllamaMessage::from).collect();

        let body = OllamaRequest {
            model: &req.model,
            messages: &messages,
            stream: true,
            options: Some(OllamaOptions {
                temperature: req.temperature,
                num_predict: req.max_tokens,
                top_p: req.top_p,
            }),
        };

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
                // Ollama stream is newline-delimited JSON
                for line in text.lines() {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    if let Ok(parsed) = serde_json::from_str::<OllamaStreamChunk>(line) {
                        if parsed.done {
                            chunks.push(Ok(StreamChunk {
                                done: true,
                                finish_reason: Some("stop".to_string()),
                                usage: Some(Usage {
                                    prompt_tokens: parsed.prompt_eval_count,
                                    completion_tokens: parsed.eval_count,
                                    total_tokens: parsed.prompt_eval_count + parsed.eval_count,
                                }),
                                ..Default::default()
                            }));
                        } else if let Some(msg) = parsed.message {
                            chunks.push(Ok(StreamChunk {
                                delta: msg.content,
                                ..Default::default()
                            }));
                        }
                    }
                }
                if chunks.is_empty() {
                    chunks.push(Ok(StreamChunk {
                        done: true,
                        ..Default::default()
                    }));
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
