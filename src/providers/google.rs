//! Google Gemini API provider.

use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::providers::{LlmError, Provider, ProviderConfig, Result};
use crate::types::{ChatRequest, ChatResponse, StreamChunk, Usage};

const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

pub struct GoogleProvider {
    client: Client,
    api_key: String,
    base_url: String,
}

impl GoogleProvider {
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

// --- Gemini API types ---

#[derive(Serialize)]
struct GeminiRequest<'a> {
    contents: &'a [GeminiContent],
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<GeminiGenerationConfig>,
}

#[derive(Serialize)]
struct GeminiContent {
    parts: Vec<GeminiPart>,
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
}

#[derive(Serialize)]
struct GeminiPart {
    text: String,
}

#[derive(Serialize)]
struct GeminiGenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f64>,
}

#[derive(Deserialize)]
struct GeminiResponse {
    candidates: Vec<GeminiCandidate>,
    #[serde(default)]
    model_version: String,
    #[serde(default)]
    usage_metadata: Option<GeminiUsageMetadata>,
}

#[derive(Deserialize)]
struct GeminiCandidate {
    content: GeminiContentResponse,
}

#[derive(Deserialize)]
struct GeminiContentResponse {
    parts: Vec<GeminiPartResponse>,
}

#[derive(Deserialize)]
struct GeminiPartResponse {
    text: String,
}

#[derive(Deserialize)]
struct GeminiUsageMetadata {
    #[serde(default)]
    prompt_token_count: u64,
    #[serde(default)]
    candidates_token_count: u64,
    #[serde(default)]
    total_token_count: u64,
}

#[derive(Deserialize)]
struct GeminiErrorBody {
    error: GeminiErrorDetail,
}

#[derive(Deserialize)]
struct GeminiErrorDetail {
    message: String,
}

// Stream SSE event
#[derive(Deserialize)]
struct GeminiStreamEvent {
    candidates: Vec<GeminiCandidate>,
}

fn map_gemini_role(role: &crate::types::Role) -> Option<String> {
    match role {
        crate::types::Role::System => Some("user".to_string()),
        crate::types::Role::User => Some("user".to_string()),
        crate::types::Role::Assistant => Some("model".to_string()),
    }
}

#[async_trait]
impl Provider for GoogleProvider {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse> {
        let contents: Vec<GeminiContent> = req
            .messages
            .iter()
            .map(|msg| GeminiContent {
                parts: vec![GeminiPart {
                    text: msg.content.clone(),
                }],
                role: map_gemini_role(&msg.role),
            })
            .collect();

        let gen_config = GeminiGenerationConfig {
            temperature: req.temperature,
            max_output_tokens: req.max_tokens,
            top_p: req.top_p,
        };

        let body = GeminiRequest {
            contents: &contents,
            generation_config: Some(gen_config),
        };

        let url = format!(
            "{}/models/{}:generateContent?key={}",
            self.base_url, req.model, self.api_key
        );

        let resp = self.client.post(&url).json(&body).send().await?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            let msg = serde_json::from_str::<GeminiErrorBody>(&text)
                .map(|e| e.error.message)
                .unwrap_or(text);
            return Err(LlmError::Api {
                status: status.as_u16(),
                message: msg,
            });
        }

        let parsed: GeminiResponse = resp
            .json()
            .await
            .map_err(|e| LlmError::Parse(e.to_string()))?;

        let content = parsed
            .candidates
            .first()
            .and_then(|c| c.content.parts.first())
            .map(|p| p.text.clone())
            .unwrap_or_default();

        Ok(ChatResponse {
            content,
            model: parsed.model_version,
            usage: parsed.usage_metadata.map(|u| Usage {
                prompt_tokens: u.prompt_token_count,
                completion_tokens: u.candidates_token_count,
                total_tokens: u.total_token_count,
            }),
        })
    }

    async fn stream(&self, req: &ChatRequest) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        let contents: Vec<GeminiContent> = req
            .messages
            .iter()
            .map(|msg| GeminiContent {
                parts: vec![GeminiPart {
                    text: msg.content.clone(),
                }],
                role: map_gemini_role(&msg.role),
            })
            .collect();

        let gen_config = GeminiGenerationConfig {
            temperature: req.temperature,
            max_output_tokens: req.max_tokens,
            top_p: req.top_p,
        };

        let body = GeminiRequest {
            contents: &contents,
            generation_config: Some(gen_config),
        };

        let url = format!(
            "{}/models/{}:streamGenerateContent?alt=sse&key={}",
            self.base_url, req.model, self.api_key
        );

        let resp = self.client.post(&url).json(&body).send().await?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            let msg = serde_json::from_str::<GeminiErrorBody>(&text)
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
                    if let Ok(event) = serde_json::from_str::<GeminiStreamEvent>(data) {
                        if let Some(candidate) = event.candidates.first() {
                            if let Some(part) = candidate.content.parts.first() {
                                let delta = part.text.clone();
                                chunks.push(Ok(StreamChunk { delta, done: false }));
                            }
                        }
                    }
                }
                // If we got no chunks from this read, the stream might be done
                if chunks.is_empty() {
                    chunks.push(Ok(StreamChunk {
                        delta: String::new(),
                        done: true,
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
