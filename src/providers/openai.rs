//! OpenAI API provider (and any OpenAI-compatible API via [`OpenAIProvider`]).
//!
//! Delegates to [`crate::providers::compat::OpenAiCompatibleProvider`].

use async_trait::async_trait;
use futures::stream::BoxStream;

use crate::providers::{compat::OpenAiCompatibleProvider, Provider, ProviderConfig, Result};
use crate::types::{ChatRequest, ChatResponse, EmbeddingRequest, EmbeddingResponse, StreamChunk};

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

/// OpenAI provider (also works with any OpenAI-compatible API).
pub struct OpenAIProvider {
    inner: OpenAiCompatibleProvider,
    /// REA-003 / AUD#8: whether the official-endpoint reasoning claim applies
    /// to THIS instance — set only by [`OpenAIProvider::new`] (the verified
    /// `api.openai.com` endpoint).
    reasoning_official: bool,
}

impl OpenAIProvider {
    /// Create a new OpenAI provider. Uses the default `api.openai.com` base
    /// URL unless overridden via `config.base_url`.
    pub fn new(config: ProviderConfig) -> Self {
        // Any caller-supplied base URL uses the compatible profile, even when
        // it happens to point at OpenAI; only the built-in origin is verified.
        let reasoning_official = config.base_url.is_none();
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
        Self {
            inner: OpenAiCompatibleProvider::new(config, []).with_reasoning(reasoning_official),
            reasoning_official,
        }
    }

    /// Create a provider for a third-party OpenAI-compatible endpoint
    /// (AUD#8, C-AUD-PROV).
    ///
    /// Unlike [`OpenAIProvider::new`], this constructor does **not** opt into
    /// reasoning. The REA-003 verification — the official OpenAI endpoint may
    /// send reasoning fields, third-party wrappers must not inherit this — is
    /// a claim about the official endpoint, so a compatible-construct instance
    /// declares the reasoning face `Unsupported` ([`Provider::capabilities`])
    /// and rejects `ThinkingConfig::Enabled` with `LlmError::Unsupported`
    /// before any network call, exactly like any other unverified compatible
    /// endpoint. [`crate::LmrsClient::set_openai_compatible`] builds
    /// registered providers this way. Uses the default `api.openai.com` base
    /// URL unless overridden via `config.base_url`.
    pub fn new_compatible(config: ProviderConfig) -> Self {
        let config = ProviderConfig {
            base_url: Some(
                config
                    .base_url
                    .unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
            ),
            ..config
        };
        Self {
            inner: OpenAiCompatibleProvider::new(config, []),
            reasoning_official: false,
        }
    }
}

#[async_trait]
impl Provider for OpenAIProvider {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse> {
        self.inner.chat(req).await
    }

    async fn stream(&self, req: &ChatRequest) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        self.inner.stream(req).await
    }

    async fn embed(&self, req: &EmbeddingRequest) -> Result<EmbeddingResponse> {
        self.inner.embed(req).await
    }

    /// ERR-004：Retry-After 提示由内层兼容实现捕获，此处**委派**。
    fn last_retry_after(&self) -> Option<std::time::Duration> {
        self.inner.last_retry_after()
    }

    fn protocol_name(&self) -> &'static str {
        "openai"
    }

    /// `CAP-002` 声明。逐项依据（代码行号见 PR 回证表）：
    /// chat/stream/tool_calling/`tool_calling_stream`/image_input/embeddings/system —— 本文件与
    /// `compat.rs` 均实现；reasoning 走请求 `reasoning_effort` + 流式 `thinking` 映射
    /// （**本地夹具**级证据）；`prompt_cache` = `CAP-005` 落地后 OpenAI 兼容侧的自动缓存
    /// 由上游处理，llmrust 只保证读价字段映射。
    ///
    /// AUD#8（C-AUD-PROV）：reasoning 是**实例级**声明——官方端点构造
    /// （[`OpenAIProvider::new`]，`verified`）与第三方兼容端点构造
    /// （[`OpenAIProvider::new_compatible`]，`unsupported`）不同；第三方包装不继承
    /// official-endpoint 的推理声明（REA-003）。
    fn capabilities(&self) -> crate::providers::capabilities::Capabilities {
        use crate::providers::capabilities::{Capabilities, Capability, EvidenceKind};
        Capabilities {
            declared: true,
            protocol: "openai-compatible",
            chat: Capability::implemented(),
            stream: Capability::implemented(),
            tool_calling: Capability::implemented(),
            tool_calling_stream: Capability::implemented(),
            image_input: Capability::implemented(),
            embeddings: Capability::implemented(),
            reasoning: if self.reasoning_official {
                Capability::verified(EvidenceKind::LocalFixture, "2026-08-02")
            } else {
                Capability::unsupported()
            },
            prompt_cache: Capability::model_dependent(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::capabilities::CapabilityLevel;

    fn cfg() -> ProviderConfig {
        ProviderConfig {
            api_key: "k".to_string(),
            base_url: None,
            timeout_secs: None,
            custom_headers: None,
        }
    }

    /// AUD#8 (C-AUD-PROV): a `new_compatible` instance registers a
    /// third-party endpoint, which must NOT inherit the official endpoint's
    /// verified reasoning declaration (REA-003); the official constructor
    /// keeps it (no regression).
    #[test]
    fn compatible_construction_does_not_declare_reasoning() {
        let compatible = OpenAIProvider::new_compatible(cfg());
        assert_eq!(
            compatible.capabilities().reasoning.level,
            CapabilityLevel::Unsupported,
            "a third-party compatible endpoint must not inherit official-endpoint \
             reasoning support"
        );

        let official = OpenAIProvider::new(cfg());
        assert_ne!(
            official.capabilities().reasoning.level,
            CapabilityLevel::Unsupported,
            "the official endpoint keeps its verified reasoning declaration (no regression)"
        );
    }
}
