//! Multi-deployment routing with automatic fallback.
//!
//! [`Router`] sits on top of an [`LmrsClient`] and maps a logical *group* name
//! to an ordered list of concrete `provider/model` deployments. When a request
//! targets a group, the router tries deployments in turn and returns the first
//! success, transparently falling back when a deployment fails with a transient
//! error. This mirrors LiteLLM's Router: hide many deployments behind a single
//! name and get failover — plus optional round-robin load balancing — for free.
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
//! #[tokio::main]
//! async fn main() {
//!     let client = Arc::new(LmrsClient::new());
//!     client.set_openai("sk-...").await;
//!     client.set_anthropic("sk-ant-...").await;
//!
//!     let router = Router::new(client).route(
//!         "smart",
//!         ["openai/gpt-4o", "anthropic/claude-sonnet-4-20250514"],
//!     );
//!
//!     // Tries gpt-4o first; on a transient failure, falls back to Claude.
//!     let resp = router.chat("smart", "Hello!").await.unwrap();
//!     println!("{}", resp.content);
//! }
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
    /// Always try deployments in registration order: the first is the primary,
    /// the rest are fallbacks. Fully deterministic.
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
/// on the **initial connection**. Once a byte stream is established, mid-stream
/// errors are propagated to the caller rather than retried on another
/// deployment.
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

        if base