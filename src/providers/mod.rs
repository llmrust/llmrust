//! Provider trait and unified LLM client.

pub mod anthropic;
pub mod compat;
pub mod deepseek;
pub mod google;
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
/// The `Debug` implementation masks the `api_key` and configured `base_url`
/// values to prevent accidental leakage in logs or panic messages.
#[derive(Clone)]
pub struct ProviderConfig {
    pub api_key: String,
    pub base_url: Option<String>,
}

impl std::fmt::Debug for ProviderConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderConfig")
            .field("api_key", &"***")
            .field("base_url", &self.base_url.as_ref().map(|_| "***"))
            .finish()
    }
}

impl ProviderConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: None,
        }
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = Some(url.into());
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
