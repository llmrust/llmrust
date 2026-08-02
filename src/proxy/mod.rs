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
    extract::{rejection::JsonRejection, DefaultBodyLimit, State},
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
use std::collections::HashMap;

mod anthropic_proxy;

use crate::types::{EmbeddingRequest, EmbeddingUsage, FinishReason};
use crate::{
    BoxStream, ChatRequest, Content, FunctionDef, LlmError, LmrsClient, LogProbs, Message,
    ResponseFormat, Role, StreamChunk, Tool, ToolCall, ToolChoice,
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
    #[serde(alias = "max_completion_tokens")]
    pub max_tokens: Option<u64>,
    pub stream: bool,
    pub top_p: Option<f64>,
    /// Tool definitions for function calling (OpenAI protocol).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,
    /// How the model should choose which tool to call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
    /// Legacy OpenAI function definitions. Converted to modern `tools`
    /// internally when `tools` is not provided.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub functions: Option<Vec<FunctionDef>>,
    /// Legacy OpenAI function-call choice. Converted to modern `tool_choice`
    /// internally when `tool_choice` is not provided.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_call: Option<ProxyFunctionCallChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<ProxyStop>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_logprobs: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<ProxyStreamOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

/// OpenAI-compatible streaming options.
#[derive(Debug, Default, Clone, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ProxyStreamOptions {
    pub include_usage: bool,
}

/// Legacy OpenAI function-call choice (`"auto"`, `"none"`, or
/// `{"name":"function_name"}`).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum ProxyFunctionCallChoice {
    Mode(String),
    Function { name: String },
}

impl ProxyFunctionCallChoice {
    fn to_tool_choice(&self) -> ToolChoice {
        match self {
            ProxyFunctionCallChoice::Mode(mode) => ToolChoice::Mode(mode.clone()),
            ProxyFunctionCallChoice::Function { name } => ToolChoice::function(name),
        }
    }
}

/// OpenAI-compatible stop sequences.
///
/// The OpenAI API accepts either a single string (`"stop": "\n"`) or a list
/// of strings (`"stop": ["END"]`). Internally llmrust stores both as
/// `Vec<String>`.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum ProxyStop {
    One(String),
    Many(Vec<String>),
}

impl ProxyStop {
    fn as_vec(&self) -> Vec<String> {
        match self {
            ProxyStop::One(stop) => vec![stop.clone()],
            ProxyStop::Many(stop) => stop.clone(),
        }
    }
}

/// OpenAI-compatible message. `content` accepts either a plain string or an
/// array of content parts (text / image_url), matching the OpenAI schema. It
/// may also be `null` on assistant turns that only carry `tool_calls`.
#[derive(Debug, Deserialize)]
pub struct ProxyMessage {
    pub role: String,
    #[serde(default)]
    pub content: Option<Content>,
    /// The id of the tool call this message responds to (present on `tool`
    /// role messages in OpenAI's tool-calling protocol).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Tool calls requested by the assistant (present on assistant turns that
    /// invoke tools).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// Optional participant name (often the function name on legacy tool
    /// result messages).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<LogProbs>,
}

/// Message inside a choice.
#[derive(Debug, Serialize)]
pub struct ProxyResponseMessage {
    pub role: String,
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
}

/// OpenAI-compatible usage stats.
#[derive(Debug, Serialize)]
pub struct ProxyUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

impl From<crate::types::Usage> for ProxyUsage {
    fn from(u: crate::types::Usage) -> Self {
        Self {
            prompt_tokens: u.prompt_tokens,
            completion_tokens: u.completion_tokens,
            total_tokens: u.total_tokens,
        }
    }
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
}

// ── OpenAI-compatible embeddings types ────────────────────────────

/// OpenAI-compatible embeddings request.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ProxyEmbeddingRequest {
    pub model: String,
    pub input: ProxyEmbeddingInput,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding_format: Option<String>,
}

/// OpenAI-compatible embeddings input: either a single string or an array.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum ProxyEmbeddingInput {
    One(String),
    Many(Vec<String>),
}

impl Default for ProxyEmbeddingInput {
    fn default() -> Self {
        ProxyEmbeddingInput::Many(Vec::new())
    }
}

impl ProxyEmbeddingInput {
    fn into_vec(self) -> Vec<String> {
        match self {
            ProxyEmbeddingInput::One(s) => vec![s],
            ProxyEmbeddingInput::Many(v) => v,
        }
    }
}

/// OpenAI-compatible embeddings response.
#[derive(Debug, Serialize)]
pub struct ProxyEmbeddingResponse {
    pub object: String,
    pub data: Vec<ProxyEmbedding>,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<ProxyEmbeddingUsage>,
}

/// A single embedding in an OpenAI-compatible response.
#[derive(Debug, Serialize)]
pub struct ProxyEmbedding {
    pub object: String,
    pub index: usize,
    pub embedding: Vec<f32>,
}

/// OpenAI-compatible embeddings usage stats.
#[derive(Debug, Serialize)]
pub struct ProxyEmbeddingUsage {
    pub prompt_tokens: u64,
    pub total_tokens: u64,
}

impl From<EmbeddingUsage> for ProxyEmbeddingUsage {
    fn from(u: EmbeddingUsage) -> Self {
        Self {
            prompt_tokens: u.prompt_tokens,
            total_tokens: u.total_tokens,
        }
    }
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
    pub param: Option<String>,
    pub code: Option<String>,
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
/// - `POST /v1/embeddings` — OpenAI-compatible embeddings endpoint
/// - `GET /health` — health check (not rate-limited, no auth)
///
/// # CORS
///
/// The router sends **no CORS allow-origin header by default** (SPCC §7.1:
/// no-auth default sends no cross-origin allow header; CORS must be enabled
/// explicitly). To allow browser access from specific origins, wrap the
/// returned `Router` with a restrictive `CorsLayer`:
///
/// ```rust,ignore
/// use tower_http::cors::{CorsLayer, AllowOrigin};
/// use axum::http::HeaderValue;
///
/// let app = llmrust::proxy::router(llm)
///     .layer(CorsLayer::new()
///         .allow_origin(AllowOrigin::list(vec![
///             HeaderValue::from_static("https://my-app.example.com"),
///         ])));
/// ```
///
/// `Access-Control-Allow-Origin: *` is only permitted on the authenticated
/// router and only with explicit Owner risk acceptance (SPCC §7.1) — it is
/// never the default.
pub fn router(llm: Arc<LmrsClient>) -> Router {
    let state = AppState { llm };
    Router::new()
        .route("/v1/chat/completions", post(handle_chat_completions))
        .route("/v1/messages", post(anthropic_proxy::handle_messages))
        .route("/v1/embeddings", post(handle_embeddings))
        .route("/health", get(health_check))
        // PRX-005: the handlers enforce the protocol-shaped 413 limit on the
        // raw body; disable axum's default (plain-text 413) so the handlers
        // own the limit path.
        .layer(DefaultBodyLimit::max(usize::MAX))
        .with_state(state)
}

/// Build the Axum router for the proxy server with bearer-token authentication.
///
/// Every request must include an `Authorization: Bearer <token>` header whose
/// token matches `expected_token`. Requests with a missing, malformed, or
/// mismatched token receive a `401 Unauthorized` response.
///
/// Note: token comparison uses constant-time comparison to prevent timing
/// side-channel attacks. For production deployments, also consider a reverse
/// proxy with TLS and rate limiting.
pub fn router_with_auth(llm: Arc<LmrsClient>, expected_token: String) -> Router {
    // PRX-002 (SPCC §7.1): empty or whitespace-only tokens are refused at
    // construction time — they would otherwise silently enable a degenerate
    // auth mode (or downgrade to unauthenticated). Signature stays `String`
    // (API-freeze safe); a panic is the documented behavior for this misuse.
    if expected_token.trim().is_empty() {
        panic!("PRX-002: auth token must not be empty or whitespace-only");
    }
    let state = AppState { llm };
    let token = expected_token;
    let mut router = Router::new()
        .route("/v1/chat/completions", post(handle_chat_completions))
        .route("/v1/messages", post(anthropic_proxy::handle_messages))
        .route("/v1/embeddings", post(handle_embeddings))
        // PRX-005: disable axum's default body limit — the handlers enforce
        // the protocol-shaped 413 limit on the raw body.
        .layer(DefaultBodyLimit::max(usize::MAX))
        .with_state(state)
        .layer(from_fn(move |req, next| {
            let expected = token.clone();
            check_bearer(expected, req, next)
        }));
    // SPCC §7.2: `/health` needs no authentication — register it AFTER the
    // auth layer so the middleware does not cover it (axum layers only apply
    // to routes registered before the layer call).
    router = router.route("/health", get(health_check));
    router
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
            Some(provided)
                if subtle_constant_time_eq(provided.trim().as_bytes(), expected.as_bytes()) =>
            {
                next.run(req).await
            }
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

/// Compare two byte slices in constant time using the reviewed `subtle`
/// crate (SPCC §7.1: use a mature, reviewed constant-time implementation; do
/// not hand-roll cryptographic helpers).
fn subtle_constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    a.ct_eq(b).into()
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
/// Returns `true` when `addr` (e.g. `"127.0.0.1:3000"` or `"[::1]:8080"`)
/// binds only to the loopback interface. Addresses such as `"0.0.0.0"`,
/// `"[::]"`, or a concrete LAN/IP are treated as **public** and must not be
/// served without authentication.
fn is_loopback_addr(addr: &str) -> bool {
    let host = addr.rsplit_once(':').map(|(h, _)| h).unwrap_or(addr);
    let host = host.trim_start_matches('[').trim_end_matches(']');
    matches!(host, "127.0.0.1" | "::1" | "localhost")
}

/// Bind to `addr` and serve the proxy with graceful shutdown.
///
/// Authentication policy (M2-20 — secure by default):
/// - If `LLMRUST_PROXY_KEY` is set and non-empty, bearer-token auth is required.
/// - If no token is set, the proxy is only served when `addr` is a **loopback**
///   address (e.g. `127.0.0.1`, `localhost`, `[::1]`). Binding a non-loopback
///   address without a token is **refused** so the proxy is never silently
///   exposed on a public interface.
pub async fn serve(llm: Arc<LmrsClient>, addr: &str) -> std::io::Result<()> {
    let token = match std::env::var("LLMRUST_PROXY_KEY") {
        // PRX-002 (SPCC §7.1): a set-but-empty/whitespace key must fail at
        // startup — never silently downgrade to unauthenticated, never treat
        // whitespace as a valid secret.
        Ok(s) if s.trim().is_empty() => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "LLMRUST_PROXY_KEY is set but empty or whitespace-only; \
                 refusing to start (SPCC §7.1)",
            ));
        }
        Ok(key) => Some(key.trim().to_string()),
        Err(_) => None,
    };
    let app = match token {
        Some(key) => {
            eprintln!("[auth] enabled — bearer token required");
            router_with_auth(llm, key)
        }
        None => {
            if is_loopback_addr(addr) {
                eprintln!(
                    "[auth] DISABLED — serving UNAUTHENTICATED on loopback {addr}; \
                     set LLMRUST_PROXY_KEY=<secret> to require bearer auth"
                );
                router(llm)
            } else {
                eprintln!(
                    "[auth] REFUSED — refusing to serve UNAUTHENTICATED on non-loopback \
                     address {addr}; set LLMRUST_PROXY_KEY=<secret> to enable auth"
                );
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!(
                        "refusing to serve unauthenticated proxy on non-loopback address {addr}; \
                         set LLMRUST_PROXY_KEY to enable bearer auth"
                    ),
                ));
            }
        }
    };
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
}

// ── Handler ───────────────────────

/// Handle POST /v1/chat/completions.
async fn handle_chat_completions(State(state): State<AppState>, body: String) -> Response {
    // PRX-005 (SPCC §11.6): reject an oversized request body with a
    // protocol-shaped 413 BEFORE any parsing/dispatch (zero upstream).
    if body.len() > proxy_max_body_bytes() {
        return body_too_large_response(false);
    }
    // PRX-003 (SPCC §6.1/§7.3): reasoning-intent keys are not expressible on
    // the OpenAI-compatible proxy — reject them with 400 BEFORE any provider
    // dispatch (zero upstream), instead of silently dropping the field.
    // Parsed as raw Value first so unknown keys stay tolerated (no
    // deny_unknown_fields), then deserialized as before.
    let raw: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("proxy: request JSON extraction failed");
            return invalid_json_response(
                StatusCode::BAD_REQUEST,
                &format!("Invalid JSON request body: {e}"),
            );
        }
    };
    if let Some(obj) = raw.as_object() {
        if obj.contains_key("reasoning_effort")
            || obj.contains_key("reasoning")
            || obj.contains_key("thinking")
        {
            return error_response(
                StatusCode::BAD_REQUEST,
                "reasoning is unsupported on the OpenAI-compatible proxy in 0.1.3; \
                 call the provider directly to use reasoning",
            );
        }
    }
    let req: ProxyChatRequest = match serde_json::from_value(raw) {
        Ok(req) => req,
        Err(e) => {
            tracing::error!("proxy: request JSON extraction failed");
            return invalid_json_response(
                StatusCode::BAD_REQUEST,
                &format!("Invalid JSON request body: {e}"),
            );
        }
    };

    tracing::info!(
        model = &req.model,
        stream = req.stream,
        "proxy: chat completion request"
    );
    let chat_req = match convert_request(&req) {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(
                model = &req.model,
                error_kind = "request_conversion",
                "proxy: request conversion failed"
            );
            return error_response(StatusCode::BAD_REQUEST, &e);
        }
    };

    if req.stream {
        let include_usage = req
            .stream_options
            .as_ref()
            .is_some_and(|opts| opts.include_usage);
        handle_stream(state, &req.model, chat_req, include_usage).await
    } else {
        handle_non_stream(state, &req.model, chat_req).await
    }
}

// ── Non-streaming handler ─────────────────

async fn handle_non_stream(state: AppState, model: &str, req: ChatRequest) -> Response {
    let (_, model_name) = match split_model(model) {
        Ok(pair) => pair,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, e),
    };
    let model_name = model_name.to_string();
    match state.llm.chat_with(model, req).await {
        Ok(resp) => {
            let has_tool_calls = resp.tool_calls.as_ref().is_some_and(|c| !c.is_empty());
            let finish_reason = resp
                .finish_reason
                .unwrap_or(if has_tool_calls {
                    FinishReason::ToolCalls
                } else {
                    FinishReason::Stop
                })
                .as_str()
                .to_string();
            let content = if has_tool_calls && resp.content.is_empty() {
                None
            } else {
                Some(resp.content)
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
                        content,
                        tool_calls: resp.tool_calls,
                    },
                    finish_reason,
                    logprobs: resp.logprobs,
                }],
                usage: resp.usage.map(ProxyUsage::from),
            })
            .into_response()
        }
        Err(e) => proxy_error_from_llm_error(e),
    }
}

// ── Embeddings handler ────────────────────

/// Handle POST /v1/embeddings.
async fn handle_embeddings(
    State(state): State<AppState>,
    payload: std::result::Result<Json<ProxyEmbeddingRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(req) => req,
        Err(e) => {
            tracing::error!("proxy: embeddings JSON extraction failed");
            return json_rejection_response(e);
        }
    };

    // Validate encoding_format
    if let Some(ref fmt) = req.encoding_format {
        if fmt != "float" {
            tracing::error!(
                model = %req.model,
                encoding_format = %fmt,
                "proxy: unsupported encoding_format"
            );
            return error_response_with_type(
                StatusCode::BAD_REQUEST,
                "unsupported encoding_format: only float is supported",
                "invalid_request_error",
            );
        }
    }

    let inputs = req.input.into_vec();
    if inputs.is_empty() {
        return error_response_with_type(
            StatusCode::BAD_REQUEST,
            "input must not be empty",
            "invalid_request_error",
        );
    }

    tracing::info!(
        model = %req.model,
        input_count = inputs.len(),
        "proxy: embedding request"
    );

    // Build EmbeddingRequest
    let mut embed_req = EmbeddingRequest::batch("", inputs);
    if let Some(dim) = req.dimensions {
        embed_req = embed_req.with_dimensions(dim);
    }
    if let Some(user) = req.user {
        embed_req = embed_req.with_user(user);
    }

    match state.llm.embed_with(&req.model, embed_req).await {
        Ok(resp) => {
            let data: Vec<ProxyEmbedding> = resp
                .data
                .into_iter()
                .map(|e| ProxyEmbedding {
                    object: "embedding".into(),
                    index: e.index,
                    embedding: e.embedding,
                })
                .collect();

            let usage = resp.usage.map(ProxyEmbeddingUsage::from);

            Json(ProxyEmbeddingResponse {
                object: "list".into(),
                data,
                model: resp.model,
                usage,
            })
            .into_response()
        }
        Err(e) => proxy_error_from_llm_error(e),
    }
}

// ── Streaming handler ─────────────────────

/// Build an OpenAI-compatible SSE byte stream from inner provider stream.
/// Factored out so tests can drive it directly without a full provider setup.
fn build_openai_sse_response(
    inner_stream: BoxStream<'static, Result<StreamChunk, LlmError>>,
    id: String,
    created: u64,
    model: String,
    include_usage: bool,
) -> Response {
    struct StreamState {
        inner: BoxStream<'static, Result<StreamChunk, LlmError>>,
        id: String,
        created: u64,
        model: String,
        include_usage: bool,
        role_sent: bool,
        terminated: bool,
    }

    let state = StreamState {
        inner: inner_stream,
        id,
        created,
        model,
        include_usage,
        role_sent: false,
        terminated: false,
    };

    let sse_stream = stream::unfold(state, |mut state| async move {
        loop {
            if state.terminated {
                return None;
            }
            match state.inner.next().await {
                Some(Ok(chunk)) => {
                    // PRX-003 (SPCC §6.3/§7.3): reasoning cannot be expressed
                    // on the OpenAI wire — a non-empty thinking delta or
                    // `thinking_done == true` must surface as exactly one
                    // stream_error event (then terminate via the existing
                    // error path), never a silent drop.
                    let has_thinking = matches!(&chunk.thinking, Some(t) if !t.is_empty());
                    if has_thinking || chunk.thinking_done == Some(true) {
                        state.terminated = true;
                        let error_payload = ProxyError {
                            error: ProxyErrorDetail {
                                message:
                                    "reasoning cannot be expressed on the OpenAI wire in 0.1.3; \
                                     use the raw provider stream instead"
                                        .to_string(),
                                error_type: "stream_error".to_string(),
                                param: None,
                                code: None,
                            },
                        };
                        let event = std::result::Result::<_, Infallible>::Ok(
                            Event::default()
                                .data(serde_json::to_string(&error_payload).unwrap_or_default()),
                        );
                        return Some((event, state));
                    }
                    let has_tool_calls = chunk.tool_calls.as_ref().is_some_and(|c| !c.is_empty());
                    let mut finish_reason = chunk.finish_reason;
                    if finish_reason.is_none() && chunk.done {
                        finish_reason = Some(if has_tool_calls {
                            FinishReason::ToolCalls
                        } else {
                            FinishReason::Stop
                        });
                    }
                    let has_usage = chunk.usage.is_some();
                    let usage = if state.include_usage {
                        chunk.usage.map(ProxyUsage::from)
                    } else {
                        None
                    };
                    let content = if chunk.delta.is_empty() {
                        None
                    } else {
                        Some(chunk.delta)
                    };
                    let is_usage_only = has_usage
                        && content.is_none()
                        && finish_reason.is_none()
                        && !has_tool_calls;
                    if is_usage_only && !state.include_usage {
                        continue;
                    }
                    let choices = if is_usage_only {
                        Vec::new()
                    } else {
                        let role = if state.role_sent {
                            None
                        } else {
                            state.role_sent = true;
                            Some("assistant".to_string())
                        };
                        vec![ProxyStreamChoice {
                            index: 0,
                            delta: ProxyDelta {
                                role,
                                content,
                                tool_calls: chunk.tool_calls,
                            },
                            finish_reason: finish_reason.map(|r| r.as_str().to_string()),
                        }]
                    };
                    let payload = ProxyStreamChunk {
                        id: state.id.clone(),
                        object: "chat.completion.chunk".to_string(),
                        created: state.created,
                        model: state.model.clone(),
                        choices,
                        usage,
                    };
                    let event = match serde_json::to_string(&payload) {
                        Ok(json) => {
                            std::result::Result::<_, Infallible>::Ok(Event::default().data(json))
                        }
                        Err(_) => {
                            tracing::error!(
                                model = state.model,
                                "failed to serialize SSE chunk, skipping event"
                            );
                            continue;
                        }
                    };
                    return Some((event, state));
                }
                Some(Err(e)) => {
                    state.terminated = true;
                    let error_payload = ProxyError {
                        error: ProxyErrorDetail {
                            message: e.to_string(),
                            error_type: "stream_error".to_string(),
                            param: None,
                            code: None,
                        },
                    };
                    let event = std::result::Result::<_, Infallible>::Ok(
                        Event::default()
                            .data(serde_json::to_string(&error_payload).unwrap_or_default()),
                    );
                    return Some((event, state));
                }
                None => return None,
            }
        }
    });

    let sse_stream: BoxStream<'static, std::result::Result<Event, Infallible>> =
        Box::pin(sse_stream);

    let done: BoxStream<'static, std::result::Result<Event, Infallible>> =
        Box::pin(stream::once(async {
            std::result::Result::<_, Infallible>::Ok(Event::default().data("[DONE]"))
        }));
    Sse::new(sse_stream.chain(done))
        .keep_alive(
            axum::response::sse::KeepAlive::new()
                .interval(std::time::Duration::from_secs(15))
                .text("keep-alive"),
        )
        .into_response()
}

async fn handle_stream(
    state: AppState,
    model: &str,
    mut req: ChatRequest,
    include_usage: bool,
) -> Response {
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
            build_openai_sse_response(
                inner_stream,
                id,
                created,
                model_name.to_string(),
                include_usage,
            )
        }
        Err(e) => proxy_error_from_llm_error(e),
    }
}

// ── Helpers ────────────────────

/// Convert a proxy request into an internal `ChatRequest`.
fn convert_request(req: &ProxyChatRequest) -> Result<ChatRequest, String> {
    if req.messages.is_empty() {
        return Err("messages must contain at least one message".to_string());
    }

    if let Some(n) = req.n {
        if n != 1 {
            return Err(
                "n must be 1 when using the llmrust proxy; multiple completions are not supported"
                    .to_string(),
            );
        }
    }

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
                content: m.content.clone().unwrap_or_default(),
                tool_calls: m.tool_calls.clone(),
                tool_call_id: m.tool_call_id.clone(),
                name: m.name.clone(),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let tools = match &req.tools {
        Some(tools) => Some(tools.clone()),
        None => req.functions.as_ref().map(|functions| {
            functions
                .iter()
                .cloned()
                .map(|function| Tool {
                    tool_type: "function".to_string(),
                    function,
                })
                .collect()
        }),
    };
    let tool_choice = req.tool_choice.clone().or_else(|| {
        req.function_call
            .as_ref()
            .map(ProxyFunctionCallChoice::to_tool_choice)
    });

    Ok(ChatRequest {
        model: String::new(),
        messages,
        temperature: req.temperature,
        max_tokens: req.max_tokens,
        stream: req.stream,
        top_p: req.top_p,
        tools,
        tool_choice,
        response_format: req.response_format.clone(),
        stop: req.stop.as_ref().map(ProxyStop::as_vec),
        n: req.n,
        seed: req.seed,
        presence_penalty: req.presence_penalty,
        frequency_penalty: req.frequency_penalty,
        logprobs: req.logprobs,
        top_logprobs: req.top_logprobs,
        parallel_tool_calls: req.parallel_tool_calls,
        service_tier: req.service_tier.clone(),
        store: req.store,
        metadata: req.metadata.clone(),
        user: req.user.clone(),
        extra: HashMap::new(),
        ..Default::default()
    })
}

/// Parse a "provider/model" string into (provider_name, model_name).
pub(crate) fn split_model(model: &str) -> Result<(&str, &str), &'static str> {
    let (provider, model) = model
        .split_once('/')
        .ok_or("model must be in 'provider/model' format")?;
    if provider.is_empty() || model.is_empty() {
        return Err("model must be in 'provider/model' format with non-empty provider and model");
    }
    Ok((provider, model))
}

/// Convert an `LlmError` into an HTTP error response.
fn proxy_error_from_llm_error(e: LlmError) -> Response {
    // PRX-005 (SPCC §11.6): normalize upstream errors to protocol-shaped
    // bodies. Message rule: truncate to ≤200 chars as the ONLY mechanical
    // rule (no fragile key/URL stripping heuristics); never echo request body
    // content. Parse is an upstream fault → 502 api_error (architect ruling).
    const MAX_MSG: usize = 200;
    let truncate = |s: &str| -> String { s.chars().take(MAX_MSG).collect() };
    let (status, message, error_type) = match &e {
        LlmError::Api { status, message } => {
            let code = StatusCode::from_u16(*status).unwrap_or(StatusCode::BAD_GATEWAY);
            (code, truncate(message), api_error_type(code))
        }
        LlmError::Parse(_) => (
            StatusCode::BAD_GATEWAY,
            "upstream returned a non-JSON error response".to_string(),
            "api_error",
        ),
        LlmError::Http(_) => (
            StatusCode::BAD_GATEWAY,
            "upstream connection failed".to_string(),
            "api_error",
        ),
        LlmError::UnknownProvider(_) => (
            StatusCode::NOT_FOUND,
            e.to_string(),
            "invalid_request_error",
        ),
        LlmError::Unsupported { .. } => (
            StatusCode::BAD_REQUEST,
            e.to_string(),
            "invalid_request_error",
        ),
        LlmError::Stream(msg) => (StatusCode::BAD_GATEWAY, truncate(msg), "api_error"),
    };
    error_response_with_type(status, &message, error_type)
}

fn api_error_type(status: StatusCode) -> &'static str {
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => "authentication_error",
        StatusCode::TOO_MANY_REQUESTS => "rate_limit_error",
        StatusCode::BAD_REQUEST => "invalid_request_error",
        _ => "api_error",
    }
}

fn json_rejection_response(e: JsonRejection) -> Response {
    let status = if e.status() == StatusCode::UNSUPPORTED_MEDIA_TYPE {
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    } else {
        StatusCode::BAD_REQUEST
    };
    invalid_json_response(status, &format!("Invalid JSON request body: {e}"))
}

/// Build the canonical "Invalid JSON request body" 400 body used by both the
/// extractor rejection path and the PRX-003 raw-Value pre-parse path, so the
/// wire contract stays identical.
fn invalid_json_response(status: StatusCode, message: &str) -> Response {
    error_response_with_type(status, message, "invalid_request_error")
}

/// PRX-005: the proxy request-body limit in bytes, read from
/// `LLMRUST_PROXY_MAX_BODY_BYTES` (SPCC §11.6 PRX-005), defaulting to the
/// axum-ecosystem default of 2 MiB. Both protocols share the same limit.
pub(crate) fn proxy_max_body_bytes() -> usize {
    std::env::var("LLMRUST_PROXY_MAX_BODY_BYTES")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(2 * 1024 * 1024)
}

/// PRX-005: reject an oversized request body with a protocol-shaped 413 JSON
/// error body (SPCC §11.6), zero upstream dispatch. `anthropic_shape` selects
/// the Anthropic error body; otherwise the OpenAI shape is used.
pub(crate) fn body_too_large_response(anthropic_shape: bool) -> Response {
    let msg = "request body exceeds the configured proxy limit \
               (LLMRUST_PROXY_MAX_BODY_BYTES)";
    if anthropic_shape {
        anthropic_proxy::anthropic_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "invalid_request_error",
            msg,
        )
    } else {
        error_response(StatusCode::PAYLOAD_TOO_LARGE, msg)
    }
}

/// Build an HTTP error response with JSON body.
fn error_response(status: StatusCode, message: &str) -> Response {
    error_response_with_type(status, message, "invalid_request_error")
}

fn error_response_with_type(status: StatusCode, message: &str, error_type: &str) -> Response {
    let body = ProxyError {
        error: ProxyErrorDetail {
            message: message.to_string(),
            error_type: error_type.to_string(),
            param: None,
            code: None,
        },
    };
    (status, Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        ChatResponse, Embedding, EmbeddingResponse, FinishReason, FunctionCall, ResponseFormat,
        StreamChunk, Usage,
    };
    use crate::{BoxStream, Provider, Result};
    use async_trait::async_trait;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use std::sync::Mutex;
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
                    ..Default::default()
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
                    finish_reason: Some(FinishReason::Stop),
                    ..Default::default()
                }),
                Ok(StreamChunk {
                    usage: Some(Usage {
                        prompt_tokens: 3,
                        completion_tokens: 5,
                        total_tokens: 8,
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
            ];
            Ok(Box::pin(futures::stream::iter(chunks)))
        }
    }

    struct ToolCallProvider;

    #[async_trait::async_trait]
    impl Provider for ToolCallProvider {
        async fn chat(&self, _req: &ChatRequest) -> Result<ChatResponse> {
            Ok(ChatResponse {
                content: String::new(),
                model: "tool-model".to_string(),
                tool_calls: Some(vec![ToolCall {
                    id: "call_1".to_string(),
                    call_type: "function".to_string(),
                    function: FunctionCall {
                        name: "get_weather".to_string(),
                        arguments: "{\"city\":\"SF\"}".to_string(),
                    },
                }]),
                finish_reason: Some(FinishReason::ToolCalls),
                ..Default::default()
            })
        }

        async fn stream(
            &self,
            _req: &ChatRequest,
        ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
            let chunks: Vec<Result<StreamChunk>> = vec![Ok(StreamChunk {
                done: true,
                finish_reason: Some(FinishReason::ToolCalls),
                tool_calls: Some(vec![ToolCall {
                    id: "call_1".to_string(),
                    call_type: "function".to_string(),
                    function: FunctionCall {
                        name: "get_weather".to_string(),
                        arguments: "{\"city\":\"SF\"}".to_string(),
                    },
                }]),
                ..Default::default()
            })];
            Ok(Box::pin(futures::stream::iter(chunks)))
        }
    }

    struct ApiErrorProvider;

    #[async_trait::async_trait]
    impl Provider for ApiErrorProvider {
        async fn chat(&self, _req: &ChatRequest) -> Result<ChatResponse> {
            Err(LlmError::Api {
                status: StatusCode::TOO_MANY_REQUESTS.as_u16(),
                message: "rate limited".to_string(),
            })
        }

        async fn stream(
            &self,
            _req: &ChatRequest,
        ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
            Err(LlmError::Api {
                status: StatusCode::TOO_MANY_REQUESTS.as_u16(),
                message: "rate limited".to_string(),
            })
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

    fn sse_json_events(text: &str) -> Vec<serde_json::Value> {
        text.lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(str::trim)
            .filter(|data| !data.is_empty() && *data != "[DONE]")
            .map(|data| serde_json::from_str(data).expect("SSE data is valid JSON"))
            .collect()
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
    async fn non_stream_forwards_tool_calls_and_finish_reason() {
        let llm = Arc::new(LmrsClient::new());
        llm.set_custom("tool", Arc::new(ToolCallProvider)).await;
        let app = router(llm);

        let body = serde_json::json!({
            "model": "tool/test-model",
            "messages": [{"role": "user", "content": "weather?"}],
            "tools": [{"type": "function", "function": {"name": "get_weather", "parameters": {}}}]
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

        assert_eq!(json["choices"][0]["finish_reason"], "tool_calls");
        assert!(json["choices"][0]["message"]["content"].is_null());
        assert_eq!(
            json["choices"][0]["message"]["tool_calls"][0]["function"]["name"],
            "get_weather"
        );
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
    async fn proxy_accepts_string_stop_sequence() {
        let llm = Arc::new(LmrsClient::new());
        llm.set_custom("mock", Arc::new(MockProvider)).await;
        let app = router(llm);

        let body = serde_json::json!({
            "model": "mock/test-model",
            "messages": [{"role": "user", "content": "hi"}],
            "stop": "END"
        })
        .to_string();

        let response = app
            .oneshot(build_request(&body))
            .await
            .expect("request failed");
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn malformed_json_returns_proxy_error_body() {
        let llm = Arc::new(LmrsClient::new());
        let app = router(llm);

        let response = app
            .oneshot(build_request("{"))
            .await
            .expect("request failed");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body read failed");
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["error"]["type"], "invalid_request_error");
        assert!(json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Invalid JSON request body"));
        assert!(json["error"]["param"].is_null());
        assert!(json["error"]["code"].is_null());
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
            "stream_options": {"include_usage": true},
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

        let events = sse_json_events(&text);
        assert_eq!(events[0]["choices"][0]["delta"]["role"], "assistant");
        assert_eq!(events[0]["choices"][0]["delta"]["content"], "mock ");
        assert!(
            events[1]["choices"][0]["delta"].get("role").is_none(),
            "role should only be emitted on the first delta: {text}"
        );
        assert_eq!(events[1]["choices"][0]["delta"]["content"], "stream");
        assert_eq!(events[2]["choices"][0]["finish_reason"], "stop");
        assert!(
            events[3]["choices"].as_array().unwrap().is_empty(),
            "usage-only chunk should have empty choices: {text}"
        );
        assert_eq!(events[3]["usage"]["total_tokens"], 8);
    }

    #[tokio::test]
    async fn stream_omits_usage_without_stream_options() {
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
        let events = sse_json_events(&text);

        assert_eq!(
            events.len(),
            3,
            "usage-only event should be skipped: {text}"
        );
        assert!(
            events.iter().all(|event| event.get("usage").is_none()),
            "usage should be omitted unless requested: {text}"
        );
        assert_eq!(events[0]["choices"][0]["delta"]["role"], "assistant");
        assert_eq!(events[2]["choices"][0]["finish_reason"], "stop");
        assert!(
            text.contains("[DONE]"),
            "expected terminal [DONE] marker, got: {text}"
        );
    }

    #[tokio::test]
    async fn stream_forwards_tool_calls() {
        let llm = Arc::new(LmrsClient::new());
        llm.set_custom("tool", Arc::new(ToolCallProvider)).await;
        let app = router(llm);

        let body = serde_json::json!({
            "model": "tool/test-model",
            "messages": [{"role": "user", "content": "weather?"}],
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
        assert!(text.contains("\"finish_reason\":\"tool_calls\""));
        assert!(text.contains("\"tool_calls\""));
        assert!(text.contains("\"name\":\"get_weather\""));
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
    async fn upstream_rate_limit_uses_openai_error_type() {
        let llm = Arc::new(LmrsClient::new());
        llm.set_custom("api", Arc::new(ApiErrorProvider)).await;
        let app = router(llm);

        let body = serde_json::json!({
            "model": "api/test-model",
            "messages": [{"role": "user", "content": "hi"}],
        })
        .to_string();

        let response = app
            .oneshot(build_request(&body))
            .await
            .expect("request failed");
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);

        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body read failed");
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["error"]["message"], "rate limited");
        assert_eq!(json["error"]["type"], "rate_limit_error");
        assert!(json["error"]["param"].is_null());
        assert!(json["error"]["code"].is_null());
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
    async fn empty_messages_returns_400() {
        let llm = Arc::new(LmrsClient::new());
        llm.set_custom("mock", Arc::new(MockProvider)).await;
        let app = router(llm);

        let body = serde_json::json!({
            "model": "mock/test",
            "messages": [],
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
        assert_eq!(json["error"]["type"], "invalid_request_error");
        assert!(json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("messages"));
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
        assert!(split_model("/gpt-4o").is_err());
        assert!(split_model("openai/").is_err());
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

        let resp = reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("noop proxy client")
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
        let resp = reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("noop proxy client")
            .post(format!("http://{}/v1/chat/completions", bound_addr))
            .header("content-type", "application/json")
            .body(body.clone())
            .send()
            .await
            .expect("request failed");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        // With auth — should pass
        let resp = reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("noop proxy client")
            .post(format!("http://{}/v1/chat/completions", bound_addr))
            .header("content-type", "application/json")
            .header("authorization", "Bearer test-secret")
            .body(body)
            .send()
            .await
            .expect("request failed");
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn serve_refuses_unauthenticated_on_public_addr() {
        // Guarantee no token is present for this test.
        std::env::remove_var("LLMRUST_PROXY_KEY");
        let llm = Arc::new(LmrsClient::new());
        // `0.0.0.0` binds every interface (public) — must be refused without a token.
        let result = serve(llm, "0.0.0.0:0").await;
        assert!(
            result.is_err(),
            "serve must refuse to run unauthenticated on a non-loopback address"
        );
    }

    #[test]
    fn convert_request_handles_tool_role() {
        let raw = serde_json::json!({
            "model": "openai/gpt-4o",
            "messages": [
                {"role": "user", "content": "What's the weather?"},
                {"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "get_weather", "arguments": "{}"}}]},
                {"role": "tool", "tool_call_id": "call_1", "name": "get_weather", "content": "sunny"}
            ]
        })
        .to_string();
        let req: ProxyChatRequest = serde_json::from_str(&raw).unwrap();
        let chat_req = convert_request(&req).expect("should not fail");
        assert_eq!(chat_req.messages.len(), 3);
        assert_eq!(chat_req.messages[2].role, Role::Tool);
        assert_eq!(chat_req.messages[2].tool_call_id.as_deref(), Some("call_1"));
        assert_eq!(chat_req.messages[2].name.as_deref(), Some("get_weather"));
        assert!(chat_req.messages[1].tool_calls.is_some());
        assert!(chat_req.messages[1].content.is_empty());
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

    #[test]
    fn convert_request_accepts_legacy_functions_and_function_call() {
        let raw = serde_json::json!({
            "model": "openai/gpt-4o",
            "messages": [{"role": "user", "content": "hi"}],
            "functions": [{
                "name": "get_weather",
                "description": "Get the weather",
                "parameters": {"type": "object"}
            }],
            "function_call": {"name": "get_weather"}
        })
        .to_string();
        let req: ProxyChatRequest = serde_json::from_str(&raw).unwrap();
        let chat_req = convert_request(&req).expect("should not fail");

        let tools = chat_req.tools.expect("legacy functions become tools");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].tool_type, "function");
        assert_eq!(tools[0].function.name, "get_weather");
        assert_eq!(
            tools[0].function.description.as_deref(),
            Some("Get the weather")
        );

        match chat_req
            .tool_choice
            .expect("function_call becomes tool_choice")
        {
            ToolChoice::Function { function, .. } => {
                assert_eq!(function.name, "get_weather");
            }
            other => panic!("expected forced function tool_choice, got {other:?}"),
        }
    }

    #[test]
    fn convert_request_prefers_modern_tools_over_legacy_functions() {
        let raw = serde_json::json!({
            "model": "openai/gpt-4o",
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{
                "type": "function",
                "function": {"name": "modern_tool", "parameters": {}}
            }],
            "tool_choice": "auto",
            "functions": [{
                "name": "legacy_function",
                "parameters": {}
            }],
            "function_call": {"name": "legacy_function"}
        })
        .to_string();
        let req: ProxyChatRequest = serde_json::from_str(&raw).unwrap();
        let chat_req = convert_request(&req).expect("should not fail");

        let tools = chat_req.tools.expect("modern tools present");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].function.name, "modern_tool");
        assert_eq!(chat_req.tool_choice, Some(ToolChoice::auto()));
    }

    #[test]
    fn convert_request_forwards_advanced_openai_fields() {
        let raw = serde_json::json!({
            "model": "openai/gpt-4o",
            "messages": [{"role": "user", "content": "hi"}],
            "response_format": {"type": "json_object"},
            "stop": ["END"],
            "n": 1,
            "seed": 42,
            "presence_penalty": 0.5,
            "frequency_penalty": -0.25,
            "logprobs": true,
            "top_logprobs": 3,
            "max_completion_tokens": 64,
            "parallel_tool_calls": false,
            "service_tier": "flex",
            "store": true,
            "metadata": {"trace_id": "abc"},
            "user": "user-123"
        })
        .to_string();
        let req: ProxyChatRequest = serde_json::from_str(&raw).unwrap();
        let chat_req = convert_request(&req).expect("should not fail");

        assert_eq!(chat_req.response_format, Some(ResponseFormat::JsonObject));
        assert_eq!(chat_req.stop, Some(vec!["END".to_string()]));
        assert_eq!(chat_req.n, Some(1));
        assert_eq!(chat_req.seed, Some(42));
        assert_eq!(chat_req.presence_penalty, Some(0.5));
        assert_eq!(chat_req.frequency_penalty, Some(-0.25));
        assert_eq!(chat_req.logprobs, Some(true));
        assert_eq!(chat_req.top_logprobs, Some(3));
        assert_eq!(chat_req.max_tokens, Some(64));
        assert_eq!(chat_req.parallel_tool_calls, Some(false));
        assert_eq!(chat_req.service_tier.as_deref(), Some("flex"));
        assert_eq!(chat_req.store, Some(true));
        assert_eq!(
            chat_req.metadata,
            Some(serde_json::json!({"trace_id": "abc"}))
        );
        assert_eq!(chat_req.user.as_deref(), Some("user-123"));
    }

    #[test]
    fn convert_request_accepts_string_stop_sequence() {
        let raw = serde_json::json!({
            "model": "openai/gpt-4o",
            "messages": [{"role": "user", "content": "hi"}],
            "stop": "END"
        })
        .to_string();
        let req: ProxyChatRequest = serde_json::from_str(&raw).unwrap();
        let chat_req = convert_request(&req).expect("should not fail");
        assert_eq!(chat_req.stop, Some(vec!["END".to_string()]));
    }

    #[test]
    fn convert_request_rejects_empty_messages() {
        let req = ProxyChatRequest {
            model: "openai/gpt-4o".to_string(),
            ..Default::default()
        };
        let err = convert_request(&req).expect_err("empty messages should fail");
        assert!(err.contains("messages"));
    }

    #[test]
    fn convert_request_accepts_missing_n() {
        let raw = r#"{"model":"openai/gpt-4o","messages":[{"role":"user","content":"hi"}]}"#;
        let req: ProxyChatRequest = serde_json::from_str(raw).unwrap();
        let chat_req = convert_request(&req).expect("missing n should be accepted");
        assert_eq!(chat_req.n, None);
    }

    #[test]
    fn convert_request_accepts_n_one() {
        let raw = r#"{"model":"openai/gpt-4o","messages":[{"role":"user","content":"hi"}],"n":1}"#;
        let req: ProxyChatRequest = serde_json::from_str(raw).unwrap();
        let chat_req = convert_request(&req).expect("n=1 should be accepted");
        assert_eq!(chat_req.n, Some(1));
    }

    #[test]
    fn convert_request_rejects_n_greater_than_one() {
        let raw = r#"{"model":"openai/gpt-4o","messages":[{"role":"user","content":"hi"}],"n":2}"#;
        let req: ProxyChatRequest = serde_json::from_str(raw).unwrap();
        let err = convert_request(&req).expect_err("n>1 should be rejected");
        assert!(err.contains("n"), "error should mention n: {err}");
        assert!(
            err.contains("multiple") || err.contains("not supported"),
            "error should explain limitation: {err}"
        );
    }

    #[test]
    fn convert_request_rejects_n_zero() {
        let raw = r#"{"model":"openai/gpt-4o","messages":[{"role":"user","content":"hi"}],"n":0}"#;
        let req: ProxyChatRequest = serde_json::from_str(raw).unwrap();
        let err = convert_request(&req).expect_err("n=0 should be rejected");
        assert!(err.contains("n"), "error should mention n: {err}");
    }

    // ── OpenAI proxy stream error tests ───────────────

    async fn collect_sse(resp: Response) -> String {
        let bytes = to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("body read");
        String::from_utf8(bytes.to_vec()).expect("valid UTF-8")
    }

    #[tokio::test]
    async fn openai_stream_error_event_is_emitted_before_done() {
        let inner = Box::pin(futures::stream::iter([
            Ok(StreamChunk {
                delta: "hello".to_string(),
                ..Default::default()
            }),
            Err(LlmError::Stream("upstream disconnected".to_string())),
        ]));
        let sse = build_openai_sse_response(inner, "id1".into(), 0, "m".into(), true);
        let text = collect_sse(sse).await;
        let err_pos = text
            .find("stream_error")
            .expect("stream_error must be emitted");
        let done_pos = text.find("[DONE]").expect("DONE must be emitted");
        assert!(err_pos < done_pos, "stream_error must appear before [DONE]");
    }

    #[tokio::test]
    async fn openai_stream_error_stops_after_first_error() {
        let inner = Box::pin(futures::stream::iter([
            Ok(StreamChunk {
                delta: "before".to_string(),
                ..Default::default()
            }),
            Err(LlmError::Stream("boom".to_string())),
            Ok(StreamChunk {
                delta: "after".to_string(),
                ..Default::default()
            }),
        ]));
        let sse = build_openai_sse_response(inner, "id2".into(), 0, "m".into(), true);
        let text = collect_sse(sse).await;
        assert!(text.contains("before"));
        assert!(text.contains("stream_error"));
        assert!(
            !text.contains("after"),
            "must not consume chunks after error"
        );
    }

    #[tokio::test]
    async fn openai_stream_normal_text_still_emits_role_and_done() {
        let inner = Box::pin(futures::stream::iter([
            Ok(StreamChunk {
                delta: "hello".to_string(),
                ..Default::default()
            }),
            Ok(StreamChunk {
                delta: " world".to_string(),
                done: true,
                finish_reason: Some(FinishReason::Stop),
                ..Default::default()
            }),
        ]));
        let sse = build_openai_sse_response(inner, "id3".into(), 0, "m".into(), false);
        let text = collect_sse(sse).await;
        assert!(text.contains(r#""role":"assistant""#));
        assert_eq!(
            text.matches(r#""role":"assistant""#).count(),
            1,
            "role must appear only once"
        );
        assert!(text.contains("hello"));
        assert!(text.contains(" world"));
        assert!(text.contains("[DONE]"));
    }

    #[tokio::test]
    async fn openai_stream_usage_only_emits_when_requested() {
        let inner = Box::pin(futures::stream::iter([
            Ok(StreamChunk {
                delta: "hi".to_string(),
                done: true,
                finish_reason: Some(FinishReason::Stop),
                ..Default::default()
            }),
            Ok(StreamChunk {
                usage: Some(Usage {
                    prompt_tokens: 10,
                    completion_tokens: 5,
                    total_tokens: 15,
                    ..Default::default()
                }),
                ..Default::default()
            }),
        ]));
        let sse = build_openai_sse_response(inner, "id4".into(), 0, "m".into(), true);
        let text = collect_sse(sse).await;
        assert!(text.contains(r#""prompt_tokens":10"#));
    }

    #[tokio::test]
    async fn openai_stream_usage_only_skipped_when_not_requested() {
        let inner = Box::pin(futures::stream::iter([
            Ok(StreamChunk {
                usage: Some(Usage {
                    prompt_tokens: 10,
                    completion_tokens: 5,
                    total_tokens: 15,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            Ok(StreamChunk {
                delta: "next".to_string(),
                done: true,
                finish_reason: Some(FinishReason::Stop),
                ..Default::default()
            }),
        ]));
        let sse = build_openai_sse_response(inner, "id5".into(), 0, "m".into(), false);
        let text = collect_sse(sse).await;
        assert!(!text.contains("\"usage\""));
        assert!(text.contains("next"));
        assert!(text.contains("[DONE]"));
    }

    #[tokio::test]
    async fn openai_stream_tool_calls_still_serialize() {
        let inner = Box::pin(futures::stream::iter([Ok(StreamChunk {
            tool_calls: Some(vec![ToolCall {
                id: "call_0".to_string(),
                call_type: "function".to_string(),
                function: FunctionCall {
                    name: "lookup".to_string(),
                    arguments: r#"{"q":"rust"}"#.to_string(),
                },
            }]),
            finish_reason: Some(FinishReason::ToolCalls),
            ..Default::default()
        })]));
        let sse = build_openai_sse_response(inner, "id6".into(), 0, "m".into(), false);
        let text = collect_sse(sse).await;
        assert!(text.contains("tool_calls"));
        assert!(text.contains("call_0"));
        assert!(text.contains("lookup"));
    }

    // ── proxy embeddings tests ───────────────────────────────────

    struct EmbedMockProvider {
        captured: Arc<Mutex<Option<EmbeddingRequest>>>,
    }

    #[async_trait]
    impl Provider for EmbedMockProvider {
        async fn chat(&self, _: &ChatRequest) -> Result<ChatResponse> {
            unimplemented!()
        }
        async fn stream(&self, _: &ChatRequest) -> Result<BoxStream<'static, Result<StreamChunk>>> {
            unimplemented!()
        }
        async fn embed(&self, req: &EmbeddingRequest) -> Result<EmbeddingResponse> {
            *self.captured.lock().unwrap() = Some(req.clone());
            Ok(EmbeddingResponse {
                model: req.model.clone(),
                data: req
                    .input
                    .iter()
                    .enumerate()
                    .map(|(i, _)| Embedding {
                        index: i,
                        embedding: vec![1.0_f32],
                    })
                    .collect(),
                usage: Some(EmbeddingUsage {
                    prompt_tokens: req.input.len() as u64,
                    total_tokens: req.input.len() as u64,
                }),
            })
        }
    }

    fn embed_body(model: &str, input: serde_json::Value) -> Request<Body> {
        let body = serde_json::json!({
            "model": model,
            "input": input
        });
        Request::builder()
            .method("POST")
            .uri("/v1/embeddings")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("build request")
    }

    #[tokio::test]
    async fn proxy_embeddings_route_returns_openai_shape() {
        let llm = Arc::new(LmrsClient::new());
        llm.set_custom(
            "fake",
            Arc::new(EmbedMockProvider {
                captured: Arc::new(Mutex::new(None)),
            }),
        )
        .await;
        let app = router(llm);

        let resp = app
            .oneshot(embed_body("fake/text-emb", serde_json::json!("hello")))
            .await
            .expect("request failed");

        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["object"], "list");
        assert_eq!(json["data"][0]["object"], "embedding");
        assert_eq!(json["data"][0]["index"], 0);
        assert_eq!(json["data"][0]["embedding"][0], 1.0);
        assert_eq!(json["model"], "text-emb");
        assert!(json["usage"]["prompt_tokens"].as_u64().unwrap() > 0);
    }

    #[tokio::test]
    async fn proxy_embeddings_accepts_batch() {
        let llm = Arc::new(LmrsClient::new());
        llm.set_custom(
            "fake",
            Arc::new(EmbedMockProvider {
                captured: Arc::new(Mutex::new(None)),
            }),
        )
        .await;
        let app = router(llm);

        let resp = app
            .oneshot(embed_body("fake/m", serde_json::json!(["a", "b", "c"])))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["data"].as_array().unwrap().len(), 3);
        assert_eq!(json["data"][0]["index"], 0);
        assert_eq!(json["data"][2]["index"], 2);
    }

    #[tokio::test]
    async fn proxy_embeddings_forwards_dimensions_and_user() {
        let llm = Arc::new(LmrsClient::new());
        let captured = Arc::new(Mutex::new(None));
        llm.set_custom(
            "fake",
            Arc::new(EmbedMockProvider {
                captured: Arc::clone(&captured),
            }),
        )
        .await;
        let app = router(llm);

        let body = serde_json::json!({
            "model": "fake/m",
            "input": "hi",
            "dimensions": 1024,
            "user": "user-1"
        });
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/embeddings")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = captured.lock().unwrap();
        let req = req.as_ref().unwrap();
        assert_eq!(req.dimensions, Some(1024));
        assert_eq!(req.user.as_deref(), Some("user-1"));
    }

    #[tokio::test]
    async fn proxy_embeddings_rejects_base64_encoding_format() {
        let llm = Arc::new(LmrsClient::new());
        let app = router(llm);

        let body = serde_json::json!({
            "model": "fake/m",
            "input": "hi",
            "encoding_format": "base64"
        });
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/embeddings")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["error"]["type"], "invalid_request_error");
        assert!(json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("encoding_format"));
    }

    #[tokio::test]
    async fn proxy_embeddings_rejects_empty_input_array() {
        let llm = Arc::new(LmrsClient::new());
        let app = router(llm);

        let body = serde_json::json!({
            "model": "fake/m",
            "input": []
        });
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/embeddings")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn proxy_embeddings_unknown_provider_maps_404() {
        let llm = Arc::new(LmrsClient::new());
        let app = router(llm);

        let resp = app
            .oneshot(embed_body("missing/text-emb", serde_json::json!("hi")))
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["error"]["type"], "invalid_request_error");
    }

    #[tokio::test]
    async fn proxy_embeddings_unsupported_maps_invalid_request() {
        struct ChatOnly;
        #[async_trait]
        impl Provider for ChatOnly {
            async fn chat(&self, _: &ChatRequest) -> Result<ChatResponse> {
                Ok(ChatResponse::default())
            }
            async fn stream(
                &self,
                _: &ChatRequest,
            ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
                Ok(Box::pin(stream::empty()))
            }
            // embed() not overridden → default Unsupported
        }
        let llm = Arc::new(LmrsClient::new());
        llm.set_custom("no-emb", Arc::new(ChatOnly)).await;
        let app = router(llm);

        let resp = app
            .oneshot(embed_body("no-emb/m", serde_json::json!("hi")))
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["error"]["type"], "invalid_request_error");
        assert!(json["error"]["message"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("unsupported"));
    }

    #[tokio::test]
    async fn proxy_embeddings_auth_required_with_router_with_auth() {
        let llm = Arc::new(LmrsClient::new());
        llm.set_custom(
            "fake",
            Arc::new(EmbedMockProvider {
                captured: Arc::new(Mutex::new(None)),
            }),
        )
        .await;
        let app = router_with_auth(llm, "secret".into());

        // No auth → 401
        let resp = app
            .clone()
            .oneshot(embed_body("fake/m", serde_json::json!("hi")))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        // Correct auth → 200
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/embeddings")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer secret")
                    .body(Body::from(
                        serde_json::json!({"model":"fake/m","input":"hi"}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn proxy_embeddings_invalid_json_returns_json_error() {
        let llm = Arc::new(LmrsClient::new());
        let app = router(llm);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/embeddings")
                    .header("content-type", "application/json")
                    .body(Body::from("not json"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["error"]["type"], "invalid_request_error");
    }

    #[tokio::test]
    async fn proxy_embeddings_model_prefix_is_stripped() {
        let llm = Arc::new(LmrsClient::new());
        let captured = Arc::new(Mutex::new(None));
        llm.set_custom(
            "fake",
            Arc::new(EmbedMockProvider {
                captured: Arc::clone(&captured),
            }),
        )
        .await;
        let app = router(llm);

        let resp = app
            .oneshot(embed_body("fake/text-embedding", serde_json::json!("hi")))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = captured.lock().unwrap();
        assert_eq!(req.as_ref().unwrap().model, "text-embedding");
    }

    // ── PRX-001: CORS / listen-address security (red first) ──────────

    fn build_cors_request(origin: &str, path: &str) -> Request<Body> {
        Request::builder()
            .method("GET")
            .uri(path)
            .header("origin", origin)
            .body(Body::empty())
            .expect("failed to build CORS test request")
    }

    /// The unauthenticated router must NOT send a CORS allow-origin header
    /// by default (SPCC §7.1: no-auth default sends no cross-origin allow
    /// header). Currently `default_cors()` sets `allow_origin(Any)`, so a
    /// request with an `Origin` header gets an `Access-Control-Allow-Origin`
    /// header back — this test is RED until the default is removed.
    #[tokio::test]
    async fn unauthenticated_router_sends_no_cors_allow_origin() {
        let llm = Arc::new(LmrsClient::new());
        let app = router(llm);

        let response = app
            .oneshot(build_cors_request("https://evil.example.com", "/health"))
            .await
            .expect("request failed");

        assert_eq!(
            response.headers().get("access-control-allow-origin"),
            None,
            "unauthenticated router must not send Access-Control-Allow-Origin (SPCC §7.1)"
        );
    }

    /// The authenticated router must NOT default to `*` either — CORS must be
    /// enabled explicitly by the caller (SPCC §7.1: `*` only with auth +
    /// explicit Owner risk acceptance). Currently `router_with_auth` also
    /// applies `default_cors()` with `allow_origin(Any)` — RED until removed.
    #[tokio::test]
    async fn authenticated_router_sends_no_default_cors_allow_origin() {
        let llm = Arc::new(LmrsClient::new());
        let app = router_with_auth(llm, "secret".to_string());

        let response = app
            .oneshot(build_cors_request("https://evil.example.com", "/health"))
            .await
            .expect("request failed");

        assert_eq!(
            response.headers().get("access-control-allow-origin"),
            None,
            "authenticated router must not default to Access-Control-Allow-Origin: * (SPCC §7.1)"
        );
    }

    /// When the caller explicitly mounts an allowlist, only the listed origin
    /// may be echoed back. Currently the router applies `allow_origin(Any)`
    /// internally, so a non-listed origin also receives
    /// `Access-Control-Allow-Origin` — RED until the default is removed and
    /// the caller's layer takes effect.
    #[tokio::test]
    async fn explicit_allowlist_rejects_non_listed_origin() {
        use axum::http::HeaderValue;
        use tower_http::cors::{AllowOrigin, CorsLayer};

        let llm = Arc::new(LmrsClient::new());
        let app = router(llm).layer(CorsLayer::new().allow_origin(AllowOrigin::list(vec![
            HeaderValue::from_static("https://trusted.example.com"),
        ])));

        let response = app
            .oneshot(build_cors_request("https://evil.example.com", "/health"))
            .await
            .expect("request failed");

        let allow = response
            .headers()
            .get("access-control-allow-origin")
            .map(|v| v.to_str().unwrap_or(""));
        assert_ne!(
            allow,
            Some("https://evil.example.com"),
            "non-listed origin must not be allowed by an explicit allowlist"
        );
    }

    /// Loopback / non-loopback × token present / absent startup semantics
    /// (SPCC §7.1: no-auth only binds loopback). This is currently GREEN
    /// (the guard exists) — a regression lock, not a red-first test.
    #[test]
    fn loopback_policy_matrix() {
        // no token + loopback → allowed
        assert!(is_loopback_addr("127.0.0.1:3000"));
        assert!(is_loopback_addr("localhost:3000"));
        assert!(is_loopback_addr("[::1]:3000"));
        // no token + non-loopback → refused (guard in serve())
        assert!(!is_loopback_addr("0.0.0.0:3000"));
        assert!(!is_loopback_addr("[::]:3000"));
        assert!(!is_loopback_addr("192.168.1.10:3000"));
    }

    // ── PRX-002: auth / constant-time / /health contract (red first) ──

    /// The authenticated router must NOT gate `/health` behind auth (SPCC
    /// §7.2: `/health` needs no authentication). Currently the `check_bearer`
    /// layer wraps every route including `/health`, so a token-less health
    /// probe gets 401 — RED until the route is mounted after the auth layer.
    #[tokio::test]
    async fn auth_router_health_accessible_without_token() {
        let llm = Arc::new(LmrsClient::new());
        let app = router_with_auth(llm, "test-secret".to_string());

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/health")
                    .body(Body::empty())
                    .expect("failed to build health request"),
            )
            .await
            .expect("request failed");

        assert_eq!(
            response.status(),
            StatusCode::OK,
            "/health must be accessible without a token under auth (SPCC §7.2)"
        );
    }

    /// `/health` with a wrong token must still be 200 (auth is bypassed for
    /// health entirely — not just token-less).
    #[tokio::test]
    async fn auth_router_health_accessible_with_wrong_token() {
        let llm = Arc::new(LmrsClient::new());
        let app = router_with_auth(llm, "test-secret".to_string());

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/health")
                    .header("authorization", "Bearer wrong-token")
                    .body(Body::empty())
                    .expect("failed to build health request"),
            )
            .await
            .expect("request failed");

        assert_eq!(
            response.status(),
            StatusCode::OK,
            "/health with a wrong token must still be 200 (SPCC §7.2)"
        );
    }

    /// `router_with_auth` must reject a blank / whitespace-only token at
    /// construction time (SPCC §7.1: empty or whitespace tokens fail at
    /// startup). Currently it accepts any string — RED until the guard panics.
    #[test]
    #[should_panic(expected = "must not be empty or whitespace-only")]
    fn router_with_auth_rejects_blank_token() {
        let llm = Arc::new(LmrsClient::new());
        let _app = router_with_auth(llm, "   ".to_string());
    }

    /// `router_with_auth` must reject an empty token too.
    #[test]
    #[should_panic(expected = "must not be empty or whitespace-only")]
    fn router_with_auth_rejects_empty_token() {
        let llm = Arc::new(LmrsClient::new());
        let _app = router_with_auth(llm, String::new());
    }

    /// `serve()` must refuse to start when `LLMRUST_PROXY_KEY` is set but
    /// empty or whitespace-only (SPCC §7.1). Currently `.filter(!is_empty)`
    /// treats a whitespace token as valid and an empty one as unauthenticated
    /// — so serve() starts (and blocks on the listener) instead of failing
    /// fast. RED until the guard returns an io error promptly.
    #[tokio::test]
    async fn serve_rejects_blank_or_empty_key() {
        for key in ["", "   "] {
            std::env::set_var("LLMRUST_PROXY_KEY", key);
            let llm = Arc::new(LmrsClient::new());
            // A correct implementation returns Err promptly without binding.
            // The current code instead starts serving and blocks, so the
            // timeout fires → the test fails (RED) with the "must refuse"
            // assertion message.
            let outcome =
                tokio::time::timeout(std::time::Duration::from_secs(2), serve(llm, "127.0.0.1:0"))
                    .await;
            match outcome {
                Ok(Ok(_)) => {
                    std::env::remove_var("LLMRUST_PROXY_KEY");
                    panic!("serve started successfully with set-but-empty/whitespace key ({key:?})")
                }
                Ok(Err(_)) => { /* correct: refused to start */ }
                Err(_) => {
                    std::env::remove_var("LLMRUST_PROXY_KEY");
                    panic!(
                        "serve blocked (started serving) instead of refusing \
                         set-but-empty/whitespace key ({key:?})"
                    )
                }
            }
            std::env::remove_var("LLMRUST_PROXY_KEY");
        }
    }

    /// Token correctness matrix: valid token passes, any mutation (different
    /// length or content) yields 401. Timing is NOT asserted (flaky); only
    /// correctness. Also guards the unchanged 401 semantics + WWW-Authenticate
    /// header (conservation anchor).
    #[tokio::test]
    async fn auth_token_correctness_matrix() {
        let llm = Arc::new(LmrsClient::new());
        llm.set_custom("mock", Arc::new(MockProvider)).await;
        let app = router_with_auth(llm, "test-secret".to_string());
        let body = serde_json::json!({
            "model": "mock/test",
            "messages": [{"role": "user", "content": "hi"}],
        })
        .to_string();

        // valid → 200
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer test-secret")
                    .body(Body::from(body.clone()))
                    .expect("failed to build request"),
            )
            .await
            .expect("request failed");
        assert_eq!(resp.status(), StatusCode::OK);

        // wrong content, same length → 401
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer test-secreX")
                    .body(Body::from(body.clone()))
                    .expect("failed to build request"),
            )
            .await
            .expect("request failed");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        // shorter token → 401
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer test")
                    .body(Body::from(body.clone()))
                    .expect("failed to build request"),
            )
            .await
            .expect("request failed");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        // missing header → 401 + WWW-Authenticate preserved
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .expect("failed to build request"),
            )
            .await
            .expect("request failed");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            resp.headers()
                .get("www-authenticate")
                .and_then(|v| v.to_str().ok()),
            Some("Bearer"),
            "WWW-Authenticate header must be preserved"
        );
    }

    // ── PRX-003: reasoning request rejection + delta guard (red first) ──

    fn build_json_request(body: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("failed to build test request")
    }

    /// A request carrying a reasoning-intent key must be rejected with 400
    /// BEFORE any provider dispatch (SPCC §6.1/§7.3: no silent ignore, no
    /// lossless path → explicit Unsupported). Currently serde silently drops
    /// the key and the request is forwarded → RED until the handler checks the
    /// raw JSON first.
    #[tokio::test]
    async fn reasoning_request_rejected_400_reasoning_effort() {
        let llm = Arc::new(LmrsClient::new());
        llm.set_custom("mock", Arc::new(MockProvider)).await;
        let app = router(llm);
        let body = serde_json::json!({
            "model": "mock/test-model",
            "messages": [{"role": "user", "content": "hi"}],
            "reasoning_effort": "high",
        })
        .to_string();

        let response = app
            .oneshot(build_json_request(&body))
            .await
            .expect("request failed");
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "reasoning_effort must be rejected with 400 (SPCC §6.1/§7.3)"
        );
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["error"]["type"], "invalid_request_error");
        assert!(json["error"]["message"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("reasoning"));
    }

    #[tokio::test]
    async fn reasoning_request_rejected_400_reasoning_field() {
        let llm = Arc::new(LmrsClient::new());
        llm.set_custom("mock", Arc::new(MockProvider)).await;
        let app = router(llm);
        let body = serde_json::json!({
            "model": "mock/test-model",
            "messages": [{"role": "user", "content": "hi"}],
            "reasoning": {"budget": 100},
        })
        .to_string();

        let response = app
            .oneshot(build_json_request(&body))
            .await
            .expect("request failed");
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "reasoning field must be rejected with 400 (SPCC §6.1/§7.3)"
        );
    }

    #[tokio::test]
    async fn reasoning_request_rejected_400_thinking_field() {
        let llm = Arc::new(LmrsClient::new());
        llm.set_custom("mock", Arc::new(MockProvider)).await;
        let app = router(llm);
        let body = serde_json::json!({
            "model": "mock/test-model",
            "messages": [{"role": "user", "content": "hi"}],
            "thinking": {"type": "enabled"},
        })
        .to_string();

        let response = app
            .oneshot(build_json_request(&body))
            .await
            .expect("request failed");
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "thinking field must be rejected with 400 (SPCC §6.1/§7.3)"
        );
    }

    /// Control: a request WITHOUT reasoning keys still passes (200) — the
    /// rejection must not disturb normal requests.
    #[tokio::test]
    async fn normal_request_not_affected_by_reasoning_rejection() {
        let llm = Arc::new(LmrsClient::new());
        llm.set_custom("mock", Arc::new(MockProvider)).await;
        let app = router(llm);
        let body = serde_json::json!({
            "model": "mock/test-model",
            "messages": [{"role": "user", "content": "hi"}],
        })
        .to_string();

        let response = app
            .oneshot(build_json_request(&body))
            .await
            .expect("request failed");
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// An inner chunk carrying a non-empty `thinking` delta must produce
    /// exactly one `stream_error` event followed by `[DONE]` — never a silent
    /// drop (SPCC §6.3/§7.3). Currently the unfold loop never reads
    /// `chunk.thinking`, so no error event is emitted → RED.
    #[tokio::test]
    async fn thinking_delta_emits_stream_error_before_done() {
        let inner = Box::pin(futures::stream::iter([
            Ok(StreamChunk {
                delta: "hello".to_string(),
                ..Default::default()
            }),
            Ok(StreamChunk {
                thinking: Some("let me think".to_string()),
                ..Default::default()
            }),
            Ok(StreamChunk {
                done: true,
                finish_reason: Some(FinishReason::Stop),
                ..Default::default()
            }),
        ]));
        let sse = build_openai_sse_response(inner, "id-p3".into(), 0, "m".into(), true);
        let text = collect_sse(sse).await;
        let err_pos = text
            .find("stream_error")
            .expect("thinking delta must emit a stream_error event (SPCC §6.3)");
        let done_pos = text.find("[DONE]").expect("DONE must be emitted");
        assert!(err_pos < done_pos, "stream_error must appear before [DONE]");
    }

    /// `thinking_done == true` also triggers the guard.
    #[tokio::test]
    async fn thinking_done_emits_stream_error_before_done() {
        let inner = Box::pin(futures::stream::iter([
            Ok(StreamChunk {
                delta: "hi".to_string(),
                ..Default::default()
            }),
            Ok(StreamChunk {
                done: true,
                thinking_done: Some(true),
                finish_reason: Some(FinishReason::Stop),
                ..Default::default()
            }),
        ]));
        let sse = build_openai_sse_response(inner, "id-p4".into(), 0, "m".into(), true);
        let text = collect_sse(sse).await;
        let err_pos = text
            .find("stream_error")
            .expect("thinking_done must emit a stream_error event (SPCC §6.3)");
        let done_pos = text.find("[DONE]").expect("DONE must be emitted");
        assert!(err_pos < done_pos, "stream_error must appear before [DONE]");
    }

    // ── PRX-005: request body limit + error normalization (red first) ──

    struct ParseErrorProvider;

    #[async_trait::async_trait]
    impl Provider for ParseErrorProvider {
        async fn chat(&self, _req: &ChatRequest) -> Result<ChatResponse> {
            Err(LlmError::Parse("upstream sent invalid JSON".to_string()))
        }

        async fn stream(
            &self,
            _req: &ChatRequest,
        ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
            Err(LlmError::Parse("upstream sent invalid JSON".to_string()))
        }
    }

    /// An oversized request body must be rejected with a PROTOCOL-SHAPED 413
    /// JSON error body (SPCC §11.6 PRX-005), not axum's default plain-text
    /// 413. Currently there is no explicit body limit layer → RED.
    #[tokio::test]
    async fn oversized_body_returns_protocol_shaped_413() {
        let llm = Arc::new(LmrsClient::new());
        llm.set_custom("mock", Arc::new(MockProvider)).await;
        let app = router(llm);

        // 3 MiB payload — over the proposed 2 MiB default.
        let big = "x".repeat(3 * 1024 * 1024);
        let body = serde_json::json!({
            "model": "mock/test",
            "messages": [{"role": "user", "content": big}],
        })
        .to_string();

        let response = app
            .oneshot(build_request(&body))
            .await
            .expect("request failed");
        assert_eq!(
            response.status(),
            StatusCode::PAYLOAD_TOO_LARGE,
            "oversized body must be rejected with 413"
        );
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
        assert_eq!(
            json["error"]["type"], "invalid_request_error",
            "413 must be a protocol-shaped JSON error body, got: {json}"
        );
    }

    /// The body limit must also apply to chunked (no Content-Length) bodies.
    #[tokio::test]
    async fn oversized_chunked_body_returns_413() {
        let llm = Arc::new(LmrsClient::new());
        llm.set_custom("mock", Arc::new(MockProvider)).await;
        let app = router(llm);

        let big = "x".repeat(3 * 1024 * 1024);
        let body = serde_json::json!({
            "model": "mock/test",
            "messages": [{"role": "user", "content": big}],
        })
        .to_string();
        // Send without Content-Length (axum treats it as chunked).
        let req = Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header("content-type", "application/json")
            .header("transfer-encoding", "chunked")
            .body(Body::from(body))
            .expect("failed to build request");

        let response = app.oneshot(req).await.expect("request failed");
        assert_eq!(
            response.status(),
            StatusCode::PAYLOAD_TOO_LARGE,
            "oversized chunked body must be rejected with 413"
        );
    }

    /// An upstream Parse failure must be normalized to a 502 protocol-shaped
    /// error body whose message does NOT expose the raw upstream text
    /// verbatim (architect ruling: Parse → 502 api_error; message truncated /
    /// sanitized). Currently proxy_error_from_llm_error maps Parse → 400 and
    /// echoes e.to_string() → RED.
    #[tokio::test]
    async fn upstream_parse_error_returns_502_sanitized() {
        let llm = Arc::new(LmrsClient::new());
        llm.set_custom("bad", Arc::new(ParseErrorProvider)).await;
        let app = router(llm);
        let body = serde_json::json!({
            "model": "bad/test",
            "messages": [{"role": "user", "content": "hi"}],
        })
        .to_string();

        let response = app
            .oneshot(build_request(&body))
            .await
            .expect("request failed");
        assert_eq!(
            response.status(),
            StatusCode::BAD_GATEWAY,
            "upstream Parse must map to 502 (architect ruling)"
        );
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
        assert_eq!(json["error"]["type"], "api_error");
        let msg = json["error"]["message"].as_str().unwrap_or("");
        assert!(
            msg.len() <= 200,
            "error message must be truncated to ≤200 chars, got {}",
            msg.len()
        );
    }

    /// `LLMRUST_PROXY_MAX_BODY_BYTES` must configure the body limit. This
    /// reads the env directly (a single synchronous call, minimal race
    /// window) and asserts the configured value is honored; the router-level
    /// 413 shape is covered by the oversized-body tests at the default 2 MiB.
    #[test]
    fn env_configured_body_limit_is_read() {
        std::env::set_var("LLMRUST_PROXY_MAX_BODY_BYTES", "1024");
        // Read immediately after set (window is one function call).
        assert_eq!(proxy_max_body_bytes(), 1024);
        std::env::remove_var("LLMRUST_PROXY_MAX_BODY_BYTES");
        // Default when unset.
        assert_eq!(proxy_max_body_bytes(), 2 * 1024 * 1024);
    }

    /// The chunked path's 413 must ALSO be a protocol-shaped JSON error body
    /// (D3 drift: axum's default chunked 413 is plain text). Currently the
    /// shape assertion is RED even though the status is 413.
    #[tokio::test]
    async fn oversized_chunked_body_returns_protocol_shaped_413() {
        let llm = Arc::new(LmrsClient::new());
        llm.set_custom("mock", Arc::new(MockProvider)).await;
        let app = router(llm);

        let big = "x".repeat(3 * 1024 * 1024);
        let body = serde_json::json!({
            "model": "mock/test",
            "messages": [{"role": "user", "content": big}],
        })
        .to_string();
        let req = Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header("content-type", "application/json")
            .header("transfer-encoding", "chunked")
            .body(Body::from(body))
            .expect("failed to build request");

        let response = app.oneshot(req).await.expect("request failed");
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
        assert_eq!(
            json["error"]["type"], "invalid_request_error",
            "chunked 413 must be a protocol-shaped JSON error body, got: {json}"
        );
    }
}
