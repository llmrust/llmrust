//! Multi-deployment routing with automatic fallback and optional
//! passive cooldown.
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
//! ## Passive cooldown
//!
//! Calling [`Router::with_cooldown`] enables opt-in passive cooldown: when a
//! deployment fails with a failoverable error (HTTP 5xx, 429, stream error,
//! network error, UnknownProvider), it enters a short cooldown period. During
//! cooldown, subsequent routing attempts prefer other deployments. Cooldown
//! expires after the configured duration; a subsequent successful call clears it
//! immediately.
//!
//! Cooldown is **passive** — no background health checker, no active pings.
//! Deployment state is updated only as a side effect of routing decisions.
//!
//! If all deployments in a group are in cooldown, the router fails open:
//! all deployments are still attempted in order rather than returning an error.
//!
//! Streaming is only affected on the initial connection. Once a stream is
//! established, mid-stream errors are the caller's responsibility.
//!
//! ## Example
//!
//! ```rust,no_run
//! use llmrust::{LmrsClient, Router};
//! use std::sync::Arc;
//! use std::time::Duration;
//!
//! let client = Arc::new(LmrsClient::new());
//! let router = Router::new(client)
//!     .with_cooldown(Duration::from_secs(30))
//!     .route(
//!         "smart",
//!         ["openai/gpt-4o", "anthropic/claude-sonnet-4-20250514"],
//!     );
//! // Then, inside an async context:
//! // let resp = router.chat("smart", "Hello!").await?;
//! let _ = router;
//! ```

use futures::stream::BoxStream;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
///
/// ## Differences from `RetryProvider::should_retry`
///
/// Both functions look similar but make **deliberately different choices** for
/// `429` and `UnknownProvider`:
///
/// - `should_failover` treats 429 as transient: switching deployments may land
///   on one that is not rate-limited.
/// - `should_failover` treats `UnknownProvider` as transient: a different
///   deployment in the group may have the missing provider registered.
/// - `should_retry` treats both as permanent: retrying the **same** deployment
///   won't help when rate-limited or when the provider is unregistered.
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
///
/// **Cooldown note:** when enabled via [`Router::with_cooldown`], deployments
/// that fail with a transient error are temporarily deprioritized. This is
/// passive — no background health check runs. Failed deployments are
/// automatically retried after the cooldown duration expires.
pub struct Router {
    client: Arc<LmrsClient>,
    groups: HashMap<String, Vec<String>>,
    strategy: RoutingStrategy,
    counter: AtomicUsize,
    cooldown: Option<Duration>,
    cooldown_until: Mutex<HashMap<String, Instant>>,
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
            cooldown: None,
            cooldown_until: Mutex::new(HashMap::new()),
        }
    }

    /// Set the routing strategy (defaults to [`RoutingStrategy::Ordered`]).
    pub fn with_strategy(mut self, strategy: RoutingStrategy) -> Self {
        self.strategy = strategy;
        self
    }

    /// Enable passive cooldown with the given duration.
    ///
    /// When a deployment fails with a failoverable error, it enters cooldown
    /// for `duration`. Subsequent routing within that window deprioritizes
    /// this deployment. A duration of zero disables cooldown (equivalent to
    /// the default).
    ///
    /// Cooldown is opt-in: the default Router behavior is unchanged.
    pub fn with_cooldown(mut self, duration: Duration) -> Self {
        if duration > Duration::ZERO {
            self.cooldown = Some(duration);
        }
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

    // ── cooldown helpers ──────────────────────────────────────────

    fn is_in_cooldown(&self, deployment: &str) -> bool {
        if self.cooldown.is_none() {
            return false;
        }
        let guard = self
            .cooldown_until
            .lock()
            .expect("cooldown lock not poisoned");
        match guard.get(deployment) {
            Some(deadline) => Instant::now() < *deadline,
            None => false,
        }
    }

    fn mark_cooldown(&self, deployment: &str) {
        let dur = match self.cooldown {
            Some(d) => d,
            None => return,
        };
        let mut guard = self
            .cooldown_until
            .lock()
            .expect("cooldown lock not poisoned");
        guard.insert(deployment.to_string(), Instant::now() + dur);
    }

    fn clear_cooldown(&self, deployment: &str) {
        let mut guard = self
            .cooldown_until
            .lock()
            .expect("cooldown lock not poisoned");
        guard.remove(deployment);
    }

    /// Return the ordered deployments to attempt for `group`, with cooldown
    /// filtering applied when enabled. If all deployments are in cooldown,
    /// returns the full list (fail-open).
    fn candidates(&self, group: &str) -> Vec<String> {
        let deployments = self.resolve(group);
        if self.cooldown.is_none() || deployments.len() <= 1 {
            return deployments;
        }

        let mut ready: Vec<String> = Vec::with_capacity(deployments.len());
        let mut cooling: Vec<String> = Vec::with_capacity(deployments.len());

        for d in deployments {
            if self.is_in_cooldown(&d) {
                cooling.push(d);
            } else {
                ready.push(d);
            }
        }

        if ready.is_empty() {
            // All deployments cooling — fail open
            cooling
        } else {
            ready
        }
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
        let deployments = self.candidates(group);
        tracing::debug!(group, deployments = ?deployments, "routing chat request");
        let mut last_error: Option<LlmError> = None;
        for model in &deployments {
            match self.client.chat_with(model, request.clone()).await {
                Ok(resp) => {
                    self.clear_cooldown(model);
                    return Ok(resp);
                }
                Err(e) if should_failover(&e) => {
                    tracing::warn!(
                        group,
                        model,
                        error_kind = "api_error",
                        "failing over to next deployment"
                    );
                    self.mark_cooldown(model);
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
        let deployments = self.candidates(group);
        tracing::debug!(group, deployments = ?deployments, "routing stream request");
        let mut last_error: Option<LlmError> = None;
        for model in &deployments {
            match self.client.stream_with(model, request.clone()).await {
                Ok(s) => {
                    self.clear_cooldown(model);
                    return Ok(s);
                }
                Err(e) if should_failover(&e) => {
                    tracing::warn!(
                        group,
                        model,
                        error_kind = "api_error",
                        "failing over to next deployment"
                    );
                    self.mark_cooldown(model);
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

    // ── cooldown tests ────────────────────────────────────────────

    #[tokio::test]
    async fn default_router_does_not_skip_failed_primary() {
        let client = Arc::new(LmrsClient::new());
        let primary = Arc::new(FailingProvider::new(500));
        let secondary = Arc::new(OkProvider::new("secondary"));
        client.set_custom("p", primary).await;
        client.set_custom("s", secondary).await;

        let router = Router::new(client).route("grp", ["p/m", "s/m"]);
        // 1st request: primary fails, secondary succeeds
        let r1 = router.chat("grp", "hi").await.unwrap();
        assert_eq!(r1.content, "secondary");

        // 2nd request: without cooldown, primary is tried again (and fails over to secondary)
        let r2 = router.chat("grp", "hi").await.unwrap();
        assert_eq!(r2.content, "secondary");
    }

    #[tokio::test]
    async fn cooldown_skips_recently_failed_primary() {
        let client = Arc::new(LmrsClient::new());
        let primary = Arc::new(FailingProvider::new(500));
        let secondary = Arc::new(OkProvider::new("secondary"));
        client.set_custom("p", primary).await;
        client.set_custom("s", secondary).await;

        let router = Router::new(client)
            .with_cooldown(Duration::from_secs(60))
            .route("grp", ["p/m", "s/m"]);

        // 1st request: primary fails, marked cooldown, secondary succeeds
        let r1 = router.chat("grp", "hi").await.unwrap();
        assert_eq!(r1.content, "secondary");

        // 2nd request: primary in cooldown, skipped; secondary succeeds
        let r2 = router.chat("grp", "hi").await.unwrap();
        assert_eq!(r2.content, "secondary");
    }

    #[tokio::test]
    async fn cooldown_expires_and_retries_primary() {
        let client = Arc::new(LmrsClient::new());

        struct StatefulProvider {
            name: String,
            has_failed: Mutex<bool>,
        }

        #[async_trait]
        impl crate::providers::Provider for StatefulProvider {
            async fn chat(&self, _req: &ChatRequest) -> Result<ChatResponse> {
                let mut guard = self.has_failed.lock().unwrap();
                if !*guard {
                    *guard = true;
                    Err(LlmError::Api {
                        status: 500,
                        message: "simulated failure".into(),
                    })
                } else {
                    Ok(ChatResponse {
                        content: self.name.clone(),
                        model: "m".into(),
                        ..Default::default()
                    })
                }
            }

            async fn stream(
                &self,
                _req: &ChatRequest,
            ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
                let mut guard = self.has_failed.lock().unwrap();
                if !*guard {
                    *guard = true;
                    Err(LlmError::Api {
                        status: 500,
                        message: "simulated failure".into(),
                    })
                } else {
                    let name = self.name.clone();
                    Ok(Box::pin(stream::once(async move {
                        Ok(StreamChunk {
                            delta: name,
                            done: true,
                            ..Default::default()
                        })
                    })))
                }
            }
        }

        let primary = Arc::new(StatefulProvider {
            name: "primary-ok".into(),
            has_failed: Mutex::new(false),
        });
        let secondary = Arc::new(OkProvider::new("secondary"));
        client.set_custom("p", primary).await;
        client.set_custom("s", secondary).await;

        let router = Router::new(client)
            .with_cooldown(Duration::from_millis(30))
            .route("grp", ["p/m", "s/m"]);

        // 1st request: primary fails (500), enters cooldown, secondary succeeds
        let r1 = router.chat("grp", "hi").await.unwrap();
        assert_eq!(r1.content, "secondary");

        // Wait for cooldown to expire
        tokio::time::sleep(Duration::from_millis(50)).await;

        // 2nd request: cooldown expired, retries primary — it now succeeds
        let r2 = router.chat("grp", "hi").await.unwrap();
        assert_eq!(r2.content, "primary-ok");
    }

    #[tokio::test]
    async fn cooldown_not_marked_for_parse_error() {
        let client = Arc::new(LmrsClient::new());
        let bad = Arc::new(FailingProvider::new(400)); // 400 is not failoverable
        let good = Arc::new(OkProvider::new("secondary"));
        client.set_custom("bad", bad).await;
        client.set_custom("good", good).await;

        let router = Router::new(client)
            .with_cooldown(Duration::from_secs(60))
            .route("grp", ["bad/m", "good/m"]);

        let err = router.chat("grp", "hi").await.unwrap_err();
        assert!(matches!(err, LlmError::Api { status: 400, .. }));
    }

    #[tokio::test]
    async fn cooldown_fail_open_when_all_deployments_are_cooling() {
        let client = Arc::new(LmrsClient::new());
        let p1 = Arc::new(FailingProvider::new(503));
        let p2 = Arc::new(FailingProvider::new(502));
        let p3 = Arc::new(FailingProvider::new(500));
        client.set_custom("p1", p1).await;
        client.set_custom("p2", p2).await;
        client.set_custom("p3", p3).await;

        let router = Router::new(client)
            .with_cooldown(Duration::from_secs(60))
            .route("grp", ["p1/m", "p2/m", "p3/m"]);

        // First request: all fail, all enter cooldown
        let err1 = router.chat("grp", "hi").await.unwrap_err();
        assert!(matches!(err1, LlmError::Api { .. }));

        // Second request: all in cooldown, must fail-open (still attempt all)
        let err2 = router.chat("grp", "hi").await.unwrap_err();
        assert!(matches!(err2, LlmError::Api { .. }));
    }

    #[tokio::test]
    async fn stream_initial_failure_marks_cooldown() {
        let client = Arc::new(LmrsClient::new());
        let bad = Arc::new(FailingProvider::new(502));
        let good = Arc::new(OkProvider::new("stream-good"));
        client.set_custom("bad", bad).await;
        client.set_custom("good", good).await;

        let router = Router::new(client)
            .with_cooldown(Duration::from_secs(60))
            .route("grp", ["bad/m", "good/m"]);

        // 1st stream: bad fails, marked cooldown, good succeeds
        let mut s1 = router.stream("grp", "hi").await.unwrap();
        let c1 = s1.next().await.unwrap().unwrap();
        assert_eq!(c1.delta, "stream-good");

        // 2nd stream: bad in cooldown, skipped; good succeeds again
        let mut s2 = router.stream("grp", "hi").await.unwrap();
        let c2 = s2.next().await.unwrap().unwrap();
        assert_eq!(c2.delta, "stream-good");
    }

    #[tokio::test]
    async fn successful_deployment_clears_cooldown() {
        let client = Arc::new(LmrsClient::new());
        let primary = Arc::new(FailingProvider::new(503));
        let secondary = Arc::new(OkProvider::new("secondary"));
        client.set_custom("p", primary).await;
        client.set_custom("s", secondary).await;

        let router = Router::new(client)
            .with_cooldown(Duration::from_millis(30))
            .route("grp", ["p/m", "s/m"]);

        // 1st request: primary fails, enters cooldown
        let r1 = router.chat("grp", "hi").await.unwrap();
        assert_eq!(r1.content, "secondary");

        // Wait for cooldown to expire
        tokio::time::sleep(Duration::from_millis(50)).await;

        // 2nd request: primary out of cooldown, but still FailingProvider → fails again
        // secondary succeeds. primary re-enters cooldown.
        let r2 = router.chat("grp", "hi").await.unwrap();
        assert_eq!(r2.content, "secondary");

        // primary’s cooldown is re-marked — still in effect
        let r3 = router.chat("grp", "hi").await.unwrap();
        assert_eq!(r3.content, "secondary");
    }
}
