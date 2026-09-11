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

    fn protocol_name(&self) -> &'static str {
        "openai"
    }

    /// `CAP-002` 声明。逐项依据（代码行号见 PR 回证表）：
    /// chat/stream/tool_calling/`tool_calling_stream`/image_input/embeddings/system —— 本文件与
    /// `compat.rs` 均实现；reasoning 走请求 `reasoning_effort` + 流式 `thinking` 映射
    /// （**本地夹具**级证据）；`prompt_cache` = `CAP-005` 落地后 OpenAI 兼容侧的自动缓存
    /// 由上游处理，llmrust 只保证读价字段映射。
    fn capabilities(&self) -> crate::providers::capabilities::Capabilities {
        use crate::providers::capabilities::{Capabilities, Capability, EvidenceKind};
        Capabilities {
            protocol: "openai-compatible",
            chat: Capability::implemented(),
            stream: Capability::implemented(),
            tool_calling: Capability::implemented(),
            tool_calling_stream: Capability::implemented(),
            image_input: Capability::implemented(),
            embeddings: Capability::implemented(),
            reasoning: Capability::verified(EvidenceKind::LocalFixture, "2026-08-02"),
            prompt_cache: Capability::model_dependent(),
        }
    }
}
