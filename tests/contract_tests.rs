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
    use futures::stream::{self, BoxStream};
    use llmrust::{ChatRequest, ChatResponse, LlmError, LmrsClient, Provider, StreamChunk};
    use std::sync::{Arc, Mutex};

    // ── Capturing provider: records the req.model value ───────────

    struct CapturingProvider {
        seen_model: Arc<Mutex<Option<String>>>,
    }

    #[async_trait::async_trait]
    impl Provider for CapturingProvider {
        async fn chat(&self, req: &ChatRequest) -> llmrust::Result<ChatResponse> {
            *self.seen_model.lock().unwrap() = Some(req.model.clone());
            Ok(ChatResponse::default())
        }

        async fn stream(
            &self,
            req: &ChatRequest,
        ) -> llmrust::Result<BoxStream<'static, llmrust::Result<StreamChunk>>> {
            *self.seen_model.lock().unwrap() = Some(req.model.clone());
            Ok(Box::pin(stream::empty()))
        }
    }

    /// Per CONTRACTS.md: "model is the model name *after* the `/`.
    /// The client sets req.model before calling the provider."
    #[tokio::test]
    async fn forwards_model_name_without_provider_prefix() {
        let llm = LmrsClient::new();
        let seen_model = Arc::new(Mutex::new(None));
        let provider = CapturingProvider {
            seen_model: Arc::clone(&seen_model),
        };
        llm.set_custom("fake", Arc::new(provider)).await;

        llm.chat_with("fake/my-model", ChatRequest::new("ignored", "hi"))
            .await
            .expect("fake provider should succeed");

        assert_eq!(seen_model.lock().unwrap().as_deref(), Some("my-model"));
    }

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
    async fn rejects_empty_model() {
        let llm = LmrsClient::new();
        match llm.chat("openai/", "test").await {
            Err(LlmError::Parse(msg)) => {
                assert!(msg.contains("non-empty provider and model"), "got: {msg}");
            }
            other => panic!("expected LlmError::Parse, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_empty_provider() {
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
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use futures::stream::{self, BoxStream};
    use llmrust::{ChatRequest, ChatResponse, LmrsClient, Provider, StreamChunk};
    use std::sync::Arc;
    use tower::ServiceExt;

    /// A trivial provider that always returns "ok".
    struct OkProvider;

    #[async_trait::async_trait]
    impl Provider for OkProvider {
        async fn chat(&self, req: &ChatRequest) -> llmrust::Result<ChatResponse> {
            Ok(ChatResponse {
                content: "ok".to_string(),
                model: req.model.clone(),
                ..Default::default()
            })
        }

        async fn stream(
            &self,
            _req: &ChatRequest,
        ) -> llmrust::Result<BoxStream<'static, llmrust::Result<StreamChunk>>> {
            Ok(Box::pin(stream::empty()))
        }
    }

    fn build_request(n: Option<u32>) -> Request<Body> {
        let mut body = serde_json::json!({
            "model": "mock/test-model",
            "messages": [{"role": "user", "content": "hi"}]
        });
        if let Some(n) = n {
            body["n"] = serde_json::json!(n);
        }

        Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("failed to build test request")
    }

    async fn post_with_n(n: Option<u32>) -> (StatusCode, serde_json::Value) {
        let llm = Arc::new(LmrsClient::new());
        llm.set_custom("mock", Arc::new(OkProvider)).await;
        let app = llmrust::proxy::router(llm);

        let response = app
            .oneshot(build_request(n))
            .await
            .expect("proxy request failed");

        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body read failed");
        let json = serde_json::from_slice(&bytes).expect("response body is JSON");
        (status, json)
    }

    fn assert_n_policy_error(json: &serde_json::Value) {
        assert_eq!(json["error"]["type"], "invalid_request_error");
        assert!(json["error"]["param"].is_null());
        assert!(json["error"]["code"].is_null());

        let message = json["error"]["message"]
            .as_str()
            .expect("error.message is a string");
        assert!(
            message.contains("n must be 1"),
            "expected n policy error message, got: {message}"
        );
    }

    /// Per CONTRACTS.md: "Accept missing n or n = 1. Reject n = 0 or n > 1."

    #[tokio::test]
    async fn n_policy_accepts_missing_n() {
        let (status, json) = post_with_n(None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["choices"][0]["message"]["content"], "ok");
    }

    #[tokio::test]
    async fn n_policy_accepts_n_one() {
        let (status, json) = post_with_n(Some(1)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["choices"][0]["message"]["content"], "ok");
    }

    #[tokio::test]
    async fn n_policy_rejects_n_zero() {
        let (status, json) = post_with_n(Some(0)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_n_policy_error(&json);
    }

    #[tokio::test]
    async fn n_policy_rejects_n_greater_than_one() {
        let (status, json) = post_with_n(Some(3)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_n_policy_error(&json);
    }
}
