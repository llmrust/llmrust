//! Provider trait and unified LLM client.

pub mod anthropic;
pub mod compat;
pub mod deepseek;
pub mod google;
pub(crate) mod http;
pub mod moonshot;
pub mod ollama;
pub mod openai;
pub mod openrouter;
pub mod retry;
pub(crate) mod stream_state;
pub mod stream_util;

use async_trait::async_trait;
use futures::stream::BoxStream;
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use crate::types::{ChatRequest, ChatResponse, EmbeddingRequest, EmbeddingResponse, StreamChunk};

/// Errors that can occur when calling an LLM provider.
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("API error ({status}): {message}")]
    Api { status: u16, message: String },

    #[error("Stream error: {0}")]
    Stream(String),

    #[error("Invalid response: {0}")]
    Parse(String),

    #[error("Unknown provider: {0}")]
    UnknownProvider(String),

    /// A requested feature this provider does not implement.
    #[error("unsupported feature `{feature}`: {message}")]
    Unsupported { feature: String, message: String },
}

pub type Result<T> = std::result::Result<T, LlmError>;

/// The core trait that all LLM providers must implement.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Send a chat completion request and get the full response.
    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse>;

    /// Send a streaming chat completion request.
    async fn stream(&self, req: &ChatRequest) -> Result<BoxStream<'static, Result<StreamChunk>>>;

    /// Generate embeddings for one or more input strings.
    ///
    /// Default returns [`LlmError::Unsupported`]. Providers that support
    /// embeddings must override this method.
    async fn embed(&self, _req: &EmbeddingRequest) -> Result<EmbeddingResponse> {
        Err(LlmError::Unsupported {
            feature: "embeddings".to_string(),
            message: "provider does not implement embeddings".to_string(),
        })
    }
}

/// Configuration for a provider.
///
/// The `Debug` implementation masks the `api_key`, `base_url`, and
/// `custom_headers` values to prevent accidental leakage in logs or panic
/// messages.
#[derive(Clone)]
pub struct ProviderConfig {
    pub api_key: String,
    pub base_url: Option<String>,
    /// Per-request timeout in seconds. `None` means use the provider default
    /// (120 s for hosted APIs; no overall timeout for local backends).
    pub timeout_secs: Option<u64>,
    /// Custom HTTP headers attached to every request. Useful for
    /// provider-specific extensions (e.g. `x-api-key`, organisation IDs,
    /// OpenRouter app attribution).
    pub custom_headers: Option<std::collections::HashMap<String, String>>,
}

impl std::fmt::Debug for ProviderConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderConfig")
            .field("api_key", &"***")
            .field("base_url", &self.base_url.as_ref().map(|_| "***"))
            .field("timeout_secs", &self.timeout_secs)
            .field(
                "custom_headers",
                &self.custom_headers.as_ref().map(|_| "***"),
            )
            .finish()
    }
}

// ── Shared helpers ────────────────────────────────────────

/// Deterministic, collision-free tool-call id. Uses a simple `call_{index}`
/// prefix matching the OpenAI convention so that chat and stream paths produce
/// identical ids for the same function calls, and same-name concurrent calls
/// never collide.
pub(crate) fn make_tool_call_id(index: usize) -> String {
    format!("call_{index}")
}

/// True when `n` requests more than one completion. llmrust can only return
/// the first choice; values > 1 are forwarded to OpenAI/Gemini (and billed)
/// but the extra choices are discarded.
pub(crate) fn n_is_unsupported(n: Option<u32>) -> bool {
    matches!(n, Some(k) if k > 1)
}

/// Emit a one-shot warning when `n > 1`. Call once at the top of each
/// provider's `chat`/`stream` so the message appears regardless of routing.
///
/// De-duplicated per `(provider, n)` for the process lifetime (E-002):
/// `RetryProvider` re-enters the inner provider on every retry attempt, which
/// would otherwise re-emit this advisory on each attempt. The message is
/// informational noise, not a functional signal, so collapsing repeats is the
/// intended behavior — the first occurrence per `(provider, n)` is sufficient.
pub(crate) fn warn_if_unsupported_n(provider: &str, n: Option<u32>) {
    if n_is_unsupported(n) {
        static WARNED_N: OnceLock<Mutex<HashSet<(String, u32)>>> = OnceLock::new();
        let key = (provider.to_string(), n.unwrap());
        if WARNED_N
            .get_or_init(Default::default)
            .lock()
            .expect("warn dedupe lock poisoned")
            .insert(key)
        {
            tracing::warn!(
                provider,
                n = n.unwrap(),
                "n > 1 requested but llmrust returns only the first completion; \
                 the remaining choices are discarded and upstream providers may still bill for all N"
            );
        }
    }
}

impl ProviderConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: None,
            timeout_secs: None,
            custom_headers: None,
        }
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = Some(url.into());
        self
    }

    /// Set a per-request timeout in seconds.
    pub fn with_timeout_secs(mut self, secs: u64) -> Self {
        self.timeout_secs = Some(secs);
        self
    }

    /// Add a single custom HTTP header.
    pub fn with_header(mut self, key: impl Into<String>, val: impl Into<String>) -> Self {
        self.custom_headers
            .get_or_insert_with(std::collections::HashMap::new)
            .insert(key.into(), val.into());
        self
    }

    /// Replace all custom headers at once.
    pub fn with_headers(mut self, headers: impl IntoIterator<Item = (String, String)>) -> Self {
        self.custom_headers = Some(headers.into_iter().collect());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_config_debug_masks_sensitive_fields() {
        let mut config =
            ProviderConfig::new("sk-secret-12345").with_base_url("https://gateway.example/v1");
        config.custom_headers = Some(
            [
                ("Authorization".into(), "Bearer hidden-token".into()),
                ("X-Api-Key".into(), "secret-key".into()),
            ]
            .into_iter()
            .collect(),
        );
        let debug = format!("{:?}", config);
        assert!(
            !debug.contains("sk-secret-12345"),
            "Debug output should not contain the API key, got: {debug}"
        );
        assert!(
            !debug.contains("gateway.example"),
            "Debug output should not contain the base URL, got: {debug}"
        );
        assert!(
            !debug.contains("Bearer hidden-token"),
            "Debug output should not contain custom header values, got: {debug}"
        );
        assert!(
            !debug.contains("secret-key"),
            "Debug output should not contain custom header values, got: {debug}"
        );
        assert!(
            debug.contains("***"),
            "Debug output should mask fields, got: {debug}"
        );
    }

    #[test]
    fn n_one_or_none_is_supported() {
        assert!(!n_is_unsupported(None));
        assert!(!n_is_unsupported(Some(1)));
    }

    #[test]
    fn n_greater_than_one_is_unsupported() {
        assert!(n_is_unsupported(Some(2)));
        assert!(n_is_unsupported(Some(5)));
        assert!(n_is_unsupported(Some(128)));
    }
}
