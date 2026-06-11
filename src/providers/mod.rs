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
pub mod stream_util;

use async_trait::async_trait;
use futures::stream::BoxStream;

use crate::types::{ChatRequest, ChatResponse, StreamChunk};

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
}

pub type Result<T> = std::result::Result<T, LlmError>;

/// The core trait that all LLM providers must implement.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Send a chat completion request and get the full response.
    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse>;

    /// Send a streaming chat completion request.
    async fn stream(&self, req: &ChatRequest) -> Result<BoxStream<'static, Result<StreamChunk>>>;
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
    fn debug_hides_api_key() {
        let config =
            ProviderConfig::new("sk-secret-12345").with_base_url("https://gateway.example/v1");
        let debug = format!("{:?}", config);
        assert!(
            !debug.contains("sk-secret-12345"),
            "Debug output should not contain the API key, got: {debug}"
        );
        assert!(
            !debug.contains("gateway.example"),
            "Debug output should not contain the base URL, got: {debug}"
        );
        assert!(debug.contains("***"));
    }
}
