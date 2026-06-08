//! Multi-deployment routing with automatic fallback.
//!
//! [`Router`] sits on top of an [`LmrsClient`] and maps a logical *group* name
//! to an ordered list of concrete `provider/model` deployments. A request to a
//! group tries deployments in turn and returns the first success, transparently
//! falling back when a deployment fails with a transient error. This mirrors
//! LiteLLM's Router: hide many deployments behind one name and get failover
//! (plus optional round-robin load balancing) for free.
//!
//! Unknown group names are treated as a single literal `provider/model`
//! deployment, so a [`Router`] is a drop-in replacement for calling
//! [`LmrsClient`] directly.
//!
//! # Example
//!
//! ```rust,no_run
//! use llmrust::{LmrsClient, Router};
//! use std::sync::Arc;
//!
//! let client = Arc::new(LmrsClient::new());
//! let router = Router::new(client).route(
//!     "smart",
//!     ["openai/gpt-4o", "anthropic/claude-sonnet-4-20250514"],
//! );
//! // Then, inside an async context:
//! // let resp = router.chat("smart", "Hello!").await?;
//! let _ = router;
//! ```

use futures::stream::BoxStream;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crate::providers::{LlmError, Result};
use crate::types::{ChatRequest, ChatResponse, StreamChunk};
use crate::LmrsClient;

/// How a [`Router`] chooses the starting deployment within a group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RoutingStrategy {
    /// Always try deployments in registration order: the first is primary, the
    /// rest are fallbacks. Fully deterministic.
    #[default]
    Ordered,
    /// Rotate the starting deployment on each call (round-robin) to spread
    /// load, then fall back through the remaining deployments in order.
    RoundRobin,
}

/// Returns `true` when an error should trigger a fallback to the next
/// deployment. Transient / server-side failures fail over; client mistakes
/// (4xx other than 429) and parse errors are treated as permanent.
fn should_failover(e: &LlmError) -> bool {
    match e {
        LlmError::Http(_) => true,
        LlmError::Stream(_) => true,
        LlmError::Api { status, .. } => *status >= 500 || *status == 429,
        // A deployment whose provider isn't registered is skipped so the
        // remaining deployments still get a chance.
        LlmError::UnknownProvider(_) => true,
        LlmError::Parse(_) => false,
    }
}

fn no_deployments(group: &str) -> LlmError {
    LlmError::UnknownProvider(format!("no deployments configured for group '{}'", group))
}

/// A failover / load-balancing router layered over an [`LmrsClient`].
///
/// **Streaming note:** like [`crate::RetryProvider`], `stream*` only fails over
/// on the **initial connection**. Once a stream is established, mid-stream
/// errors propagate to the caller rather than retrying on another deployment.
pub struct Router {
    client: Arc<LmrsClient>,
    groups: HashMap<String, Vec<String>>,
    strategy: RoutingStrategy,
    counter: AtomicUsize,
}

impl Router {
    /// Create a router over an existing [`LmrsClient`]. Register deployment
    /// groups with [`Router::route`].
    pub fn new(client: Arc<LmrsClient>) -> Self {
        Self {
            client,
            groups: HashMap::new(),
            strategy: RoutingStrategy::Ordered,
            counter: AtomicUsize::new(0),
        }
    }

    /// Set the routing strategy (defaults to [`RoutingStrategy::Ordered`]).
    pub fn with_strategy(mut self, strategy: RoutingStrategy) -> Self {
        self.strategy = strategy;
        self
    }

    /// Register a group mapping a logical name to an ordered list of
    /// `provider/model` deployments (primary first, then fallbacks).
    pub fn route<I, S>(mut self, group: impl Into<String>, deployments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let list: Vec<String> = deployments.into_iter().map(Into::into).collect();
        self.groups.insert(group.into(), list);
        self
    }

    /// Resolve the ordered deployments to attempt for `group`, applying the
    /// routing strategy. An unregistered group resolves to itself, so the
    /// router transparently forwards plain `provider/model` strings.
    fn resolve(&self, group: &str) -> Vec<String> {
        let base = match self.groups.get(group) {
            Some(list) => list.clone(),
            None => vec![group.to_string()],
        };

        if base.len() <= 1 {
            return base;
        }

        match self.strategy {
            RoutingStrategy::Ordered => base,
            RoutingStrategy::RoundRobin => {
                let start = self.counter.fetch_add(1, Ordering::Relaxed) % base.len();
                let mut rotated = base[start..].to_vec();
                rotated.extend_from_slice(&base[..start]);
                rotated
            }
        }
    }

    /// Send a chat request to `group` using a simple text prompt.
    pub async fn chat(&self, group: &str, prompt: &str) -> Result<ChatResponse> {
        self.chat_with(group, ChatRequest::new("", prompt)).await
    }

    /// Send a fully-specified chat request to `group`, failing over across
    /// deployments. The request's `model` field is overwritten per deployment.
    pub async fn chat_with(&self, group: &str, request: ChatRequest) -> Result<ChatResponse> {
        let deployments = self.resolve(group);
        tracing::debug!(group, deployments = ?deployments, "routing chat request");
        let mut last_error: Option<LlmError> = None;
        for model in &deployments {
            match self.client.chat_with(model, request.clone()).await {
                Ok(resp) => return Ok(resp),
                Err(e) if should_failover(&e) => {
                    tracing::warn!(group, model, error = %e, "failing over to next deployment");
                    last_error = Some(e);
                }
                Err(e) => return Err(e),
            }
        }
        Err(last_error.unwrap_or_else(|| no_deployments(group)))
    }

    /// Open a streaming chat for `group` using a simple text prompt.
    pub async fn stream(
        &self,
        group: &str,
        prompt: &str,
    ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        self.stream_with(group, ChatRequest::new("", prompt)).await
    }

    /// Open a streaming chat for `group`, failing over across deployments on
    /// the initial connection only.
    pub async fn stream_with(
        &self,
        group: &str,
        request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        let deployments = self.resolve(group);
        tracing::debug!(group, deployments = ?deployments, "routing stream request");
        let mut last_error: Option<LlmError> = None;
        for model in &deployments {
            match self.client.stream_with(model, request.clone()).await {
                Ok(s) => return Ok(s),
                Err(e) if should_failover(&e) => {
                    tracing::warn!(group, model, error = %e, "failing over to next deployment");
                    last_error = Some(e);
                }
                Err(e) => return Err(e),
            }
        }
        Err(last_error.unwrap_or_else(|| no_deployments(group)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use futures::stream;
    use futures::StreamExt;

    struct FailingProvider {
        status: u16,
    }

    impl FailingProvider {
        fn new(status: u16) -> Self {
            Self { status }
        }
    }

    #[async_trait]
    impl crate::providers::Provider for FailingProvider {
        async fn chat(&self, _req: &ChatRequest) -> Result<ChatResponse> {
            Err(LlmError::Api {
                status: self.status,
                message: "fail".to_string(),
            })
        }

        async fn stream(
            &self,
            _req: &ChatRequest,
        ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
            Err(LlmError::Api {
                status: self.status,
                message: "fail".to_string(),
            })
        }
    }

    struct OkProvider {
        name: String,
    }

    impl OkProvider {
        fn new(name: impl Into<String>) -> Self {
            Self { name: name.into() }
        }
    }

    #[async_trait]
    impl crate::providers::Provider for OkProvider {
        async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse> {
            Ok(ChatResponse {
                content: self.name.clone(),
                model: req.model.clone(),
                ..Default::default()
            })
        }

        async fn stream(
            &self,
            _req: &ChatRequest,
        ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
            let chunk = StreamChunk {
                delta: self.name.clone(),
                done: true,
                ..Default::default()
            };
            Ok(Box::pin(stream::once(async move { Ok(chunk) })))
        }
    }

    #[tokio::test]
    async fn falls_back_on_transient_error() {
        let client = Arc::new(LmrsClient::new());
        let bad = Arc::new(FailingProvider::new(503));
        let good = Arc::new(OkProvider::new("good"));
        client.set_custom("bad", bad).await;
        client.set_custom("good", good).await;

        let router = Router::new(client).route("grp", ["bad/m1", "good/m2"]);
        let resp = router.chat("grp", "hi").await.unwrap();
        assert_eq!(resp.content, "good");
    }

    #[tokio::test]
    async fn permanent_error_is_not_retried() {
        let client = Arc::new(LmrsClient::new());
        let bad = Arc::new(FailingProvider::new(400));
        let good = Arc::new(OkProvider::new("good"));
        client.set_custom("bad", bad).await;
        client.set_custom("good", good).await;

        let router = Router::new(client).route("grp", ["bad/m1", "good/m2"]);
        let err = router.chat("grp", "hi").await.unwrap_err();
        assert!(matches!(err, LlmError::Api { status: 400, .. }));
    }

    #[tokio::test]
    async fn all_deployments_fail_returns_last_error() {
        let client = Arc::new(LmrsClient::new());
        let bad1 = Arc::new(FailingProvider::new(500));
        let bad2 = Arc::new(FailingProvider::new(503));
        client.set_custom("b1", bad1).await;
        client.set_custom("b2", bad2).await;

        let router = Router::new(client).route("grp", ["b1/m", "b2/m"]);
        let err = router.chat("grp", "hi").await.unwrap_err();
        assert!(matches!(err, LlmError::Api { status: 503, .. }));
    }

    #[tokio::test]
    async fn unknown_group_routes_directly() {
        let client = Arc::new(LmrsClient::new());
        let good = Arc::new(OkProvider::new("good"));
        client.set_custom("good", good).await;

        let router = Router::new(client);
        let resp = router.chat("good/gpt", "hi").await.unwrap();
        assert_eq!(resp.content, "good");
    }

    #[tokio::test]
    async fn round_robin_rotates_start() {
        let client = Arc::new(LmrsClient::new());
        let a = Arc::new(OkProvider::new("a"));
        let b = Arc::new(OkProvider::new("b"));
        client.set_custom("a", a).await;
        client.set_custom("b", b).await;

        let router = Router::new(client)
            .with_strategy(RoutingStrategy::RoundRobin)
            .route("grp", ["a/m", "b/m"]);
        let r0 = router.chat("grp", "hi").await.unwrap();
        let r1 = router.chat("grp", "hi").await.unwrap();
        let r2 = router.chat("grp", "hi").await.unwrap();
        assert_eq!(r0.content, "a");
        assert_eq!(r1.content, "b");
        assert_eq!(r2.content, "a");
    }

    #[tokio::test]
    async fn stream_falls_back_on_transient_error() {
        let client = Arc::new(LmrsClient::new());
        let bad = Arc::new(FailingProvider::new(502));
        let good = Arc::new(OkProvider::new("stream-ok"));
        client.set_custom("bad", bad).await;
        client.set_custom("good", good).await;

        let router = Router::new(client).route("grp", ["bad/m", "good/m"]);
        let mut s = router.stream("grp", "hi").await.unwrap();
        let chunk = s.next().await.unwrap().unwrap();
        assert_eq!(chunk.delta, "stream-ok");
    }
}
