//! OpenRouter API provider (OpenAI-compatible).
//!
//! Delegates to [`crate::providers::compat::OpenAiCompatibleProvider`].

use async_trait::async_trait;
use futures::stream::BoxStream;

use crate::providers::{compat::OpenAiCompatibleProvider, Provider, ProviderConfig, Result};
use crate::types::{ChatRequest, ChatResponse, EmbeddingRequest, EmbeddingResponse, StreamChunk};

const DEFAULT_BASE_URL: &str = "https://openrouter.ai/api/v1";

/// OpenRouter provider.
pub struct OpenRouterProvider(OpenAiCompatibleProvider);

impl OpenRouterProvider {
    /// Create a new OpenRouter provider. Adds `HTTP-Referer` and `X-Title`
    /// headers as required by OpenRouter.
    pub fn new(config: ProviderConfig) -> Self {
        let config = ProviderConfig {
            base_url: Some(
                config
                    .base_url
                    .unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
            ),
            ..config
        };
        Self(OpenAiCompatibleProvider::new(
            config,
            [
                (
                    "HTTP-Referer".to_string(),
                    "https://github.com/llmrust/llmrust".to_string(),
                ),
                ("X-Title".to_string(), "llmrust".to_string()),
            ],
        ))
    }
}

#[async_trait]
impl Provider for OpenRouterProvider {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse> {
        self.0.chat(req).await
    }

    async fn stream(&self, req: &ChatRequest) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        self.0.stream(req).await
    }

    async fn embed(&self, req: &EmbeddingRequest) -> Result<EmbeddingResponse> {
        self.0.embed(req).await
    }
}
