//! Anthropic Claude API provider.

use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::providers::stream_util::line_stream;
use crate::providers::{LlmError, Provider, ProviderConfig, Result};
use crate::types::{ChatRequest, ChatResponse, Content, ContentPart, StreamChunk, Usage};

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

#[derive(Serialize)]
struct AnthropicMessage {
    role: String,
    content: AnthropicMessageContent,
}

/// A Claude message body is either a plain string (text-only, the common case)
/// or an array of typed content blocks (used when images are present).
#[derive(Serialize)]
#[serde(untagged)]
enum AnthropicMessageContent {
    Text(String),
    Blocks(Vec<AnthropicContentBlock>),
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicContentBlock {
    Text { text: String },
    Image { source: AnthropicImageSource },
}

/// Claude accepts images either as inline base64 data or, on recent API
/// versions, by URL. Data URLs are decomposed into a base64 source; any other
/// URL is passed through as a `url` source.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicImageSource {
    Base64 { media_type: String, data: String },
    Url { url: String },
}

/// Map an llmrust [`Content`] into Claude's message content. Text stays a
/// plain string; mixed text/image parts become typed content blocks.
fn to_anthropic_content(content: &Content) -> AnthropicMessageContent {
    match content {
        Content::Text(text) => AnthropicMessageContent::Text(text.clone()),
        Content::Parts(parts) => {
            let mut blocks = Vec::new();
            for part in parts {
                match part {
                    ContentPart::Text { text } => {
                        blocks.push(AnthropicContentBlock::Text { text: text.clone() });
                    }
                    ContentPart::ImageUrl { image_url } => {
                        let source = anthropic_image_source(&image_url.url);
                        blocks.push(AnthropicContentBlock::Image { source });
                    }
                }
            }
            AnthropicMessageContent::Blocks(blocks)
        }
    }
}

/// Build a Claude image source from a URL. `data:` URLs are split into their
/// media type and base64 payload; everything else becomes a URL source.
fn anthropic_image_source(url: &str) -> AnthropicImageSource {
    if let Some(rest) = url.strip_prefix("data:") {
        if let Some((meta, data)) = rest.split_once(',') {
            let media_type = meta
                .split(';')
                .next()
                .filter(|s| !s.is_empty())
                .unwrap_or("image/png")
                .to_string();
            return AnthropicImageSource::Base64 {
                media_type,
                data: data.to_string(),
            };
        }
    }
    AnthropicImageSource::Url {
        url: url.to_string(),
    }
}

/// Split request messages into Claude's separate `system` prompt and the
/// `messages` array. System content is flattened to text (Claude takes the
/// system prompt as a string); user/assistant/tool turns keep full multimodal
/// content, with tool results folded back in as `user` turns.
fn split_messages(req: &ChatRequest) -> (Option<String>, Vec<AnthropicMessage>) {
    let mut system = None;
    let mut messages = Vec::new();
    for msg in &req.messages {
        match msg.role {
            crate::types::Role::System => system = Some(msg.content.as_text()),
            crate::types::Role::User => messages.push(AnthropicMessage {
                role: "user".to_string(),
                content: to_anthropic_content(&msg.content),
            }),
            crate::types::Role::Assistant => messages.push(AnthropicMessage {
                role: "assistant".to_string(),
                content: to_anthropic_content(&msg.content),
            }),
            crate::types::Role::Tool => messages.push(AnthropicMessage {
                role: "user".to_string(),
                content: to_anthropic_content(&msg.content),
            }),
        }
    }
    (system, messages)
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
    stop_reason: Option<String>,
}

/// Parse a single SSE line from an Anthropic stream into zero or more
/// [`StreamChunk`]s. Lines are guaranteed complete by [`line_stream`].
///
/// Anthropic delivers the stop reason on the `message_delta` event rather than
/// `message_stop`, so completion is keyed off that event's `stop_reason`.
fn parse_sse_line(line: &str) -> Vec<Result<StreamChunk>> {
    let line = line.trim();
    let Some(data) = line.strip_prefix("data: ") else {
        return Vec::new();
    };
    let Ok(event) = serde_json::from_str::<AnthropicStreamEvent>(data) else {
        return Vec::new();
    };
    match event.event_type.as_str() {
        "content_block_delta" => {
            let text = event.delta.and_then(|d| d.text).unwrap_or_default();
            vec![Ok(StreamChunk {
                delta: text,
                ..Default::default()
            })]
        }
        "message_delta" => {
            let finish_reason = event.delta.and_then(|d| d.stop_reason);
            vec![Ok(StreamChunk {
                done: finish_reason.is_some(),
                finish_reason,
                ..Default::default()
            })]
        }
        _ => Vec::new(),
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse> {
        let (system, messages) = split_messages(req);

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
            ..Default::default()
        })
    }

    async fn stream(&self, req: &ChatRequest) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        let (system, messages) = split_messages(req);

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Message;

    #[test]
    fn text_message_serializes_as_plain_string() {
        let content = to_anthropic_content(&Content::Text("hello".to_string()));
        let v = serde_json::to_value(&content).unwrap();
        assert_eq!(v, serde_json::json!("hello"));
    }

    #[test]
    fn data_url_image_becomes_base64_block() {
        let source = anthropic_image_source("data:image/jpeg;base64,QUJD");
        let v = serde_json::to_value(&source).unwrap();
        assert_eq!(v["type"], "base64");
        assert_eq!(v["media_type"], "image/jpeg");
        assert_eq!(v["data"], "QUJD");
    }

    #[test]
    fn http_url_image_becomes_url_block() {
        let source = anthropic_image_source("https://example.com/cat.png");
        let v = serde_json::to_value(&source).unwrap();
        assert_eq!(v["type"], "url");
        assert_eq!(v["url"], "https://example.com/cat.png");
    }

    #[test]
    fn split_messages_extracts_system_text() {
        let req = ChatRequest {
            model: "claude".to_string(),
            messages: vec![Message::system("be brief"), Message::user("hi")],
            temperature: None,
            max_tokens: None,
            stream: false,
            top_p: None,
            tools: None,
            tool_choice: None,
        };
        let (system, messages) = split_messages(&req);
        assert_eq!(system.as_deref(), Some("be brief"));
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role.as_str(), "user");
    }

    #[test]
    fn image_message_serializes_as_content_blocks() {
        let content = to_anthropic_content(&Content::Parts(vec![
            ContentPart::text("describe"),
            ContentPart::image_url("https://example.com/cat.png"),
        ]));
        let v = serde_json::to_value(&content).unwrap();
        assert_eq!(v[0]["type"], "text");
        assert_eq!(v[0]["text"], "describe");
        assert_eq!(v[1]["type"], "image");
        assert_eq!(v[1]["source"]["type"], "url");
        assert_eq!(v[1]["source"]["url"], "https://example.com/cat.png");
    }
}
