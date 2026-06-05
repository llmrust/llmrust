//! HTTP proxy server that exposes llmrust as an OpenAI-compatible API.
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

use crate::{ChatRequest, LlmError, LmrsClient, Message, Role};

// ── ID generation ───────────────────────────────────────────

fn generate_id() -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("chatcmpl-{:016x}", ts)
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ── OpenAI-compatible request/response types ──────────────────

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
}

/// OpenAI-compatible message.
#[derive(Debug, Deserialize)]
pub struct ProxyMessage {
    pub role: String,
    pub content: String,
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

// ── Application state ───────────────────────────────────

/// Shared application state for the proxy router.
#[derive(Clone)]
pub struct AppState {
    pub llm: Arc<LmrsClient>,
}

// ── Router ────────────────────────────────────────────

/// Build the Axum router for the proxy server.
///
/// The router accepts every request without authentication. If you need to
/// expose the proxy on anything other than `localhost`, use
/// [`router_with_auth`] instead.
///
/// Routes:
/// - `POST /v1/chat/completions` — OpenAI-compatible chat endpoint
/// - `GET /health` — health check (not rate-limited, no auth)
///
/// CORS is **permissive** by default (all origins allowed). Tighten this in
/// production by wrapping the returned `Router` with a restrictive layer.
pub fn router(llm: Arc<LmrsClient>) -> Router {
    let state = AppState { llm };
    Router::new()
        .route("/v1/chat/completions", post(handle_chat_completions))
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

// ── Health check ────────────────────────────────────────

/// `GET /health` — simple liveness probe. Returns `{"status":"ok"}`.
async fn health_check() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status": "ok"}))
}

// ── Graceful shutdown ───────────────────────────────────

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

// ── Handler ──────────────────────────────────────────

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

// ── Non-streaming handler ───────────────────────────────

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
                created: unix_timestamp