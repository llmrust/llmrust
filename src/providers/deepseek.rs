//! DeepSeek API provider (OpenAI-compatible).
//!
//! Delegates to [`crate::providers::compat::OpenAiCompatibleProvider`].

use async_trait::async_trait;
use futures::stream::BoxStream;

use crate::providers::{compat::OpenAiCompatibleProvider, Provider, ProviderConfig, Result};
use crate::types::{ChatRequest, ChatResponse, EmbeddingRequest, EmbeddingResponse, StreamChunk};

const DEFAULT_BASE_URL: &str = "https://api.deepseek.com/v1";

/// DeepSeek provider.
pub struct DeepSeekProvider(OpenAiCompatibleProvider);

impl DeepSeekProvider {
    /// Create a new DeepSeek provider.
    pub fn new(config: ProviderConfig) -> Self {
        let config = ProviderConfig {
            base_url: Some(
                config
                    .base_url
                    .unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
            ),
            ..config
        };
        Self(OpenAiCompatibleProvider::new(config, []))
    }
}

#[async_trait]
impl Provider for DeepSeekProvider {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse> {
        self.0.chat(req).await
    }

    async fn stream(&self, req: &ChatRequest) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        self.0.stream(req).await
    }

    async fn embed(&self, req: &EmbeddingRequest) -> Result<EmbeddingResponse> {
        self.0.embed(req).await
    }

    fn protocol_name(&self) -> &'static str {
        "deepseek"
    }

    /// `CAP-002` 声明：OpenAI 兼容族；DeepSeek 侧**不映射** reasoning 的流式契约
    /// （见 `docs/CAPABILITIES.md` 与 `llmrust.capabilities.json` 的既有记载），
    /// 故 reasoning 记 `model_dependent` 而非 `verified`。
    fn capabilities(&self) -> crate::providers::capabilities::Capabilities {
        use crate::providers::capabilities::{Capabilities, Capability};
        Capabilities {
            protocol: "openai-compatible",
            chat: Capability::implemented(),
            stream: Capability::implemented(),
            tool_calling: Capability::implemented(),
            tool_calling_stream: Capability::implemented(),
            image_input: Capability::unsupported(),
            embeddings: Capability::implemented(),
            reasoning: Capability::model_dependent(),
            prompt_cache: Capability::model_dependent(),
        }
    }
}
