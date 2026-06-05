//! Google Gemini API provider.

use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::providers::stream_util::line_stream;
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
    system_instruction: Option<GeminiContent>,
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
    #[serde(default)]
    candidates: Vec<GeminiStreamCandidate>,
}

#[derive(Deserialize)]
struct GeminiStreamCandidate {
    #[serde(default)]
    content: Option<GeminiContentResponse>,
    #[serde(default, rename = "finishReason")]
    finish_reason: Option<String>,
}

/// Map an llmrust role to a Gemini `role`. Gemini only understands `user` and
/// `model`; system prompts are delivered separately via `systemInstruction`,
/// so a `System` message has no inline role here.
fn map_gemini_role(role: &crate::types::Role) -> Option<String> {
    match role {
        crate::types::Role::System => None,
        crate::types::Role::User => Some("user".to_string()),
        crate::types::Role::Assistant => Some("model".to_string()),
    }
}

/// Build Gemini `contents` (conversation turns) and an optional
/// `systemInstruction` from the request messages. System messages are
/// collected into the dedicated `systemInstruction` field instead of being
/// mislabeled as `user` turns (which would break Gemini's strict
/// user/model alternation).
fn build_contents(req: &ChatRequest) -> (Vec<GeminiContent>, Option<GeminiContent>) {
    let mut contents = Vec::new();
    let mut system_parts: Vec<GeminiPart> = Vec::new();

    for msg in &req.messages {
        match msg.role {
            crate::types::Role::System => system_parts.push(GeminiPart {
                text: msg.content.clone(),
            }),
            _ => contents.push(GeminiContent {
                parts: vec![GeminiPart {
                    text: msg.content.clone(),
                }],
                role: map_gemini_role(&msg.role),
            }),
        }
    }

    let system_instruction = if system_parts.is_empty() {
        None
    } else {
        Some(GeminiContent {
            parts: system_parts,
            role: None,
        })
    };

    (contents, system_instruction)
}

/// Parse a single SSE line from a Gemini stream into zero or more
/// [`StreamChunk`]s. Lines are guaranteed complete by [`line_stream`].
///
/// Completion is keyed off the real `finishReason` field rather than guessing
/// based on an empty chunk, which previously truncated responses whenever a
/// keep-alive or unparsed chunk arrived.
fn parse_sse_line(line: &str) -> Vec<Result<StreamChunk>> {
    let line = line.trim();
    let Some(data) = line.strip_prefix("data: ") else {
        return Vec::new();
    };
    let Ok(event) = serde_json::from_str::<GeminiStreamEvent>(data) else {
        return Vec::new();
    };
    let Some(candidate) = event.candidates.into_iter().next() else {
        return Vec::new();
    };

    let mut chunks = Vec::new();
    if let Some(text) = candidate
        .content
        .and_then(|c| c.parts.into_iter().next())
        .map(|p| p.text)
    {
        if !text.is_empty() {
            chunks.push(Ok(StreamChunk {
                delta: text,
                done: false,
            }));
        }
    }
    if candidate.finish_reason.is_some() {
        chunks.push(Ok(StreamChunk {
            delta: String::new(),
            done: true,
        }));
    }
    chunks
}

#[async_trait]
impl Provider for GoogleProvider {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse> {
        let (contents, system_instruction) = build_contents(req);

        let gen_config = GeminiGenerationConfig {
            temperature: req.temperature,
            max_output_tokens: req.max_tokens,
            top_p: req.top_p,
        };

        let body = GeminiRequest {
            contents: &contents,
            system_instruction,
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
        let (contents, system_instruction) = build_contents(req);

        let gen_config = GeminiGenerationConfig {
            temperature: req.temperature,
            max_output_tokens: req.max_tokens,
            top_p: req.top_p,
        };

        let body = GeminiRequest {
            contents: &contents,
            system_instruction,
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
