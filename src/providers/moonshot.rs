//! Moonshot / Kimi API provider (OpenAI-compatible).
//!
//! Delegates to [`crate::providers::compat::OpenAiCompatibleProvider`].

use async_trait::async_trait;
use futures::stream::BoxStream;

use crate::providers::{compat::OpenAiCompatibleProvider, Provider, ProviderConfig, Result};
use crate::types::{ChatRequest, ChatResponse, StreamChunk};

const DEFAULT_BASE_URL: &str = "https://api.moonshot.cn/v1";

/// Moonshot / Kimi provider.
pub struct MoonshotProvider(OpenAiCompatibleProvider);

impl MoonshotProvider {
    /// Create a new Moonshot provider.
    pub fn new(config: ProviderConfig) -> Self {
        let config = ProviderConfig::new(&config.api_key).with_base_url(
            config
                .base_url
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
        );
        Self(OpenAiCompatibleProvider::new(config, []))
    }
}

#[async_trait]
impl Provider for MoonshotProvider {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse> {
        self.0.chat(req).await
    }

    async fn stream(&self, req: &ChatRequest) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        self.0.stream(req).await
    }
}
