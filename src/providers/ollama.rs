//! Ollama local LLM provider.

use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::providers::http::build_http_client;
use crate::providers::stream_util::line_stream;
use crate::providers::{LlmError, Provider, ProviderConfig, Result};
use crate::types::{
    ChatRequest, ChatResponse, Embedding, EmbeddingRequest, EmbeddingResponse, EmbeddingUsage,
    FinishReason, Message, StreamChunk, ThinkingConfig, Usage,
};

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
            client: build_http_client(
                config.timeout_secs.map(Duration::from_secs),
                config.custom_headers.as_ref(),
            ),
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

/// True when reasoning is enabled. Ollama's wire has no lossless mapping for
/// `ThinkingConfig.budget_tokens` (`options.think` is bool/level), so any
/// `Enabled` reasoning request is rejected before a network call (REA-001 §2.5,
/// REA-004O).
fn thinking_enabled(cfg: &Option<ThinkingConfig>) -> bool {
    matches!(cfg, Some(ThinkingConfig::Enabled { .. }))
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

// ── Ollama embeddings types ───────────────────────────────────────

#[derive(Serialize)]
struct OllamaEmbedRequest<'a> {
    model: &'a str,
    input: &'a [String],
    #[serde(skip_serializing_if = "Option::is_none")]
    dimensions: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    truncate: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    keep_alive: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct OllamaEmbedResponse {
    #[serde(default)]
    model: Option<String>,
    embeddings: Vec<Vec<f32>>,
    #[serde(default)]
    prompt_eval_count: Option<u64>,
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
    let parsed = match serde_json::from_str::<OllamaStreamChunk>(line) {
        Ok(p) => p,
        Err(e) => {
            return vec![Err(LlmError::Parse(format!(
                "failed to parse Ollama stream chunk: {e}"
            )))];
        }
    };
    if parsed.done {
        vec![Ok(StreamChunk {
            done: true,
            finish_reason: Some(
                parsed
                    .done_reason
                    .map(FinishReason::from)
                    .unwrap_or(FinishReason::Stop),
            ),
            usage: Some(Usage {
                prompt_tokens: parsed.prompt_eval_count,
                completion_tokens: parsed.eval_count,
                total_tokens: parsed.prompt_eval_count.saturating_add(parsed.eval_count),
                ..Default::default()
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
        // REA-004O (REA-001 §2.5): Ollama wire has no lossless mapping for
        // `ThinkingConfig.budget_tokens`, so Enabled reasoning fails BEFORE any
        // network call; Disabled/None pass through unchanged.
        if thinking_enabled(&req.thinking) {
            return Err(LlmError::Unsupported {
                feature: "reasoning".to_string(),
                message: "Ollama reasoning is unsupported in 0.1.3 (no lossless wire \
                          mapping for ThinkingConfig.budget_tokens)"
                    .to_string(),
            });
        }
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
            tracing::error!(
                provider = "ollama",
                status = status.as_u16(),
                error_kind = "api_error",
                "API error"
            );
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
                ..Default::default()
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
        // REA-004O: same gate as chat — Enabled reasoning fails BEFORE any
        // network call; Disabled/None pass through unchanged.
        if thinking_enabled(&req.thinking) {
            return Err(LlmError::Unsupported {
                feature: "reasoning".to_string(),
                message: "Ollama reasoning is unsupported in 0.1.3 (no lossless wire \
                          mapping for ThinkingConfig.budget_tokens)"
                    .to_string(),
            });
        }
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
            tracing::error!(
                provider = "ollama",
                status = status.as_u16(),
                error_kind = "api_error",
                "API error"
            );
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

    async fn embed(&self, req: &EmbeddingRequest) -> Result<EmbeddingResponse> {
        // Pick known Ollama-specific fields from extra; ignore everything else
        let truncate = req.extra.get("truncate").and_then(|v| v.as_bool());
        let options = req.extra.get("options").cloned();
        let keep_alive = req.extra.get("keep_alive").cloned();

        let body = OllamaEmbedRequest {
            model: &req.model,
            input: &req.input,
            dimensions: req.dimensions,
            truncate,
            options,
            keep_alive,
        };

        tracing::debug!(
            provider = "ollama",
            model = %req.model,
            input_count = req.input.len(),
            "sending embedding request"
        );

        let resp = self
            .client
            .post(format!("{}/api/embed", self.base_url))
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
            tracing::error!(
                provider = "ollama",
                status = status.as_u16(),
                error_kind = "api_error",
                "embedding API error"
            );
            return Err(err);
        }

        let parsed: OllamaEmbedResponse = resp
            .json()
            .await
            .map_err(|e| LlmError::Parse(format!("Ollama embed parse: {e}")))?;

        let data: Vec<Embedding> = parsed
            .embeddings
            .into_iter()
            .enumerate()
            .map(|(i, vec)| Embedding {
                index: i,
                embedding: vec,
            })
            .collect();

        let usage = parsed.prompt_eval_count.map(|c| EmbeddingUsage {
            prompt_tokens: c,
            total_tokens: c,
        });

        let model = parsed.model.unwrap_or_else(|| req.model.clone());

        let result = EmbeddingResponse { model, data, usage };

        tracing::debug!(
            provider = "ollama",
            model = %result.model,
            embedding_count = result.data.len(),
            "embedding response received"
        );
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn ollama_stream_malformed_json_returns_parse_error() {
        let chunks = parse_ndjson_line("{not valid json}");
        let err = chunks.into_iter().next().unwrap().unwrap_err();
        assert!(matches!(err, LlmError::Parse(_)));
    }

    #[test]
    fn ollama_stream_ignores_empty_lines() {
        let chunks = parse_ndjson_line("");
        assert!(chunks.is_empty());
    }

    #[test]
    fn ollama_stream_valid_chunk_still_parses() {
        let chunks = parse_ndjson_line(r#"{"message":{"content":"hello"},"done":false}"#);
        let chunk = chunks.into_iter().next().unwrap().unwrap();
        assert_eq!(chunk.delta, "hello");
        assert!(!chunk.done);
    }

    // ── embedding fake server tests ───────────────────────────────

    use std::io::{Read, Write};
    use std::net::TcpListener as StdTcpListener;
    use std::sync::Barrier;

    fn fake_ollama_server(
        handler: impl Fn(&str, &str, &[u8]) -> (u16, String) + Send + 'static,
    ) -> String {
        let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{}", addr);
        let barrier = Arc::new(Barrier::new(2));
        let b = Arc::clone(&barrier);

        std::thread::spawn(move || {
            b.wait();
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = vec![0u8; 16384];
                let n = stream.read(&mut buf).unwrap_or(0);
                let raw = String::from_utf8_lossy(&buf[..n]).to_string();
                let first_line = raw.lines().next().unwrap_or("");
                let mut parts = first_line.split_whitespace();
                let method = parts.next().unwrap_or("GET");
                let path = parts.next().unwrap_or("/");
                let body_start = raw.find("\r\n\r\n").map(|i| i + 4).unwrap_or(raw.len());
                let body_bytes = &buf[body_start..n];
                let (status, body) = handler(method, path, body_bytes);
                let resp = format!(
                    "HTTP/1.1 {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    status,
                    body.len(),
                    body
                );
                stream.write_all(resp.as_bytes()).ok();
                stream.flush().ok();
                let _ = stream.read(&mut [0u8; 1]);
            }
        });
        barrier.wait();
        url
    }

    #[test]
    fn ollama_embed_posts_to_api_embed() {
        let url = fake_ollama_server(|method, path, body| {
            assert_eq!(method, "POST");
            assert_eq!(path, "/api/embed");
            let json: serde_json::Value = serde_json::from_slice(body).unwrap();
            assert_eq!(json["model"], "all-minilm");
            assert_eq!(json["input"][0], "hello");
            (200, r#"{"model":"all-minilm","embeddings":[[0.1]]}"#.into())
        });
        let config = ProviderConfig::new("").with_base_url(&url);
        let provider = OllamaProvider::new(config);
        let req = EmbeddingRequest::new("all-minilm", "hello");
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(provider.embed(&req)).unwrap();
    }

    #[test]
    fn ollama_embed_accepts_batch_and_preserves_order() {
        let url = fake_ollama_server(|_m, _p, _b| {
            (200, r#"{"model":"m","embeddings":[[0.1],[0.2]]}"#.into())
        });
        let config = ProviderConfig::new("").with_base_url(&url);
        let provider = OllamaProvider::new(config);
        let req = EmbeddingRequest::batch("m", ["a", "b"]);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let resp = rt.block_on(provider.embed(&req)).unwrap();
        assert_eq!(resp.data.len(), 2);
        assert_eq!(resp.data[0].index, 0);
        assert_eq!(resp.data[0].embedding, vec![0.1_f32]);
        assert_eq!(resp.data[1].index, 1);
        assert_eq!(resp.data[1].embedding, vec![0.2_f32]);
    }

    #[test]
    fn ollama_embed_sends_dimensions() {
        let url = fake_ollama_server(|_m, _p, body| {
            let json: serde_json::Value = serde_json::from_slice(body).unwrap();
            assert_eq!(json["dimensions"], 384);
            (200, r#"{"model":"m","embeddings":[[0.1]]}"#.into())
        });
        let config = ProviderConfig::new("").with_base_url(&url);
        let provider = OllamaProvider::new(config);
        let req = EmbeddingRequest::new("m", "hi").with_dimensions(384);
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(provider.embed(&req)).unwrap();
    }

    #[test]
    fn ollama_embed_forwards_truncate_options_keep_alive_from_extra() {
        let url = fake_ollama_server(|_m, _p, body| {
            let json: serde_json::Value = serde_json::from_slice(body).unwrap();
            assert_eq!(json["truncate"], false);
            assert_eq!(json["options"]["temperature"], 0);
            assert_eq!(json["keep_alive"], "10m");
            (200, r#"{"model":"m","embeddings":[[0.1]]}"#.into())
        });
        let config = ProviderConfig::new("").with_base_url(&url);
        let provider = OllamaProvider::new(config);
        let req = EmbeddingRequest::new("m", "hi")
            .with_extra("truncate", false)
            .with_extra("options", serde_json::json!({"temperature": 0}))
            .with_extra("keep_alive", "10m");
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(provider.embed(&req)).unwrap();
    }

    #[test]
    fn ollama_embed_parses_usage_from_prompt_eval_count() {
        let url = fake_ollama_server(|_m, _p, _b| {
            (
                200,
                r#"{"model":"m","embeddings":[[0.1,0.2]],"prompt_eval_count":8}"#.into(),
            )
        });
        let config = ProviderConfig::new("").with_base_url(&url);
        let provider = OllamaProvider::new(config);
        let req = EmbeddingRequest::new("m", "hi");
        let rt = tokio::runtime::Runtime::new().unwrap();
        let resp = rt.block_on(provider.embed(&req)).unwrap();
        let usage = resp.usage.expect("usage should be present");
        assert_eq!(usage.prompt_tokens, 8);
        assert_eq!(usage.total_tokens, 8);
    }

    #[test]
    fn ollama_embed_model_falls_back_to_request_model() {
        let url = fake_ollama_server(|_m, _p, _b| (200, r#"{"embeddings":[[0.1]]}"#.into()));
        let config = ProviderConfig::new("").with_base_url(&url);
        let provider = OllamaProvider::new(config);
        let req = EmbeddingRequest::new("fallback-model", "hi");
        let rt = tokio::runtime::Runtime::new().unwrap();
        let resp = rt.block_on(provider.embed(&req)).unwrap();
        assert_eq!(resp.model, "fallback-model");
    }

    #[test]
    fn ollama_embed_maps_api_error() {
        let url = fake_ollama_server(|_m, _p, _b| (404, r#"{"error":"model not found"}"#.into()));
        let config = ProviderConfig::new("").with_base_url(&url);
        let provider = OllamaProvider::new(config);
        let req = EmbeddingRequest::new("nonexistent", "hi");
        let rt = tokio::runtime::Runtime::new().unwrap();
        let err = rt.block_on(provider.embed(&req)).unwrap_err();
        assert!(
            matches!(&err, LlmError::Api { status: 404, message } if message.contains("model not found"))
        );
    }

    #[test]
    fn lmrs_client_ollama_embed_strips_prefix() {
        use crate::LmrsClient;
        let url = fake_ollama_server(|_m, _p, body| {
            let json: serde_json::Value = serde_json::from_slice(body).unwrap();
            assert_eq!(json["model"], "all-minilm");
            (200, r#"{"model":"all-minilm","embeddings":[[0.1]]}"#.into())
        });
        let config = ProviderConfig::new("").with_base_url(&url);
        let provider = Arc::new(OllamaProvider::new(config));
        let rt = tokio::runtime::Runtime::new().unwrap();
        let llm = LmrsClient::new();
        rt.block_on(llm.set_custom("ollama", provider));
        rt.block_on(llm.embed("ollama/all-minilm", "hello"))
            .unwrap();
    }

    #[test]
    fn ollama_embed_does_not_send_user() {
        let url = fake_ollama_server(|_m, _p, body| {
            let json: serde_json::Value = serde_json::from_slice(body).unwrap();
            assert!(json.get("user").is_none(), "user should not be sent");
            (200, r#"{"model":"m","embeddings":[[0.1]]}"#.into())
        });
        let config = ProviderConfig::new("").with_base_url(&url);
        let provider = OllamaProvider::new(config);
        let req = EmbeddingRequest::new("m", "hi").with_user("u");
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(provider.embed(&req)).unwrap();
    }

    #[test]
    fn ollama_embed_malformed_response_returns_parse() {
        let url = fake_ollama_server(|_m, _p, _b| (200, r#"not json"#.into()));
        let config = ProviderConfig::new("").with_base_url(&url);
        let provider = OllamaProvider::new(config);
        let req = EmbeddingRequest::new("m", "hi");
        let rt = tokio::runtime::Runtime::new().unwrap();
        let err = rt.block_on(provider.embed(&req)).unwrap_err();
        assert!(matches!(err, LlmError::Parse(_)));
    }

    // ── REA-004O: reasoning gate (red first) ───────────────────

    use crate::types::ThinkingConfig;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Fake server that records how many HTTP connections it accepted.
    /// Each accepted connection runs `handler`; a handler call means the
    /// client made a real network request.
    fn counting_server(
        counter: Arc<AtomicUsize>,
        handler: impl Fn(&str, &str, &[u8]) -> (u16, String) + Send + 'static,
    ) -> String {
        let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{}", addr);
        let barrier = Arc::new(Barrier::new(2));
        let b = Arc::clone(&barrier);

        std::thread::spawn(move || {
            b.wait();
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                counter.fetch_add(1, Ordering::SeqCst);
                let mut buf = vec![0u8; 16384];
                let n = stream.read(&mut buf).unwrap_or(0);
                let raw = String::from_utf8_lossy(&buf[..n]).to_string();
                let first_line = raw.lines().next().unwrap_or("");
                let mut parts = first_line.split_whitespace();
                let method = parts.next().unwrap_or("GET");
                let path = parts.next().unwrap_or("/");
                let body_start = raw.find("\r\n\r\n").map(|i| i + 4).unwrap_or(raw.len());
                let body_bytes = &buf[body_start..n];
                let (status, body) = handler(method, path, body_bytes);
                let resp = format!(
                    "HTTP/1.1 {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    status,
                    body.len(),
                    body
                );
                stream.write_all(resp.as_bytes()).ok();
                stream.flush().ok();
                let _ = stream.read(&mut [0u8; 1]);
            }
        });
        barrier.wait();
        url
    }

    #[test]
    fn ollama_chat_thinking_enabled_returns_unsupported_without_network() {
        let hits = Arc::new(AtomicUsize::new(0));
        let url = counting_server(Arc::clone(&hits), |_m, _p, _b| {
            (
                200,
                r#"{"model":"m","message":{"role":"assistant","content":"hi"}}"#.into(),
            )
        });
        let config = ProviderConfig::new("").with_base_url(&url);
        let provider = OllamaProvider::new(config);
        let req = ChatRequest::new("m", "hi").with_thinking(ThinkingConfig::Enabled {
            budget_tokens: None,
        });
        let rt = tokio::runtime::Runtime::new().unwrap();
        let err = rt.block_on(provider.chat(&req)).unwrap_err();
        assert!(
            matches!(err, LlmError::Unsupported { .. }),
            "Enabled reasoning on chat must fail with Unsupported"
        );
        assert_eq!(
            hits.load(Ordering::SeqCst),
            0,
            "no network call allowed when reasoning is unsupported"
        );
    }

    #[test]
    fn ollama_stream_thinking_enabled_returns_unsupported_without_network() {
        let hits = Arc::new(AtomicUsize::new(0));
        let url = counting_server(Arc::clone(&hits), |_m, _p, _b| {
            (
                200,
                r#"{"model":"m","message":{"role":"assistant","content":"hi"},"done":true}"#.into(),
            )
        });
        let config = ProviderConfig::new("").with_base_url(&url);
        let provider = OllamaProvider::new(config);
        let req = ChatRequest::new("m", "hi").with_thinking(ThinkingConfig::Enabled {
            budget_tokens: None,
        });
        let rt = tokio::runtime::Runtime::new().unwrap();
        let stream = rt.block_on(provider.stream(&req));
        assert!(
            matches!(stream, Err(LlmError::Unsupported { .. })),
            "Enabled reasoning on stream must fail with Unsupported, got: {:?}",
            stream.as_ref().err()
        );
        assert_eq!(
            hits.load(Ordering::SeqCst),
            0,
            "no network call allowed when reasoning is unsupported"
        );
    }

    #[test]
    fn ollama_chat_thinking_disabled_passes_through() {
        let hits = Arc::new(AtomicUsize::new(0));
        let url = counting_server(Arc::clone(&hits), |_m, _p, body| {
            let json: serde_json::Value = serde_json::from_slice(body).unwrap();
            assert!(
                json.get("thinking").is_none(),
                "no thinking field on the wire when Disabled"
            );
            assert!(
                json.get("options").and_then(|o| o.get("think")).is_none(),
                "no options.think on the wire when Disabled"
            );
            (
                200,
                r#"{"model":"m","message":{"role":"assistant","content":"hi"}}"#.into(),
            )
        });
        let config = ProviderConfig::new("").with_base_url(&url);
        let provider = OllamaProvider::new(config);
        let req = ChatRequest::new("m", "hi").with_thinking(ThinkingConfig::Disabled);
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(provider.chat(&req)).unwrap();
        assert!(
            hits.load(Ordering::SeqCst) >= 1,
            "Disabled reasoning must still reach the server"
        );
    }

    #[test]
    fn ollama_chat_thinking_unset_passes_through() {
        let hits = Arc::new(AtomicUsize::new(0));
        let url = counting_server(Arc::clone(&hits), |_m, _p, _b| {
            (
                200,
                r#"{"model":"m","message":{"role":"assistant","content":"hi"}}"#.into(),
            )
        });
        let config = ProviderConfig::new("").with_base_url(&url);
        let provider = OllamaProvider::new(config);
        let req = ChatRequest::new("m", "hi");
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(provider.chat(&req)).unwrap();
        assert!(
            hits.load(Ordering::SeqCst) >= 1,
            "unset reasoning must still reach the server"
        );
    }
}
