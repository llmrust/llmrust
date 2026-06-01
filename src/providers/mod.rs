//! Provider trait and unified LLM client.

pub mod anthropic;
pub mod deepseek;
pub mod google;
pub mod moonshot;
pub mod ollama;
pub mod openai;
pub mod openrouter;

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
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub api_key: String,
    pub base_url: Option<String>,
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
