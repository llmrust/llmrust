//! Retry decorator for any [`Provider`].
//!
//! Wraps an existing provider and retries failed requests with exponential
//! backoff and jitter. Retryable errors:
//!
//! - HTTP 5xx (server errors — the upstream may recover)
//! - Network / transport errors (`LlmError::Http`)
//! - Stream connection errors
//!
//! Non-retryable errors (propagated immediately):
//!
//! - HTTP 4xx (client errors — your request is bad)
//! - `LlmError::Parse` (response format changed — retrying won't help)
//! - `LlmError::UnknownProvider` (wrong wiring)
//! - `LlmError::Unsupported` (feature not implemented — retrying won't help)
//!
//! # Example
//!
//! ```rust,ignore
//! let inner = OpenAIProvider::new(config);
//! let retrying = RetryProvider::new(Arc::new(inner) as Arc<dyn Provider>, 3);
//! // use retrying where a Provider is expected
//! ```

use async_trait::async_trait;
use futures::stream::BoxStream;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

use crate::providers::{LlmError, Provider, Result};
use crate::types::{ChatRequest, ChatResponse, EmbeddingRequest, EmbeddingResponse, StreamChunk};

// ── Constants ──────────────────────────────────────────────────────────────

const DEFAULT_BASE_DELAY_MS: u64 = 500; // 0.5 s initial back-off
const DEFAULT_MAX_DELAY_MS: u64 = 30_000; // 30 s ceiling

// ── Retry decision ─────────────────────────────────────────────────────────

/// Returns `true` when the error is (likely) transient and worth retrying.
///
/// ## Differences from [`crate::router`]'s `should_failover`
///
/// Both functions look similar but make **deliberately different choices** for
/// `429` and `UnknownProvider`:
///
/// | Error              | `should_retry` | `should_failover` | Rationale |
/// |--------------------|----------------|--------------------|-----------|
/// | 5xx                | ✅ yes          | ✅ yes              | Transient server error — retry or failover both help. |
/// | 429 (rate limit)   | ❌ no           | ✅ yes              | Retrying the **same deployment** when rate-limited only worsens the situation; but the Router can switch to a **different deployment** that may not be throttled. |
/// | `UnknownProvider`  | ❌ no           | ✅ yes              | A missing provider is a configuration error when retrying a **single** deployment — retrying won't fix it. But a Router with **multiple** deployments can skip the broken one and try the next. |
/// | `Parse`            | ❌ no           | ❌ no               | Malformed responses won't become valid on a second try (or a different deployment). |
///
/// **In short:** `should_retry` asks "should I try this same thing again?" while
/// `should_failover` asks "should I try a different thing instead?".
fn should_retry(e: &LlmError) -> bool {
    match e {
        // Network / transport errors are transient by nature.
        LlmError::Http(_) => true,
        // 5xx means the upstream is having trouble; it may recover.
        LlmError::Api { status, .. } if *status >= 500 => true,
        // Stream connection drops — worth one retry.
        LlmError::Stream(_) => true,
        // Everything else is a permanent failure.
        _ => false,
    }
}

// ── Exponential back-off with jitter ───────────────────────────────────────

fn backoff(attempt: u32, base_ms: u64, max_ms: u64) -> Duration {
    // 2^attempt capped at 2^30 (~1 billion) to avoid u64 overflow.
    let exponent = 1u64 << attempt.min(30);
    let delay = base_ms.saturating_mul(exponent).min(max_ms);

    // "Equal jitter" — spread retries within [delay/2, delay] to avoid
    // thundering-herd. Uses `fastrand` for real randomness so multiple
    // clients retrying the same service don't synchronize.
    let quarter = delay / 4;
    let jitter = match quarter {
        0 => 0,
        q => fastrand::u64(0..q),
    };
    let actual = delay - quarter + jitter;

    Duration::from_millis(actual.max(1))
}

// ── RetryProvider ──────────────────────────────────────────────────────────

/// A [`Provider`] decorator that retries transient failures.
///
/// Internally holds an `Arc<dyn Provider>`, making it possible to wrap an
/// already-boxed provider without knowing its concrete type.
///
/// **Embeddings:** non-streaming [`Provider::embed`] requests are retried with
/// the same transient-error policy as [`Provider::chat`].
///
/// **Note on streaming:** `stream()` retries only the **initial connection**
/// (the call to `inner.stream(req)`). Once a byte stream is established,
/// mid-stream chunk errors are **not** retried — they are propagated to the
/// caller. If you need full mid-stream reliability, use `chat()` instead,
/// or implement application-level retry on the consuming end (e.g. re-drive
/// the prompt on failure).
pub struct RetryProvider {
    inner: Arc<dyn Provider>,
    max_retries: u32,
    base_delay_ms: u64,
    max_delay_ms: u64,
}

impl RetryProvider {
    /// Create a retry wrapper with `max_retries` retry attempts.
    ///
    /// The first retry waits ~`base_delay_ms`, then doubles each attempt up
    /// to `max_delay_ms`.
    pub fn new(inner: Arc<dyn Provider>, max_retries: u32) -> Self {
        Self {
            inner,
            max_retries,
            base_delay_ms: DEFAULT_BASE_DELAY_MS,
            max_delay_ms: DEFAULT_MAX_DELAY_MS,
        }
    }

    /// Customise the initial back-off delay (default: 500 ms).
    pub fn with_base_delay(mut self, ms: u64) -> Self {
        self.base_delay_ms = ms;
        self
    }

    /// Customise the maximum back-off delay (default: 30 s).
    pub fn with_max_delay(mut self, ms: u64) -> Self {
        self.max_delay_ms = ms;
        self
    }
}

#[async_trait]
impl Provider for RetryProvider {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse> {
        for attempt in 0..=self.max_retries {
            match self.inner.chat(req).await {
                Ok(resp) => {
                    if attempt > 0 {
                        tracing::info!(attempt, "retry succeeded");
                    }
                    return Ok(resp);
                }
                Err(e) => {
                    // The final attempt (attempt == max_retries) never retries —
                    // it returns immediately regardless of the error kind.
                    if !(should_retry(&e) && attempt < self.max_retries) {
                        return Err(e);
                    }
                    let delay = backoff(attempt, self.base_delay_ms, self.max_delay_ms);
                    tracing::warn!(
                        attempt = attempt + 1,
                        max_retries = self.max_retries,
                        delay_ms = delay.as_millis() as u64,
                        error_kind = "transient",
                        "retrying transient failure"
                    );
                    sleep(delay).await;
                }
            }
        }
        // The loop body always returns on the final iteration, so this point
        // is unreachable.
        unreachable!("retry loop always returns on the final attempt")
    }

    async fn stream(&self, req: &ChatRequest) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        for attempt in 0..=self.max_retries {
            match self.inner.stream(req).await {
                Ok(stream) => {
                    if attempt > 0 {
                        tracing::info!(attempt, "retry succeeded (stream)");
                    }
                    return Ok(stream);
                }
                Err(e) => {
                    if !(should_retry(&e) && attempt < self.max_retries) {
                        return Err(e);
                    }
                    let delay = backoff(attempt, self.base_delay_ms, self.max_delay_ms);
                    tracing::warn!(
                        attempt = attempt + 1,
                        max_retries = self.max_retries,
                        delay_ms = delay.as_millis() as u64,
                        error_kind = "transient",
                        "retrying transient failure (stream)"
                    );
                    sleep(delay).await;
                }
            }
        }
        unreachable!("retry loop always returns on the final attempt")
    }

    async fn embed(&self, req: &EmbeddingRequest) -> Result<EmbeddingResponse> {
        for attempt in 0..=self.max_retries {
            match self.inner.embed(req).await {
                Ok(resp) => {
                    if attempt > 0 {
                        tracing::info!(attempt, "retry succeeded (embeddings)");
                    }
                    return Ok(resp);
                }
                Err(e) => {
                    if !(should_retry(&e) && attempt < self.max_retries) {
                        return Err(e);
                    }
                    let delay = backoff(attempt, self.base_delay_ms, self.max_delay_ms);
                    tracing::warn!(
                        attempt = attempt + 1,
                        max_retries = self.max_retries,
                        delay_ms = delay.as_millis() as u64,
                        error_kind = "transient",
                        "retrying transient failure (embeddings)"
                    );
                    sleep(delay).await;
                }
            }
        }
        unreachable!("retry loop always returns on the final attempt")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use futures::stream;
    use futures::StreamExt;

    // ── Mock provider that fails N times then succeeds ──

    struct FlakyProvider {
        fail_count: Arc<std::sync::atomic::AtomicU32>,
        max_fails: u32,
    }

    impl FlakyProvider {
        fn new(max_fails: u32) -> Self {
            Self {
                fail_count: Arc::new(std::sync::atomic::AtomicU32::new(0)),
                max_fails,
            }
        }
    }

    #[async_trait]
    impl Provider for FlakyProvider {
        async fn chat(&self, _req: &ChatRequest) -> Result<ChatResponse> {
            let fails = self
                .fail_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if fails < self.max_fails {
                Err(LlmError::Api {
                    status: 502,
                    message: "bad gateway".to_string(),
                })
            } else {
                Ok(ChatResponse {
                    content: "ok".to_string(),
                    model: "test".to_string(),
                    usage: None,
                    ..Default::default()
                })
            }
        }

        async fn stream(
            &self,
            _req: &ChatRequest,
        ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
            let fails = self
                .fail_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if fails < self.max_fails {
                Err(LlmError::Api {
                    status: 502,
                    message: "bad gateway".to_string(),
                })
            } else {
                Ok(Box::pin(stream::once(async {
                    Ok(StreamChunk {
                        delta: "ok".to_string(),
                        done: true,
                        ..Default::default()
                    })
                })))
            }
        }
    }

    fn dummy_req() -> ChatRequest {
        ChatRequest::new("test", "hello")
    }

    #[tokio::test]
    async fn succeeds_on_first_try() {
        let provider = Arc::new(FlakyProvider::new(0));
        let retry = RetryProvider::new(provider, 3);
        let resp = retry.chat(&dummy_req()).await.unwrap();
        assert_eq!(resp.content, "ok");
    }

    #[tokio::test]
    async fn retries_and_succeeds() {
        let provider = Arc::new(FlakyProvider::new(2));
        let retry = RetryProvider::new(provider, 3);
        let resp = retry.chat(&dummy_req()).await.unwrap();
        assert_eq!(resp.content, "ok");
    }

    #[tokio::test]
    async fn exhausts_retries() {
        let provider = Arc::new(FlakyProvider::new(10));
        let retry = RetryProvider::new(provider, 2); // only 2 retries
        let err = retry.chat(&dummy_req()).await.unwrap_err();
        assert!(matches!(err, LlmError::Api { status: 502, .. }));
    }

    #[tokio::test]
    async fn does_not_retry_4xx() {
        struct FourHundredProvider;

        #[async_trait]
        impl Provider for FourHundredProvider {
            async fn chat(&self, _: &ChatRequest) -> Result<ChatResponse> {
                Err(LlmError::Api {
                    status: 400,
                    message: "bad request".to_string(),
                })
            }
            async fn stream(
                &self,
                _: &ChatRequest,
            ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
                Err(LlmError::Api {
                    status: 400,
                    message: "bad request".to_string(),
                })
            }
        }

        let retry = RetryProvider::new(Arc::new(FourHundredProvider), 3);
        let err = retry.chat(&dummy_req()).await.unwrap_err();
        assert!(matches!(err, LlmError::Api { status: 400, .. }));
    }

    #[tokio::test]
    async fn stream_retries_and_succeeds() {
        let provider = Arc::new(FlakyProvider::new(2));
        let retry = RetryProvider::new(provider, 3);
        let mut stream = retry.stream(&dummy_req()).await.unwrap();
        let chunk = stream.next().await.unwrap().unwrap();
        assert_eq!(chunk.delta, "ok");
    }

    #[tokio::test]
    async fn backoff_produces_increasing_delays() {
        // With random jitter the exact values vary, but the *range* of
        // successive attempts must trend upward.
        for _ in 0..20 {
            let d0 = backoff(0, 500, 30_000);
            let _d1 = backoff(1, 500, 30_000);
            let d2 = backoff(2, 500, 30_000);
            // Upper bound of attempt-0 range < lower bound of attempt-2 range
            assert!(d0 <= Duration::from_millis(500));
            assert!(d2 >= Duration::from_millis(1500));
        }
    }

    #[tokio::test]
    async fn backoff_respects_max() {
        let d = backoff(30, 500, 5_000);
        assert!(d <= Duration::from_millis(5_000));
    }

    #[tokio::test]
    async fn backoff_jitter_is_random() {
        // Call backoff with the same attempt many times; at least some values
        // must differ (probability of all-equal with a real RNG is negligible).
        let values: Vec<u64> = (0..50)
            .map(|_| backoff(3, 500, 30_000).as_millis() as u64)
            .collect();
        let first = values[0];
        assert!(
            values.iter().any(|&v| v != first),
            "jitter should produce varying values, got all {first}"
        );
    }

    // ── embed retry tests ─────────────────────────────────────────

    struct EmbedOkProvider;

    #[async_trait]
    impl Provider for EmbedOkProvider {
        async fn chat(&self, _: &ChatRequest) -> Result<ChatResponse> {
            unimplemented!()
        }

        async fn stream(&self, _: &ChatRequest) -> Result<BoxStream<'static, Result<StreamChunk>>> {
            unimplemented!()
        }

        async fn embed(&self, req: &EmbeddingRequest) -> Result<EmbeddingResponse> {
            Ok(EmbeddingResponse {
                model: req.model.clone(),
                data: vec![crate::Embedding {
                    index: 0,
                    embedding: vec![0.1, 0.2],
                }],
                usage: None,
            })
        }
    }

    /// A provider that only implements chat/stream — embed uses the default unsupported.
    struct EmbedUnsupportedProvider;

    #[async_trait]
    impl Provider for EmbedUnsupportedProvider {
        async fn chat(&self, _: &ChatRequest) -> Result<ChatResponse> {
            Ok(ChatResponse::default())
        }

        async fn stream(&self, _: &ChatRequest) -> Result<BoxStream<'static, Result<StreamChunk>>> {
            Ok(Box::pin(stream::empty()))
        }
        // embed() is NOT overridden — uses default Unsupported
    }

    #[tokio::test]
    async fn retry_provider_delegates_embed() {
        let provider = Arc::new(EmbedOkProvider);
        let retry = RetryProvider::new(provider, 0);
        let resp = retry
            .embed(&EmbeddingRequest::new("test", "hello"))
            .await
            .expect("embed should succeed through RetryProvider");
        assert_eq!(resp.data.len(), 1);
        assert_eq!(resp.data[0].embedding, vec![0.1, 0.2]);
    }

    #[tokio::test]
    async fn retry_provider_keeps_unsupported_for_chat_only_provider() {
        let provider = Arc::new(EmbedUnsupportedProvider);
        let retry = RetryProvider::new(provider, 0);
        let err = retry
            .embed(&EmbeddingRequest::new("test", "hello"))
            .await
            .unwrap_err();
        assert!(
            matches!(err, LlmError::Unsupported { .. }),
            "chat-only provider should return Unsupported through RetryProvider, got: {err:?}"
        );
    }

    struct FlakyEmbedProvider {
        fail_count: Arc<std::sync::atomic::AtomicU32>,
        max_fails: u32,
    }

    impl FlakyEmbedProvider {
        fn new(max_fails: u32) -> Self {
            Self {
                fail_count: Arc::new(std::sync::atomic::AtomicU32::new(0)),
                max_fails,
            }
        }
    }

    #[async_trait]
    impl Provider for FlakyEmbedProvider {
        async fn chat(&self, _: &ChatRequest) -> Result<ChatResponse> {
            unimplemented!()
        }

        async fn stream(&self, _: &ChatRequest) -> Result<BoxStream<'static, Result<StreamChunk>>> {
            unimplemented!()
        }

        async fn embed(&self, req: &EmbeddingRequest) -> Result<EmbeddingResponse> {
            let fails = self
                .fail_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if fails < self.max_fails {
                Err(LlmError::Api {
                    status: 502,
                    message: "bad gateway".into(),
                })
            } else {
                Ok(EmbeddingResponse {
                    model: req.model.clone(),
                    data: vec![crate::Embedding {
                        index: 0,
                        embedding: vec![1.0],
                    }],
                    usage: None,
                })
            }
        }
    }

    #[tokio::test]
    async fn retry_provider_retries_transient_embedding_failure() {
        let provider = Arc::new(FlakyEmbedProvider::new(1)); // fail 1st, succeed 2nd
        let retry = RetryProvider::new(provider, 1);
        let resp = retry
            .embed(&EmbeddingRequest::new("test", "hello"))
            .await
            .expect("embed should succeed after retry");
        assert_eq!(resp.data[0].embedding, vec![1.0]);
    }
}
