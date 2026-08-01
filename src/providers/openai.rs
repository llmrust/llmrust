//! OpenAI API provider (and any OpenAI-compatible API via [`OpenAIProvider`]).
//!
//! Delegates to [`crate::providers::compat::OpenAiCompatibleProvider`].

use async_trait::async_trait;
use futures::stream::BoxStream;

use crate::providers::{compat::OpenAiCompatibleProvider, Provider, ProviderConfig, Result};
use crate::types::{ChatRequest, ChatResponse, EmbeddingRequest, EmbeddingResponse, StreamChunk};

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

/// OpenAI provider (also works with any OpenAI-compatible API).
pub struct OpenAIProvider(OpenAiCompatibleProvider);

impl OpenAIProvider {
    /// Create a new OpenAI provider. Uses the default `api.openai.com` base
    /// URL unless overridden via `config.base_url`.
    pub fn new(config: ProviderConfig) -> Self {
        let config = ProviderConfig {
            base_url: Some(
                config
                    .base_url
                    .unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
            ),
            ..config
        };
        // REA-003: the verified OpenAI endpoint opts into reasoning support;
        // third-party OpenAI-compatible wrappers must NOT inherit this.
        Self(OpenAiCompatibleProvider::new(config, []).with_reasoning(true))
    }
}

#[async_trait]
impl Provider for OpenAIProvider {
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
