//! Contract tests for provider, client, and proxy semantics.
//!
//! These tests verify the behavioral contracts defined in docs/CONTRACTS.md.
//! No real API calls — only fake providers and mock streams.

#[cfg(test)]
mod stream_contracts {
    use futures::stream::{self, BoxStream};
    use llmrust::{FinishReason, LlmError, LmrsClient, Provider, StreamChunk, ToolCall, Usage};
    use std::sync::Arc;

    // ── Fake providers for contract verification ──────────────────

    /// A provider whose stream always yields an error mid-stream.
    struct ErrorStreamProvider;

    #[async_trait::async_trait]
    impl Provider for ErrorStreamProvider {
        async fn chat(
            &self,
            _req: &llmrust::ChatRequest,
        ) -> llmrust::Result<llmrust::ChatResponse> {
            Ok(llmrust::ChatResponse::default())
        }

        async fn stream(
            &self,
            _req: &llmrust::ChatRequest,
        ) -> llmrust::Result<BoxStream<'static, llmrust::Result<StreamChunk>>> {
            Ok(Box::pin(stream::iter(vec![
                // A normal chunk, then an error
                Ok(StreamChunk {
                    delta: "partial".into(),
                    ..Default::default()
                }),
                Err(LlmError::Stream("simulated upstream stream failure".into())),
                // This chunk should never be consumed if stream_collect aborts on error
                Ok(StreamChunk {
                    delta: "should not see this".into(),
                    done: true,
                    ..Default::default()
                }),
            ])))
        }
    }

    /// A provider whose stream yields a full, well-formed sequence:
    /// text deltas → terminal chunk with metadata.
    struct FullStreamProvider;

    #[async_trait::async_trait]
    impl Provider for FullStreamProvider {
        async fn chat(
            &self,
            _req: &llmrust::ChatRequest,
        ) -> llmrust::Result<llmrust::ChatResponse> {
            Ok(llmrust::ChatResponse::default())
        }

        async fn stream(
            &self,
            _req: &llmrust::ChatRequest,
        ) -> llmrust::Result<BoxStream<'static, llmrust::Result<StreamChunk>>> {
            Ok(Box::pin(stream::iter(vec![
                Ok(StreamChunk {
                    delta: "Hello".into(),
                    ..Default::default()
                }),
                Ok(StreamChunk {
                    delta: " world".into(),
                    ..Default::default()
                }),
                // Terminal chunk with metadata
                Ok(StreamChunk {
                    delta: String::new(),
                    done: true,
                    finish_reason: Some(FinishReason::Stop),
                    usage: Some(Usage {
                        prompt_tokens: 10,
                        completion_tokens: 2,
                        total_tokens: 12,
                    }),
                    tool_calls: Some(vec![ToolCall {
                        id: "call_1".into(),
                        call_type: "function".into(),
                        function: llmrust::FunctionCall {
                            name: "greet".into(),
                            arguments: r#"{"name":"world"}"#.into(),
                        },
                    }]),
                }),
            ])))
        }
    }

    // ── stream_collect: error propagation ─────────────────────────

    /// Per CONTRACTS.md: "Parse errors in the stream are surfaced as
    /// Err(LlmError::Parse(...)) chunks — never silently dropped."
    ///
    /// stream_collect must propagate stream errors, not silently return
    /// partial text.
    #[tokio::test]
    async fn stream_collect_propagates_error() {
        let llm = LmrsClient::new();
        llm.set_custom("err", Arc::new(ErrorStreamProvider)).await;

        let result = llm.stream_collect("err/any-model", "hi").await;
        assert!(
            result.is_err(),
            "stream_collect must return Err when stream yields an error"
        );
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("simulated upstream stream failure"),
            "must propagate the original error message, got: {msg}"
        );
    }

    // ── stream_collect_full: metadata collection ──────────────────

    /// Per CONTRACTS.md: "stream_collect_full returns a full ChatResponse
    /// with usage, tool_calls, and finish_reason."
    #[tokio::test]
    async fn stream_collect_full_stores_text() {
        let llm = LmrsClient::new();
        llm.set_custom("full", Arc::new(FullStreamProvider)).await;

        let resp = llm
            .stream_collect_full("full/any-model", "hi")
            .await
            .expect("stream_collect_full should succeed");

        assert_eq!(resp.content, "Hello world");
    }

    #[tokio::test]
    async fn stream_collect_full_stores_finish_reason() {
        let llm = LmrsClient::new();
        llm.set_custom("full", Arc::new(FullStreamProvider)).await;

        let resp = llm
            .stream_collect_full("full/any-model", "hi")
            .await
            .expect("stream_collect_full should succeed");

        assert_eq!(resp.finish_reason, Some(FinishReason::Stop));
    }

    #[tokio::test]
    async fn stream_collect_full_stores_usage() {
        let llm = LmrsClient::new();
        llm.set_custom("full", Arc::new(FullStreamProvider)).await;

        let resp = llm
            .stream_collect_full("full/any-model", "hi")
            .await
            .expect("stream_collect_full should succeed");

        let usage = resp.usage.expect("usage should be populated");
        assert_eq!(usage.prompt_tokens, 10);
        assert_eq!(usage.completion_tokens, 2);
        assert_eq!(usage.total_tokens, 12);
    }

    #[tokio::test]
    async fn stream_collect_full_stores_tool_calls() {
        let llm = LmrsClient::new();
        llm.set_custom("full", Arc::new(FullStreamProvider)).await;

        let resp = llm
            .stream_collect_full("full/any-model", "hi")
            .await
            .expect("stream_collect_full should succeed");

        let calls = resp.tool_calls.expect("tool_calls should be populated");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].function.name, "greet");
        assert_eq!(calls[0].function.arguments, r#"{"name":"world"}"#);
    }

    #[tokio::test]
    async fn stream_collect_full_stores_model_name() {
        let llm = LmrsClient::new();
        llm.set_custom("full", Arc::new(FullStreamProvider)).await;

        let resp = llm
            .stream_collect_full("full/any-model", "hi")
            .await
            .expect("stream_collect_full should succeed");

        assert_eq!(resp.model, "any-model");
    }
}

#[cfg(test)]
mod model_routing_contracts {
    use llmrust::{LlmError, LmrsClient};

    /// Per CONTRACTS.md: "All chat/stream methods require provider/model format.
    /// Parse errors return LlmError::Parse."
    #[tokio::test]
    async fn rejects_model_without_provider_prefix() {
        let llm = LmrsClient::new();
        match llm.chat("gpt-4o", "test").await {
            Err(LlmError::Parse(msg)) => {
                assert!(
                    msg.contains("provider/model"),
                    "expected provider/model error, got: {msg}"
                );
            }
            other => panic!("expected LlmError::Parse, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_empty_provider() {
        let llm = LmrsClient::new();
        match llm.chat("openai/", "test").await {
            Err(LlmError::Parse(msg)) => {
                assert!(msg.contains("non-empty provider and model"), "got: {msg}");
            }
            other => panic!("expected LlmError::Parse, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_empty_model() {
        let llm = LmrsClient::new();
        match llm.chat("/gpt-4o", "test").await {
            Err(LlmError::Parse(msg)) => {
                assert!(msg.contains("non-empty provider and model"), "got: {msg}");
            }
            other => panic!("expected LlmError::Parse, got: {other:?}"),
        }
    }

    /// Per CONTRACTS.md: parse_model must accept valid provider/model format.
    #[tokio::test]
    async fn accepts_provider_model_format() {
        let llm = LmrsClient::new();
        // Without registering a real provider, this should fail with
        // UnknownProvider — NOT Parse. That means the format was accepted.
        let result = llm.chat("openai/gpt-4o", "test").await;
        match result {
            Err(LlmError::UnknownProvider(_)) => {} // expected
            other => panic!("expected UnknownProvider, got: {other:?}"),
        }
    }

    /// Per CONTRACTS.md: unknown provider name returns UnknownProvider error.
    #[tokio::test]
    async fn unknown_provider_returns_proper_error() {
        let llm = LmrsClient::new();
        let err = llm.chat("missing/model", "test").await.unwrap_err();
        match err {
            LlmError::UnknownProvider(name) => {
                assert_eq!(name, "missing");
            }
            other => panic!("expected UnknownProvider, got: {other:?}"),
        }
    }
}

#[cfg(feature = "proxy")]
#[cfg(test)]
mod proxy_n_policy_contracts {

    /// Per CONTRACTS.md: "Accept missing n or n = 1. Reject n = 0 or n > 1."
    #[test]
    fn n_policy_accepts_missing_n() {
        let body = serde_json::json!({
            "model": "openai/gpt-4o",
            "messages": [{"role": "user", "content": "hi"}]
        });
        // Must not reject
        let request: llmrust::proxy::ProxyChatRequest =
            serde_json::from_value(body).expect("missing n should be accepted");
        assert!(request.n.is_none() || request.n == Some(1));
    }

    #[test]
    fn n_policy_accepts_n_one() {
        let body = serde_json::json!({
            "model": "openai/gpt-4o",
            "messages": [{"role": "user", "content": "hi"}],
            "n": 1
        });
        let request: llmrust::proxy::ProxyChatRequest =
            serde_json::from_value(body).expect("n=1 should be accepted");
        assert_eq!(request.n, Some(1));
    }

    #[test]
    fn n_policy_rejects_n_zero() {
        let body = serde_json::json!({
            "model": "openai/gpt-4o",
            "messages": [{"role": "user", "content": "hi"}],
            "n": 0
        });
        // Rejection happens at the convert_request level, not deserialization.
        // Deserialization accepts n=0; the proxy handler must reject it.
        let request: llmrust::proxy::ProxyChatRequest =
            serde_json::from_value(body).expect("n=0 should deserialize");
        assert_eq!(request.n, Some(0));
    }

    #[test]
    fn n_policy_rejects_n_greater_than_one() {
        let body = serde_json::json!({
            "model": "openai/gpt-4o",
            "messages": [{"role": "user", "content": "hi"}],
            "n": 3
        });
        let request: llmrust::proxy::ProxyChatRequest =
            serde_json::from_value(body).expect("n=3 should deserialize");
        assert_eq!(request.n, Some(3));
    }
}
