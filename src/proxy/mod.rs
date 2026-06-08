//! HTTP proxy server that exposes llmrust as an OpenAI-compatible API.
//!
//! Supports both **OpenAI** (`/v1/chat/completions`) and **Anthropic**
//! (`/v1/messages`) protocols, so any SDK client can talk to any registered
//! provider through automatic format conversion.
//!
//! # Usage
//!
//! ```rust,no_run
//! use llmrust::{LmrsClient, proxy};
//! use std::sync::Arc;
//!
//! #[tokio::main]
//! async fn main() {
//!     let llm = Arc::new(LmrsClient::new());
//!     llm.set_openai("sk-...").await;
//!
//!     // Option A: bind and serve yourself (requires manual graceful shutdown)
//!     let app = proxy::router(llm.clone());
//!     let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
//!     axum::serve(listener, app).await.unwrap();
//!
//!     // Option B: use the built-in serve() with graceful shutdown
//!     // proxy::serve(llm, "0.0.0.0:3000").await.unwrap();
//! }
//! ```

use std::convert::Infallible;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    extract::State,
    http::{header, Request, StatusCode},
    middleware::{from_fn, Next},
    response::{
        sse::{Event, Sse},
        IntoResponse, Json, Response,
    },
    routing::{get, post},
    Router,
};
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use tower_http::cors::CorsLayer;

mod anthropic_proxy;

use crate::{
    ChatRequest, Content, LlmError, LmrsClient, Message, Role, Tool, ToolCall, ToolChoice,
};

// ── ID generation ────────────────────────────

fn generate_id() -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let rand = fastrand::u64(..);
    format!("chatcmpl-{:016x}{:08x}", ts, rand)
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ── OpenAI-compatible request/response types ──────────

/// OpenAI-compatible chat completion request.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ProxyChatRequest {
    pub model: String,
    pub messages: Vec<ProxyMessage>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<u64>,
    pub stream: bool,
    pub top_p: Option<f64>,
    /// Tool definitions for function calling (OpenAI protocol).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,
    /// How the model should choose which tool to call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
}

/// OpenAI-compatible message. `content` accepts either a plain string or an
/// array of content parts (text / image_url), matching the OpenAI schema.
#[derive(Debug, Deserialize)]
pub struct ProxyMessage {
    pub role: String,
    pub content: Content,
    /// The id of the tool call this message responds to (present on `tool`
    /// role messages in OpenAI's tool-calling protocol).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Tool calls requested by the assistant (present on assistant turns that
    /// invoke tools).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
}

/// OpenAI-compatible chat completion response (non-streaming).
#[derive(Debug, Serialize)]
pub struct ProxyChatResponse {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<ProxyChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<ProxyUsage>,
}

/// A single choice in a chat completion response.
#[derive(Debug, Serialize)]
pub struct ProxyChoice {
    pub index: u32,
    pub message: ProxyResponseMessage,
    pub finish_reason: String,
}

/// Message inside a choice.
#[derive(Debug, Serialize)]
pub struct ProxyResponseMessage {
    pub role: String,
    pub content: String,
}

/// OpenAI-compatible usage stats.
#[derive(Debug, Serialize)]
pub struct ProxyUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

/// OpenAI-compatible streaming chunk.
#[derive(Debug, Serialize)]
pub struct ProxyStreamChunk {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<ProxyStreamChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<ProxyUsage>,
}

/// A choice in a streaming chunk.
#[derive(Debug, Serialize)]
pub struct ProxyStreamChoice {
    pub index: u32,
    pub delta: ProxyDelta,
    pub finish_reason: Option<String>,
}

/// Delta content in a streaming chunk.
#[derive(Debug, Serialize)]
pub struct ProxyDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

/// Proxy error response body.
#[derive(Debug, Serialize)]
pub struct ProxyError {
    pub error: ProxyErrorDetail,
}

#[derive(Debug, Serialize)]
pub struct ProxyErrorDetail {
    pub message: String,
    #[serde(rename = "type")]
    pub error_type: String,
}

// ── Application state ───────────────────────

/// Shared application state for the proxy router.
#[derive(Clone)]
pub struct AppState {
    pub llm: Arc<LmrsClient>,
}

// ── Router ───────────────────────────

/// Build the Axum router for the proxy server.
///
/// The router accepts every request without authentication. If you need to
/// expose the proxy on anything other than `localhost`, use
/// [`router_with_auth`] instead.
///
/// Routes:
/// - `POST /v1/chat/completions` — OpenAI-compatible chat endpoint
/// - `POST /v1/messages` — Anthropic Messages API endpoint
/// - `GET /health` — health check (not rate-limited, no auth)
///
/// CORS is **permissive** by default (all origins allowed). Tighten this in
/// production by wrapping the returned `Router` with a restrictive layer.
pub fn router(llm: Arc<LmrsClient>) -> Router {
    let state = AppState { llm };
    Router::new()
        .route("/v1/chat/completions", post(handle_chat_completions))
        .route("/v1/messages", post(anthropic_proxy::handle_messages))
        .route("/health", get(health_check))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

/// Build the Axum router for the proxy server with bearer-token authentication.
///
/// Every request must include an `Authorization: Bearer <token>` header whose
/// token matches `expected_token`. Requests with a missing, malformed, or
/// mismatched token receive a `401 Unauthorized` response.
///
/// Note: token comparison uses standard string equality, which is **not**
/// constant-time. This is acceptable when the proxy is reachable only by
/// trusted clients (e.g., behind a reverse proxy). For higher security,
/// consider a reverse proxy with TLS and rate limiting.
pub fn router_with_auth(llm: Arc<LmrsClient>, expected_token: String) -> Router {
    let state = AppState { llm };
    let token = expected_token.clone();
    Router::new()
        .route("/v1/chat/completions", post(handle_chat_completions))
        .route("/v1/messages", post(anthropic_proxy::handle_messages))
        .route("/health", get(health_check))
        .with_state(state)
        .layer(from_fn(move |req, next| {
            let expected = token.clone();
            check_bearer(expected, req, next)
        }))
        .layer(CorsLayer::permissive())
}

/// Bearer-token middleware. Returns 401 if the `Authorization` header is
/// missing, malformed, or carries a token other than `expected`.
async fn check_bearer(expected: String, req: Request<axum::body::Body>, next: Next) -> Response {
    match req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        Some(s) => match s.strip_prefix("Bearer ") {
            Some(provided) if provided.trim() == expected => next.run(req).await,
            Some(_) => error_response(StatusCode::UNAUTHORIZED, "Invalid bearer token"),
            None => error_response(
                StatusCode::UNAUTHORIZED,
                "Authorization header must use Bearer scheme",
            ),
        },
        None => {
            let mut response =
                error_response(StatusCode::UNAUTHORIZED, "Missing Authorization header");
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                "Bearer".parse().expect("static header value parses"),
            );
            response
        }
    }
}

// ── Health check ───────────────────────────

/// `GET /health` — simple liveness probe. Returns `{"status":"ok"}`.
async fn health_check() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status": "ok"}))
}

// ── Graceful shutdown ───────────────────────

/// Shutdown signal listener. Waits for:
/// - `SIGINT` (Ctrl+C) on all platforms
/// - `SIGTERM` on Unix (container orchestration, systemd, etc.)
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

/// Bind to `addr` and serve the proxy with graceful shutdown.
///
/// If the `LLMRUST_PROXY_KEY` environment variable is set and non-empty,
/// bearer-token authentication is required for every request (see
/// [`router_with_auth`]). Otherwise, the proxy accepts all requests
/// without authentication.
///
/// This is a convenience wrapper around the common boot sequence:
///
/// ```rust,ignore
/// let app = router(llm);
/// let listener = tokio::net::TcpListener::bind(addr).await?;
/// axum::serve(listener, app)
///     .with_graceful_shutdown(shutdown_signal())
///     .await
/// ```
pub async fn serve(llm: Arc<LmrsClient>, addr: &str) -> std::io::Result<()> {
    let token = std::env::var("LLMRUST_PROXY_KEY")
        .ok()
        .filter(|s| !s.is_empty());
    let app = match token {
        Some(key) => {
            eprintln!("[auth] enabled — bearer token required");
            router_with_auth(llm, key)
        }
        None => {
            eprintln!("[auth] DISABLED — set LLMRUST_PROXY_KEY=<secret> to require bearer auth");
            router(llm)
        }
    };
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
}

// ── Handler ───────────────────────

/// Handle POST /v1/chat/completions.
async fn handle_chat_completions(
    State(state): State<AppState>,
    Json(req): Json<ProxyChatRequest>,
) -> Response {
    let chat_req = match convert_request(&req) {
        Ok(r) => r,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, &e),
    };

    if req.stream {
        handle_stream(state, &req.model, chat_req).await
    } else {
        handle_non_stream(state, &req.model, chat_req).await
    }
}

// ── Non-streaming handler ─────────────────

async fn handle_non_stream(state: AppState, model: &str, req: ChatRequest) -> Response {
    match state.llm.chat_with(model, req).await {
        Ok(resp) => {
            let (_, model_name) = match split_model(model) {
                Ok(pair) => pair,
                Err(_) => (model, model),
            };
            Json(ProxyChatResponse {
                id: generate_id(),
                object: "chat.completion".to_string(),
                created: unix_timestamp(),
                model: model_name.to_string(),
                choices: vec![ProxyChoice {
                    index: 0,
                    message: ProxyResponseMessage {
                        role: "assistant".to_string(),
                        content: resp.content,
                    },
                    finish_reason: "stop".to_string(),
                }],
                usage: resp.usage.map(|u| ProxyUsage {
                    prompt_tokens: u.prompt_tokens,
                    completion_tokens: u.completion_tokens,
                    total_tokens: u.total_tokens,
                }),
            })
            .into_response()
        }
        Err(e) => proxy_error_from_llm_error(e),
    }
}

// ── Streaming handler ─────────────────────

async fn handle_stream(state: AppState, model: &str, mut req: ChatRequest) -> Response {
    let (provider_name, model_name) = match split_model(model) {
        Ok(pair) => pair,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, e),
    };

    req.model = model_name.to_string();
    req.stream = true;

    let provider = match state.llm.get_provider(provider_name).await {
        Ok(p) => p,
        Err(e) => return proxy_error_from_llm_error(e),
    };

    let stream_result = provider.stream(&req).await;

    match stream_result {
        Ok(inner_stream) => {
            let id = generate_id();
            let created = unix_timestamp();
            let model_clone = model_name.to_string();

            let sse_stream = inner_stream.map(move |chunk_result| match chunk_result {
                Ok(chunk) => {
                    let mut finish_reason = chunk.finish_reason;
                    if finish_reason.is_none() && chunk.done {
                        finish_reason = Some("stop".to_string());
                    }
                    let usage = chunk.usage.map(|u| ProxyUsage {
                        prompt_tokens: u.prompt_tokens,
                        completion_tokens: u.completion_tokens,
                        total_tokens: u.total_tokens,
                    });
                    let payload = ProxyStreamChunk {
                        id: id.clone(),
                        object: "chat.completion.chunk".to_string(),
                        created,
                        model: model_clone.clone(),
                        choices: vec![ProxyStreamChoice {
                            index: 0,
                            delta: ProxyDelta {
                                role: Some("assistant".to_string()),
                                content: Some(chunk.delta),
                            },
                            finish_reason,
                        }],
                        usage,
                    };
                    Ok::<_, Infallible>(
                        Event::default().data(serde_json::to_string(&payload).unwrap_or_default()),
                    )
                }
                Err(e) => {
                    let error_payload = ProxyError {
                        error: ProxyErrorDetail {
                            message: e.to_string(),
                            error_type: "stream_error".to_string(),
                        },
                    };
                    Ok::<_, Infallible>(
                        Event::default()
                            .data(serde_json::to_string(&error_payload).unwrap_or_default()),
                    )
                }
            });

            // Final [DONE] chunk matching OpenAI conventions
            let done = stream::once(async { Ok::<_, Infallible>(Event::default().data("[DONE]")) });

            Sse::new(sse_stream.chain(done))
                .keep_alive(
                    axum::response::sse::KeepAlive::new()
                        .interval(std::time::Duration::from_secs(15))
                        .text("keep-alive"),
                )
                .into_response()
        }
        Err(e) => proxy_error_from_llm_error(e),
    }
}

// ── Helpers ────────────────────

/// Convert a proxy request into an internal `ChatRequest`.
fn convert_request(req: &ProxyChatRequest) -> Result<ChatRequest, String> {
    let messages: Vec<Message> = req
        .messages
        .iter()
        .map(|m| {
            let role = match m.role.as_str() {
                "system" => Role::System,
                "user" => Role::User,
                "assistant" => Role::Assistant,
                "tool" => Role::Tool,
                other => return Err(format!("Unknown role: {}", other)),
            };
            Ok(Message {
                role,
                content: m.content.clone(),
                tool_calls: m.tool_calls.clone(),
                tool_call_id: m.tool_call_id.clone(),
                name: None,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(ChatRequest {
        model: String::new(),
        messages,
        temperature: req.temperature,
        max_tokens: req.max_tokens,
        stream: req.stream,
        top_p: req.top_p,
        tools: req.tools.clone(),
        tool_choice: req.tool_choice.clone(),
        ..Default::default()
    })
}

/// Parse a "provider/model" string into (provider_name, model_name).
fn split_model(model: &str) -> Result<(&str, &str), &'static str> {
    model
        .split_once('/')
        .ok_or("model must be in 'provider/model' format")
}

/// Convert an `LlmError` into an HTTP error response.
fn proxy_error_from_llm_error(e: LlmError) -> Response {
    let (status, message) = match &e {
        LlmError::Api { status, message } => {
            let code = StatusCode::from_u16(*status).unwrap_or(StatusCode::BAD_GATEWAY);
            (code, message.clone())
        }
        LlmError::UnknownProvider(_) => (StatusCode::NOT_FOUND, e.to_string()),
        LlmError::Parse(_) => (StatusCode::BAD_REQUEST, e.to_string()),
        _ => (StatusCode::BAD_GATEWAY, e.to_string()),
    };
    error_response(status, &message)
}

/// Build an HTTP error response with JSON body.
fn error_response(status: StatusCode, message: &str) -> Response {
    let body = ProxyError {
        error: ProxyErrorDetail {
            message: message.to_string(),
            error_type: "invalid_request_error".to_string(),
        },
    };
    (status, Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChatResponse, StreamChunk, Usage};
    use crate::{BoxStream, Provider, Result};
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::ServiceExt;

    /// Mock provider that returns a fixed response, for proxy tests.
    struct MockProvider;

    #[async_trait::async_trait]
    impl Provider for MockProvider {
        async fn chat(&self, _req: &ChatRequest) -> Result<ChatResponse> {
            Ok(ChatResponse {
                content: "mocked reply".to_string(),
                model: "mock-model".to_string(),
                usage: Some(Usage {
                    prompt_tokens: 3,
                    completion_tokens: 5,
                    total_tokens: 8,
                }),
                ..Default::default()
            })
        }

        async fn stream(
            &self,
            _req: &ChatRequest,
        ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
            let chunks: Vec<Result<StreamChunk>> = vec![
                Ok(StreamChunk {
                    delta: "mock ".to_string(),
                    ..Default::default()
                }),
                Ok(StreamChunk {
                    delta: "stream".to_string(),
                    ..Default::default()
                }),
                Ok(StreamChunk {
                    done: true,
                    finish_reason: Some("stop".to_string()),
                    ..Default::default()
                }),
                Ok(StreamChunk {
                    usage: Some(Usage {
                        prompt_tokens: 3,
                        completion_tokens: 5,
                        total_tokens: 8,
                    }),
                    ..Default::default()
                }),
            ];
            Ok(Box::pin(futures::stream::iter(chunks)))
        }
    }

    fn build_request(body: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("failed to build test request")
    }

    #[tokio::test]
    async fn router_can_be_built() {
        let llm = Arc::new(LmrsClient::new());
        // Should not panic.
        let _router = router(llm);
    }

    #[tokio::test]
    async fn non_stream_chat_with_mock_provider() {
        let llm = Arc::new(LmrsClient::new());
        llm.set_custom("mock", Arc::new(MockProvider)).await;
        let app = router(llm);

        let body = serde_json::json!({
            "model": "mock/test-model",
            "messages": [{"role": "user", "content": "hi"}],
        })
        .to_string();

        let response = app
            .oneshot(build_request(&body))
            .await
            .expect("request failed");
        assert_eq!(response.status(), StatusCode::OK);

        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body read failed");
        let json: serde_json::Value =
            serde_json::from_slice(&bytes).expect("body is not valid JSON");

        assert_eq!(json["model"], "test-model");
        assert_eq!(json["object"], "chat.completion");
        assert_eq!(json["choices"][0]["message"]["role"], "assistant");
        assert_eq!(json["choices"][0]["message"]["content"], "mocked reply");
        assert_eq!(json["choices"][0]["finish_reason"], "stop");
        assert_eq!(json["usage"]["prompt_tokens"], 3);
        assert_eq!(json["usage"]["completion_tokens"], 5);
        assert_eq!(json["usage"]["total_tokens"], 8);
    }

    #[tokio::test]
    async fn proxy_accepts_multimodal_content() {
        let llm = Arc::new(LmrsClient::new());
        llm.set_custom("mock", Arc::new(MockProvider)).await;
        let app = router(llm);

        let body = serde_json::json!({
            "model": "mock/test-model",
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": "what is in this image?"},
                    {"type": "image_url", "image_url": {"url": "https://example.com/cat.png"}}
                ]
            }],
        })
        .to_string();

        let response = app
            .oneshot(build_request(&body))
            .await
            .expect("request failed");
        assert_eq!(response.status(), StatusCode::OK);

        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body read failed");
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["choices"][0]["message"]["content"], "mocked reply");
    }

    #[tokio::test]
    async fn stream_forwards_finish_reason_and_usage() {
        let llm = Arc::new(LmrsClient::new());
        llm.set_custom("mock", Arc::new(MockProvider)).await;
        let app = router(llm);

        let body = serde_json::json!({
            "model": "mock/test-model",
            "messages": [{"role": "user", "content": "hi"}],
            "stream": true,
        })
        .to_string();

        let response = app
            .oneshot(build_request(&body))
            .await
            .expect("request failed");
        assert_eq!(response.status(), StatusCode::OK);

        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body read failed");
        let text = String::from_utf8(bytes.to_vec()).expect("stream body is valid UTF-8");

        assert!(
            text.contains("mock "),
            "expected streamed content delta, got: {text}"
        );
        assert!(
            text.contains("\"finish_reason\":\"stop\""),
            "expected finish_reason to be forwarded, got: {text}"
        );
        assert!(
            text.contains("\"total_tokens\":8"),
            "expected usage to be forwarded, got: {text}"
        );
        assert!(
            text.contains("[DONE]"),
            "expected terminal [DONE] marker, got: {text}"
        );
    }

    #[tokio::test]
    async fn unknown_provider_returns_404() {
        let llm = Arc::new(LmrsClient::new());
        let app = router(llm);

        let body = serde_json::json!({
            "model": "openai/gpt-4o",
            "messages": [{"role": "user", "content": "hi"}],
        })
        .to_string();

        let response = app
            .oneshot(build_request(&body))
            .await
            .expect("request failed");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body read failed");
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let message = json["error"]["message"]
            .as_str()
            .expect("error.message is not a string");
        assert!(
            message.contains("openai"),
            "expected error to mention provider name 'openai', got: {}",
            message
        );
    }

    #[tokio::test]
    async fn invalid_model_format_returns_400() {
        let llm = Arc::new(LmrsClient::new());
        let app = router(llm);

        let body = serde_json::json!({
            "model": "no-slash-here",
            "messages": [{"role": "user", "content": "hi"}],
        })
        .to_string();

        let response = app
            .oneshot(build_request(&body))
            .await
            .expect("request failed");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn invalid_role_returns_400() {
        let llm = Arc::new(LmrsClient::new());
        llm.set_custom("mock", Arc::new(MockProvider)).await;
        let app = router(llm);

        let body = serde_json::json!({
            "model": "mock/test",
            "messages": [{"role": "wizard", "content": "hi"}],
        })
        .to_string();

        let response = app
            .oneshot(build_request(&body))
            .await
            .expect("request failed");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body read failed");
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let message = json["error"]["message"]
            .as_str()
            .expect("error.message is not a string");
        assert!(
            message.contains("wizard"),
            "expected error to mention role 'wizard', got: {}",
            message
        );
    }

    #[tokio::test]
    async fn split_model_helper_handles_valid_input() {
        let (provider, model) = split_model("openai/gpt-4o").expect("parse should succeed");
        assert_eq!(provider, "openai");
        assert_eq!(model, "gpt-4o");
    }

    #[tokio::test]
    async fn split_model_helper_rejects_invalid_input() {
        assert!(split_model("nope").is_err());
    }

    // ── Auth middleware tests ──

    const TEST_TOKEN: &str = "secret-token-123";

    fn build_request_with_auth(body: &str, auth_header: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header("content-type", "application/json");
        if let Some(auth) = auth_header {
            builder = builder.header("authorization", auth);
        }
        builder
            .body(Body::from(body.to_string()))
            .expect("failed to build test request")
    }

    #[tokio::test]
    async fn auth_missing_header_returns_401() {
        let llm = Arc::new(LmrsClient::new());
        llm.set_custom("mock", Arc::new(MockProvider)).await;
        let app = router_with_auth(llm, TEST_TOKEN.to_string());

        let body = serde_json::json!({
            "model": "mock/test",
            "messages": [{"role": "user", "content": "hi"}],
        })
        .to_string();

        let response = app
            .oneshot(build_request_with_auth(&body, None))
            .await
            .expect("request failed");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(response.headers().contains_key("www-authenticate"));
    }

    #[tokio::test]
    async fn auth_wrong_token_returns_401() {
        let llm = Arc::new(LmrsClient::new());
        llm.set_custom("mock", Arc::new(MockProvider)).await;
        let app = router_with_auth(llm, TEST_TOKEN.to_string());

        let body = serde_json::json!({
            "model": "mock/test",
            "messages": [{"role": "user", "content": "hi"}],
        })
        .to_string();

        let response = app
            .oneshot(build_request_with_auth(&body, Some("Bearer wrong-token")))
            .await
            .expect("request failed");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn auth_malformed_header_returns_401() {
        let llm = Arc::new(LmrsClient::new());
        llm.set_custom("mock", Arc::new(MockProvider)).await;
        let app = router_with_auth(llm, TEST_TOKEN.to_string());

        let body = serde_json::json!({
            "model": "mock/test",
            "messages": [{"role": "user", "content": "hi"}],
        })
        .to_string();

        // Missing "Bearer " prefix
        let response = app
            .oneshot(build_request_with_auth(&body, Some("Basic dXNlcjpwYXNz")))
            .await
            .expect("request failed");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn auth_valid_token_passes_through() {
        let llm = Arc::new(LmrsClient::new());
        llm.set_custom("mock", Arc::new(MockProvider)).await;
        let app = router_with_auth(llm, TEST_TOKEN.to_string());

        let body = serde_json::json!({
            "model": "mock/test",
            "messages": [{"role": "user", "content": "hi"}],
        })
        .to_string();

        let response = app
            .oneshot(build_request_with_auth(
                &body,
                Some("Bearer secret-token-123"),
            ))
            .await
            .expect("request failed");
        assert_eq!(response.status(), StatusCode::OK);

        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body read failed");
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["choices"][0]["message"]["content"], "mocked reply");
    }

    // ── serve() integration tests ──

    #[tokio::test]
    async fn serve_starts_and_answers_health() {
        let llm = Arc::new(LmrsClient::new());
        let addr = "127.0.0.1:0";
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .expect("bind failed");
        let bound_addr = listener.local_addr().unwrap();

        let app = router(llm);
        tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(std::future::pending())
                .await
                .ok();
        });

        // Give the server a moment to start
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let resp = reqwest::Client::new()
            .get(format!("http://{}/health", bound_addr))
            .send()
            .await
            .expect("health request failed");
        assert_eq!(resp.status(), StatusCode::OK);
        let json: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn serve_with_bearer_requires_auth() {
        let llm = Arc::new(LmrsClient::new());
        llm.set_custom("mock", Arc::new(MockProvider)).await;
        let addr = "127.0.0.1:0";
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .expect("bind failed");
        let bound_addr = listener.local_addr().unwrap();

        let app = router_with_auth(llm, "test-secret".to_string());
        tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(std::future::pending())
                .await
                .ok();
        });

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Without auth — should get 401
        let body = serde_json::json!({
            "model": "mock/test",
            "messages": [{"role": "user", "content": "hi"}],
        })
        .to_string();
        let resp = reqwest::Client::new()
            .post(format!("http://{}/v1/chat/completions", bound_addr))
            .header("content-type", "application/json")
            .body(body.clone())
            .send()
            .await
            .expect("request failed");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        // With auth — should pass
        let resp = reqwest::Client::new()
            .post(format!("http://{}/v1/chat/completions", bound_addr))
            .header("content-type", "application/json")
            .header("authorization", "Bearer test-secret")
            .body(body)
            .send()
            .await
            .expect("request failed");
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn convert_request_handles_tool_role() {
        let raw = serde_json::json!({
            "model": "openai/gpt-4o",
            "messages": [
                {"role": "user", "content": "What's the weather?"},
                {"role": "assistant", "content": "", "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "get_weather", "arguments": "{}"}}]},
                {"role": "tool", "tool_call_id": "call_1", "content": "sunny"}
            ]
        })
        .to_string();
        let req: ProxyChatRequest = serde_json::from_str(&raw).unwrap();
        let chat_req = convert_request(&req).expect("should not fail");
        assert_eq!(chat_req.messages.len(), 3);
        assert_eq!(chat_req.messages[2].role, Role::Tool);
        assert_eq!(chat_req.messages[2].tool_call_id.as_deref(), Some("call_1"));
        assert!(chat_req.messages[1].tool_calls.is_some());
    }

    #[test]
    fn convert_request_forwards_tools_and_tool_choice() {
        let raw = serde_json::json!({
            "model": "openai/gpt-4o",
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{"type": "function", "function": {"name": "f", "parameters": {}}}],
            "tool_choice": "auto"
        })
        .to_string();
        let req: ProxyChatRequest = serde_json::from_str(&raw).unwrap();
        let chat_req = convert_request(&req).expect("should not fail");
        assert!(chat_req.tools.is_some());
        assert!(chat_req.tool_choice.is_some());
    }
}
