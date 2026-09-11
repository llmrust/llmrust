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

// ── GRD-003 ②：门自身必须会红 ──────────────────────────────────────────
//
// 本文件此前 9 条测试全是"跑一遍、断言结果"的正向测试：它们能证明**今天**策略正确，
// 但**不能证明"策略被改坏会被发现"**。下面把被钉住的 retry 策略抽成纯函数
// [`retry_policy_problems`]，再用**真实观测**与**人为违约样本**双向驱动。

/// 一次观测到的重试行为：`(http status, 实际尝试次数, 配置的最大重试次数)`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryObservation {
    /// 上游返回的 HTTP 状态码。
    pub status: u16,
    /// 内层 provider 被调用的次数（1 = 未重试）。
    pub attempts: u32,
    /// `RetryProvider::new(inner, max_retries)` 的配置值。
    pub max_retries: u32,
}

/// **纯函数**：观测相对 0.1.3 钉住的 RetryProvider 策略的违规（空 = 合规）。
///
/// 策略（与 `it3a/it3b/it3c` 同源）：
/// - **5xx**：可重试，最多 `max_retries` 次重试 ⇒ `attempts <= max_retries + 1`；
/// - **4xx / 429**：**不重试** ⇒ `attempts == 1`（这是 ACL/计费安全的关键：
///   4xx 重试会重复计费且掩盖客户端错误）。
pub fn retry_policy_problems(obs: &[RetryObservation]) -> Vec<String> {
    let mut out = Vec::new();
    for o in obs {
        if (400..500).contains(&o.status) {
            if o.attempts != 1 {
                out.push(format!(
                    "status {}: attempts = {} but 4xx/429 must NOT be retried (expected 1)",
                    o.status, o.attempts
                ));
            }
        } else if ((500..600).contains(&o.status) || o.status == 0)
            && o.attempts > o.max_retries + 1
        {
            out.push(format!(
                "status {}: attempts = {} exceeds max_retries + 1 = {}",
                o.status,
                o.attempts,
                o.max_retries + 1
            ));
        }
        if o.attempts == 0 {
            out.push(format!(
                "status {}: attempts = 0 (provider was never called)",
                o.status
            ));
        }
    }
    out
}

/// 负例一：**4xx 被重试**（策略违约，且会重复计费）→ 必须报。
#[test]
fn negative_retried_4xx_is_reported() {
    let problems = retry_policy_problems(&[RetryObservation {
        status: 400,
        attempts: 3,
        max_retries: 3,
    }]);
    assert_eq!(problems.len(), 1, "got {problems:?}");
    assert!(problems[0].contains("must NOT be retried"), "{problems:?}");
}

/// 负例二：**429 被重试** → 必须报（429 与 4xx 同策）。
#[test]
fn negative_retried_429_is_reported() {
    let problems = retry_policy_problems(&[RetryObservation {
        status: 429,
        attempts: 2,
        max_retries: 3,
    }]);
    assert_eq!(problems.len(), 1, "got {problems:?}");
}

/// 负例三：**5xx 重试次数超上限** → 必须报。
#[test]
fn negative_5xx_over_retry_budget_is_reported() {
    let problems = retry_policy_problems(&[RetryObservation {
        status: 502,
        attempts: 9,
        max_retries: 3,
    }]);
    assert_eq!(problems.len(), 1, "got {problems:?}");
    assert!(problems[0].contains("exceeds max_retries"), "{problems:?}");
}

/// 负例四：`attempts == 0`（provider 根本没被调用）→ 必须报（否则"没调用"会被当成合规）。
#[test]
fn negative_zero_attempts_is_reported() {
    let problems = retry_policy_problems(&[RetryObservation {
        status: 200,
        attempts: 0,
        max_retries: 3,
    }]);
    assert_eq!(problems.len(), 1, "got {problems:?}");
    assert!(problems[0].contains("never called"), "{problems:?}");
}

/// **正向对照 + 与真实行为的对账**：真跑一遍 `RetryProvider`，把**实测观测值**
/// 喂给同一个纯函数 —— 必须零违规（证明线上的确遵守被钉住的策略，且抽出的判据
/// 与线上行为是同一口径，不是自说自话）。
#[tokio::test]
async fn real_retry_behaviour_satisfies_the_extracted_policy() {
    // 4xx/429：不得重试
    for status in [400u16, 401, 403, 429] {
        let inner = FlakyProvider::new(10, status);
        let retry = llmrust::RetryProvider::new(inner.clone(), 3);
        let _ = retry.chat(&req()).await;
        let obs = RetryObservation {
            status,
            attempts: inner.call_count(),
            max_retries: 3,
        };
        let problems = retry_policy_problems(&[obs]);
        assert!(
            problems.is_empty(),
            "real provider violated the pinned policy: {problems:?} (obs = {obs:?})"
        );
    }

    // 5xx：允许重试，但不得超预算（这里 3 次失败 + 1 次成功 = 4 次尝试，预算 3+1）
    let inner = FlakyProvider::new(3, 502);
    let retry = llmrust::RetryProvider::new(inner.clone(), 3);
    let resp = retry.chat(&req()).await.expect("must eventually succeed");
    assert_eq!(resp.content, "ok");
    let obs = RetryObservation {
        status: 502,
        attempts: inner.call_count(),
        max_retries: 3,
    };
    let problems = retry_policy_problems(&[obs]);
    assert!(
        problems.is_empty(),
        "5xx retry budget violated: {problems:?} (obs = {obs:?})"
    );
}
