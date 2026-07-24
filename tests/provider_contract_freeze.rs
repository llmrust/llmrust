//! Provider / error / Retry contract freeze tests for 0.1.3 (API-003).
//!
//! These pin the *behavior* shipped in 0.1.3 so that any later shape change
//! (Provider trait, `LlmError`, `RetryProvider` policy, embed default, client
//! delegation) breaks CI instead of silently drifting. They are additive and
//! do not alter any runtime behavior.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use futures::stream::{self, BoxStream};

use llmrust::providers::{LlmError, Provider, Result};
use llmrust::types::{ChatRequest, ChatResponse, EmbeddingRequest, EmbeddingResponse, StreamChunk};

// ── Mock providers ─────────────────────────────────────────────────────────

struct FlakyProvider {
    attempts: Arc<AtomicU32>,
    max_fails: u32,
    status: u16,
}

impl FlakyProvider {
    fn new(max_fails: u32, status: u16) -> Arc<Self> {
        Arc::new(Self {
            attempts: Arc::new(AtomicU32::new(0)),
            max_fails,
            status,
        })
    }
    fn call_count(&self) -> u32 {
        self.attempts.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Provider for FlakyProvider {
    async fn chat(&self, _req: &ChatRequest) -> Result<ChatResponse> {
        let n = self.attempts.fetch_add(1, Ordering::SeqCst);
        if n < self.max_fails {
            return Err(LlmError::Api {
                status: self.status,
                message: "transient".into(),
            });
        }
        Ok(ChatResponse {
            content: "ok".into(),
            model: "test".into(),
            ..Default::default()
        })
    }
    async fn stream(&self, _req: &ChatRequest) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        let n = self.attempts.fetch_add(1, Ordering::SeqCst);
        if n < self.max_fails {
            return Err(LlmError::Api {
                status: self.status,
                message: "transient".into(),
            });
        }
        Ok(Box::pin(stream::once(async {
            Ok(StreamChunk {
                delta: "ok".into(),
                done: true,
                ..Default::default()
            })
        })))
    }
    // embed() uses the trait default (Unsupported) — exercised by IT-1.
}

struct EmbedOkProvider;
#[async_trait]
impl Provider for EmbedOkProvider {
    async fn chat(&self, _: &ChatRequest) -> Result<ChatResponse> {
        Ok(ChatResponse::default())
    }
    async fn stream(&self, _: &ChatRequest) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        Ok(Box::pin(stream::empty()))
    }
    async fn embed(&self, req: &EmbeddingRequest) -> Result<EmbeddingResponse> {
        Ok(EmbeddingResponse {
            model: req.model.clone(),
            data: vec![llmrust::Embedding {
                index: 0,
                embedding: vec![0.1],
            }],
            usage: None,
        })
    }
}

struct EmbedUnsupportedProvider;
#[async_trait]
impl Provider for EmbedUnsupportedProvider {
    async fn chat(&self, _: &ChatRequest) -> Result<ChatResponse> {
        Ok(ChatResponse::default())
    }
    async fn stream(&self, _: &ChatRequest) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        Ok(Box::pin(stream::empty()))
    }
    // embed() default Unsupported — exercised by IT-4b.
}

fn req() -> ChatRequest {
    ChatRequest::new("test", "hello")
}

// ── IT-1: embed default is Unsupported ─────────────────────────────────────

#[tokio::test]
async fn it1_embed_default_unsupported() {
    let p = FlakyProvider::new(0, 400);
    let err = p
        .embed(&EmbeddingRequest::new("test", "hello"))
        .await
        .unwrap_err();
    match err {
        LlmError::Unsupported { feature, .. } => assert_eq!(feature, "embeddings"),
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

// ── IT-2: LlmError variant surface ─────────────────────────────────────────

#[tokio::test]
async fn it2_llm_error_variants() {
    // `Api` exposes `status` (used by retry policy / proxy mapping).
    let LlmError::Api { status, .. } = (LlmError::Api {
        status: 429,
        message: "rl".into(),
    }) else {
        unreachable!()
    };
    assert_eq!(status, 429);

    // The other constructible variants exist with their documented shapes.
    let _s = LlmError::Stream("x".into());
    let _p = LlmError::Parse("bad".into());
    let _u = LlmError::UnknownProvider("p".into());
    let LlmError::Unsupported { feature, .. } = (LlmError::Unsupported {
        feature: "f".into(),
        message: "m".into(),
    }) else {
        unreachable!()
    };
    assert_eq!(feature, "f");

    // `Http` is `#[from] reqwest::Error` with no public constructor; it is
    // covered by real-network tests elsewhere.
}

// ── IT-3: RetryProvider retry policy ───────────────────────────────────────

#[tokio::test]
async fn it3a_retries_5xx() {
    let inner = FlakyProvider::new(2, 502);
    let retry = llmrust::RetryProvider::new(inner, 3);
    let resp = retry.chat(&req()).await.unwrap();
    assert_eq!(resp.content, "ok");
}

#[tokio::test]
async fn it3b_no_retry_4xx() {
    let inner = FlakyProvider::new(10, 400);
    let retry = llmrust::RetryProvider::new(inner.clone(), 3);
    let err = retry.chat(&req()).await.unwrap_err();
    assert!(matches!(err, LlmError::Api { status: 400, .. }));
    assert_eq!(inner.call_count(), 1, "4xx must not be retried");
}

#[tokio::test]
async fn it3c_no_retry_429() {
    let inner = FlakyProvider::new(10, 429);
    let retry = llmrust::RetryProvider::new(inner.clone(), 3);
    let err = retry.chat(&req()).await.unwrap_err();
    assert!(matches!(err, LlmError::Api { status: 429, .. }));
    assert_eq!(
        inner.call_count(),
        1,
        "429 must not be retried by RetryProvider"
    );
}

#[tokio::test]
async fn it3d_respects_max_retries() {
    let inner = FlakyProvider::new(10, 502);
    let retry = llmrust::RetryProvider::new(inner.clone(), 2); // 2 retries => 3 attempts
    let err = retry.chat(&req()).await.unwrap_err();
    assert!(matches!(err, LlmError::Api { status: 502, .. }));
    assert_eq!(inner.call_count(), 3, "must stop after max_retries");
}

// ── IT-4: embed delegation through RetryProvider ───────────────────────────

#[tokio::test]
async fn it4a_embed_delegates() {
    let inner = Arc::new(EmbedOkProvider);
    let retry = llmrust::RetryProvider::new(inner, 0);
    let resp = retry
        .embed(&EmbeddingRequest::new("test", "hello"))
        .await
        .unwrap();
    assert_eq!(resp.data.len(), 1);
}

#[tokio::test]
async fn it4b_embed_unsupported_passthrough() {
    let inner = Arc::new(EmbedUnsupportedProvider);
    let retry = llmrust::RetryProvider::new(inner, 0);
    let err = retry
        .embed(&EmbeddingRequest::new("test", "hello"))
        .await
        .unwrap_err();
    assert!(matches!(err, LlmError::Unsupported { .. }));
}

// ── IT-5: LmrsClient delegation API surface (compile-time freeze) ──────────

#[tokio::test]
async fn it5_client_delegation_api_present() {
    // Runtime freeze: every LmrsClient delegation entry point used by 0.1.3
    // must exist and be callable. These only store provider config — no network.
    let client = llmrust::LmrsClient::new();
    client.set_openai("sk-test").await;
    client
        .set_openai_compatible("sk-test", "https://api.example.com/v1")
        .await;
    client.set_anthropic("sk-test").await;
    client.set_deepseek("sk-test").await;
    client.set_google("sk-test").await;
    client
        .set_ollama(Some("http://localhost:11434".into()))
        .await;
    client.set_moonshot("sk-test").await;
    client.set_openrouter("sk-test").await;
    let custom: std::sync::Arc<dyn llmrust::providers::Provider> =
        std::sync::Arc::new(EmbedUnsupportedProvider);
    client.set_custom("custom", custom).await;
    // from_env() reads env and returns a client (no panic when vars absent).
    let _from_env = llmrust::LmrsClient::from_env().await;
}
