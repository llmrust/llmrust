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
                        ..Default::default()
                    }),
                    tool_calls: Some(vec![ToolCall {
                        id: "call_1".into(),
                        call_type: "function".into(),
                        function: llmrust::FunctionCall {
                            name: "greet".into(),
                            arguments: r#"{"name":"world"}"#.into(),
                        },
                    }]),
                    ..Default::default()
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

#[cfg(test)]
mod audit_2026_stream {
    //! C-018-STREAM-CORE production-path tests: every assertion is driven by a
    //! real provider parsing real response bytes (via a local loopback server),
    //! or by `LmrsClient::stream_with` exercising the public collapse layer.

    use futures::{Stream, StreamExt};
    use llmrust::providers::anthropic::AnthropicProvider;
    use llmrust::providers::compat::OpenAiCompatibleProvider;
    use llmrust::providers::google::GoogleProvider;
    use llmrust::providers::ollama::OllamaProvider;
    use llmrust::{
        ChatRequest, ChatResponse, LlmError, LmrsClient, Provider, ProviderConfig, StreamChunk,
    };
    use std::io::{Read, Write};
    use std::marker::PhantomPinned;
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::task::{Context, Poll};
    use std::time::Duration;

    /// Consume one HTTP request so the client can finish writing its body
    /// before the fixture responds (avoids a write-side reset).
    fn read_request(stream: &mut TcpStream) {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
        let mut bytes = Vec::new();
        let mut header_end = None;
        while header_end.is_none() {
            let mut buffer = [0_u8; 4096];
            match stream.read(&mut buffer) {
                Ok(0) | Err(_) => return,
                Ok(count) => {
                    bytes.extend_from_slice(&buffer[..count]);
                    header_end = bytes
                        .windows(4)
                        .position(|window| window == b"\r\n\r\n")
                        .map(|index| index + 4);
                }
            }
        }
        let Some(header_end) = header_end else {
            return;
        };
        let length = String::from_utf8_lossy(&bytes[..header_end])
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().unwrap_or(0))
            })
            .unwrap_or(0);
        while bytes.len() < header_end + length {
            let mut buffer = [0_u8; 4096];
            match stream.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(count) => bytes.extend_from_slice(&buffer[..count]),
            }
        }
    }

    /// Serve exactly one `200 OK` response with the given Content-Type and body.
    fn serve_once(content_type: &str, body: Vec<u8>) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let content_type = content_type.to_string();
        let handle = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                read_request(&mut stream);
                let headers = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(headers.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
                let _ = stream.shutdown(Shutdown::Write);
            }
        });
        (format!("http://{address}"), handle)
    }

    fn compat_provider(base_url: &str) -> OpenAiCompatibleProvider {
        OpenAiCompatibleProvider::new(
            ProviderConfig::new("test-key")
                .with_base_url(base_url)
                .with_timeout_secs(5),
            [],
        )
    }

    async fn collect<S>(stream: &mut S) -> Vec<llmrust::Result<StreamChunk>>
    where
        S: Stream<Item = llmrust::Result<StreamChunk>> + Unpin,
    {
        let mut items = Vec::new();
        while let Some(item) = stream.next().await {
            items.push(item);
        }
        items
    }

    #[tokio::test]
    async fn audit_2026_stream_terminal_thinking_survives_public_client() {
        // One terminal frame carrying BOTH the reasoning increment and the
        // finish reason. The public `stream_with` collapse layer must keep the
        // terminal thinking (and mark thinking_done exactly once).
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"terminal-thought\"},",
            "\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n"
        )
        .as_bytes()
        .to_vec();
        let (base, server) = serve_once("text/event-stream", body);
        let llm = LmrsClient::new();
        llm.set_custom("compat", Arc::new(compat_provider(&base)))
            .await;
        let mut stream = llm
            .stream_with("compat/model", ChatRequest::new("model", "hi"))
            .await
            .expect("stream established");
        let items = collect(&mut stream).await;
        server.join().unwrap();

        let terminals: Vec<&StreamChunk> = items
            .iter()
            .filter_map(|item| item.as_ref().ok())
            .filter(|chunk| chunk.done)
            .collect();
        assert_eq!(terminals.len(), 1, "exactly one terminal: {items:?}");
        let terminal = terminals[0];
        assert_eq!(
            terminal.thinking.as_deref(),
            Some("terminal-thought"),
            "terminal thinking must survive the public collapse layer"
        );
        assert_eq!(
            terminal.thinking_done,
            Some(true),
            "thinking_done lands on the single terminal"
        );
        let thinking_chunks: Vec<&StreamChunk> = items
            .iter()
            .filter_map(|item| item.as_ref().ok())
            .filter(|chunk| {
                chunk
                    .thinking
                    .as_deref()
                    .is_some_and(|text| !text.is_empty())
            })
            .collect();
        assert_eq!(thinking_chunks.len(), 1, "reasoning surfaced exactly once");
        assert!(thinking_chunks[0].done, "reasoning is on the terminal");
    }

    #[tokio::test]
    async fn audit_2026_stream_oversized_line_is_bounded() {
        // A single physical line far beyond MAX_LINE_BYTES. The parser must
        // refuse it with an explicit error instead of buffering without bound.
        let mut body = Vec::new();
        body.extend_from_slice(b"data: ");
        body.resize(2 * 1024 * 1024 + 16, b'x');
        body.extend_from_slice(b"\n\n");
        let (base, server) = serve_once("text/event-stream", body);
        let provider = compat_provider(&base);
        let mut stream = provider
            .stream(&ChatRequest::new("model", "hi"))
            .await
            .expect("stream established");
        let items = collect(&mut stream).await;
        server.join().unwrap();
        assert!(
            items.iter().any(|item| matches!(
                item,
                Err(LlmError::Parse(message)) if message.contains("MAX_LINE_BYTES")
            )),
            "oversized line must be a bounded Parse error, got {items:?}"
        );
    }

    #[tokio::test]
    async fn audit_2026_stream_invalid_utf8_is_error() {
        // Valid frame first, then a line containing invalid UTF-8 bytes. The
        // invalid bytes must surface as an error, never as U+FFFD.
        let mut body = Vec::new();
        body.extend_from_slice(
            b"data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":null}]}\n\n",
        );
        body.extend_from_slice(b"data: \xFF\xFE\n\n");
        let (base, server) = serve_once("text/event-stream", body);
        let provider = compat_provider(&base);
        let mut stream = provider
            .stream(&ChatRequest::new("model", "hi"))
            .await
            .expect("stream established");
        let items = collect(&mut stream).await;
        server.join().unwrap();

        assert!(
            items
                .iter()
                .any(|item| matches!(item, Ok(chunk) if chunk.delta == "ok")),
            "the valid frame before the bad one is surfaced: {items:?}"
        );
        assert!(
            items.iter().any(|item| matches!(
                item,
                Err(LlmError::Parse(message)) if message.contains("UTF-8")
            )),
            "invalid UTF-8 must be an explicit Parse error, got {items:?}"
        );
        assert!(
            !items.iter().any(|item| matches!(
                item,
                Ok(chunk) if chunk.delta.contains('\u{FFFD}') || chunk.thinking.as_deref().is_some_and(|text| text.contains('\u{FFFD}'))
            )),
            "no replacement characters may be emitted: {items:?}"
        );
    }

    #[tokio::test]
    async fn audit_2026_stream_standard_sse_variants_parse() {
        // BOM + comment, `data:` without a space, CRLF, bare CR, multi-line
        // data, `[DONE]`, and `event: error` — all through the real provider.
        let mut body = Vec::new();
        body.extend_from_slice(b"\xEF\xBB\xBF: leading comment\r\n");
        body.extend_from_slice(
            b"data:{\"choices\":[{\"delta\":{\"content\":\"A\"},\"finish_reason\":null}]}\r\n\r\n",
        );
        body.extend_from_slice(b": keepalive\r\r");
        body.extend_from_slice(
            b"data: {\"choices\":[{\"delta\":{\"content\":\"B\"},\"finish_reason\":null}]}\r\r",
        );
        body.extend_from_slice(
            b"data: {\"choices\":[{\"delta\":\r\ndata: {\"content\":\"C\"},\"finish_reason\":null}]}\r\n\r\n",
        );
        body.extend_from_slice(b"data: [DONE]\r\n\r\n");
        body.extend_from_slice(
            b"event: error\r\ndata: {\"error\":{\"message\":\"variant-error\"}}\r\n\r\n",
        );
        let (base, server) = serve_once("text/event-stream; charset=utf-8", body);
        let provider = compat_provider(&base);
        let mut stream = provider
            .stream(&ChatRequest::new("model", "hi"))
            .await
            .expect("stream established");
        let items = collect(&mut stream).await;
        server.join().unwrap();

        let text: String = items
            .iter()
            .filter_map(|item| item.as_ref().ok())
            .map(|chunk| chunk.delta.as_str())
            .collect();
        assert_eq!(text, "ABC", "all framing variants parse: {items:?}");
        assert!(
            items
                .iter()
                .any(|item| matches!(item, Ok(chunk) if chunk.done)),
            "[DONE] still terminates: {items:?}"
        );
        assert!(
            items.iter().any(|item| matches!(
                item,
                Err(LlmError::Stream(message)) if message.contains("variant-error")
            )),
            "`event: error` is surfaced: {items:?}"
        );
    }

    #[tokio::test]
    async fn audit_2026_stream_non_sse_200_is_error() {
        // A 200 response whose Content-Type is not the protocol's type is a
        // protocol error — for all four provider implementations.
        let (compat_base, compat_server) =
            serve_once("application/json", br#"{"choices":[]}"#.to_vec());
        let (anthropic_base, anthropic_server) = serve_once("text/plain", b"not a stream".to_vec());
        let (google_base, google_server) = serve_once("text/html", b"<html></html>".to_vec());
        let (ollama_base, ollama_server) =
            serve_once("text/event-stream", b"data: {}\n\n".to_vec());

        let providers: Vec<Arc<dyn Provider>> = vec![
            Arc::new(compat_provider(&compat_base)),
            Arc::new(AnthropicProvider::new(
                ProviderConfig::new("k").with_base_url(format!("{anthropic_base}/v1")),
            )),
            Arc::new(GoogleProvider::new(
                ProviderConfig::new("k").with_base_url(format!("{google_base}/v1beta/")),
            )),
            Arc::new(OllamaProvider::new(
                ProviderConfig::new("").with_base_url(&ollama_base),
            )),
        ];
        for provider in providers {
            let error = match provider.stream(&ChatRequest::new("model", "hi")).await {
                Ok(_) => panic!("a non-SSE 200 must not become an empty success"),
                Err(error) => error,
            };
            assert!(
                matches!(error, LlmError::Parse(_)),
                "expected Parse, got {error:?}"
            );
        }
        for server in [
            compat_server,
            anthropic_server,
            google_server,
            ollama_server,
        ] {
            server.join().unwrap();
        }
    }

    struct ErrorThenPending {
        polls: Arc<AtomicUsize>,
    }

    impl Stream for ErrorThenPending {
        type Item = llmrust::Result<StreamChunk>;

        fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            if self.polls.fetch_add(1, Ordering::SeqCst) == 0 {
                Poll::Ready(Some(Err(LlmError::Stream("boom".into()))))
            } else {
                Poll::Pending
            }
        }
    }

    struct ErrorThenPendingProvider {
        polls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl Provider for ErrorThenPendingProvider {
        async fn chat(&self, _req: &ChatRequest) -> llmrust::Result<ChatResponse> {
            Ok(ChatResponse::default())
        }

        async fn stream(
            &self,
            _req: &ChatRequest,
        ) -> llmrust::Result<futures::stream::BoxStream<'static, llmrust::Result<StreamChunk>>>
        {
            Ok(Box::pin(ErrorThenPending {
                polls: Arc::clone(&self.polls),
            }))
        }
    }

    #[tokio::test]
    async fn audit_2026_stream_error_closes_without_pending_poll() {
        // After the error is forwarded, the collapse layer must close without
        // polling the (Pending-forever) inner stream again.
        let polls = Arc::new(AtomicUsize::new(0));
        let llm = LmrsClient::new();
        llm.set_custom(
            "err",
            Arc::new(ErrorThenPendingProvider {
                polls: Arc::clone(&polls),
            }),
        )
        .await;
        let mut stream = llm
            .stream_with("err/model", ChatRequest::new("model", "hi"))
            .await
            .expect("stream established");
        let first = stream.next().await.expect("one item");
        assert!(matches!(first, Err(LlmError::Stream(_))), "got {first:?}");
        assert_eq!(polls.load(Ordering::SeqCst), 1, "one inner poll so far");
        // Poll the next item exactly once (no wall-clock timeout): the machine
        // must already be closed, so this is `Ready(None)` immediately.
        let next = stream.next();
        let polled = futures::poll!(next);
        assert!(
            matches!(polled, Poll::Ready(None)),
            "after Err the next poll must be Ready(None), got {polled:?}"
        );
        assert_eq!(
            polls.load(Ordering::SeqCst),
            1,
            "a possibly-Pending inner must not be polled after Err"
        );
    }

    #[tokio::test]
    async fn audit_2026_stream_clean_eof_flush_keeps_missing_finish_reason() {
        // Clean EOF with fully accumulated tool calls: flush them onto the
        // terminal, finish_reason stays None. A parse error in the same shape
        // must NOT flush half-accumulated arguments.
        let clean = concat!(
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",",
            "\"function\":{\"name\":\"write\",\"arguments\":\"{\\\"path\\\":\"}}]},",
            "\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,",
            "\"function\":{\"arguments\":\"\\\"a.rs\\\"}\"}}]},\"finish_reason\":null}]}\n\n"
        )
        .as_bytes()
        .to_vec();
        let (clean_base, clean_server) = serve_once("text/event-stream", clean);
        let llm = LmrsClient::new();
        llm.set_custom("compat", Arc::new(compat_provider(&clean_base)))
            .await;
        let mut stream = llm
            .stream_with("compat/model", ChatRequest::new("model", "hi"))
            .await
            .expect("stream established");
        let items = collect(&mut stream).await;
        clean_server.join().unwrap();

        let terminals: Vec<&StreamChunk> = items
            .iter()
            .filter_map(|item| item.as_ref().ok())
            .filter(|chunk| chunk.done)
            .collect();
        assert_eq!(terminals.len(), 1, "exactly one terminal: {items:?}");
        let terminal = terminals[0];
        assert_eq!(
            terminal.finish_reason, None,
            "a missing finish reason is never invented"
        );
        let calls = terminal
            .tool_calls
            .as_ref()
            .expect("complete tool calls flush at clean EOF");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].function.name, "write");
        assert_eq!(calls[0].function.arguments, "{\"path\":\"a.rs\"}");

        let broken = concat!(
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",",
            "\"function\":{\"name\":\"write\",\"arguments\":\"{\\\"path\\\":\"}}]},",
            "\"finish_reason\":null}]}\n\n",
            "data: {not valid json}\n\n"
        )
        .as_bytes()
        .to_vec();
        let (broken_base, broken_server) = serve_once("text/event-stream", broken);
        let broken_llm = LmrsClient::new();
        broken_llm
            .set_custom("compat", Arc::new(compat_provider(&broken_base)))
            .await;
        let mut broken_stream = broken_llm
            .stream_with("compat/model", ChatRequest::new("model", "hi"))
            .await
            .expect("stream established");
        let broken_items = collect(&mut broken_stream).await;
        broken_server.join().unwrap();
        assert!(
            broken_items
                .iter()
                .any(|item| matches!(item, Err(LlmError::Parse(_)))),
            "the malformed frame surfaces: {broken_items:?}"
        );
        assert!(
            !broken_items
                .iter()
                .any(|item| matches!(item, Ok(chunk) if chunk.done)),
            "no success terminal may be fabricated after an error: {broken_items:?}"
        );
    }

    /// Deliberately does not implement `From<LlmError>` and is `!Unpin`. The
    /// public 0.1.6 compatibility signature must accept it unchanged.
    #[derive(Debug)]
    struct LegacyProbeError(PhantomPinned);

    #[allow(deprecated)]
    #[tokio::test]
    async fn audit_2026_stream_legacy_line_stream_signature_compatibility() {
        let chunks = vec![Ok::<&'static [u8], LegacyProbeError>(b"compat\n")];
        let items = llmrust::providers::stream_util::line_stream(futures::stream::iter(chunks))
            .collect::<Vec<_>>()
            .await;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].as_ref().unwrap(), "compat");
    }

    #[allow(deprecated)]
    #[tokio::test]
    async fn audit_2026_stream_legacy_line_stream_behavior_unchanged() {
        // Only LF splits lines; CRLF is stripped, a bare CR is retained, invalid
        // bytes stay lossy, and the newline-less EOF tail is flushed.
        let chunks = vec![
            Ok::<&'static [u8], LegacyProbeError>(b"a\r\n"),
            Ok(b"b\rc\nlast\xFF"),
        ];
        let lines = llmrust::providers::stream_util::line_stream(futures::stream::iter(chunks))
            .map(|item| item.expect("legacy stream item"))
            .collect::<Vec<_>>()
            .await;
        assert_eq!(lines, vec!["a", "b\rc", "last\u{FFFD}"]);
    }

    #[tokio::test]
    async fn audit_2026_stream_single_newline_json_frames_tolerated() {
        // No blank lines: the first complete JSON frame must be dispatched when
        // the next `data:` line arrives. The second frame starts incomplete and
        // continues on the following line; `[DONE]` is flushed at EOF.
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"A\"},\"finish_reason\":null}]}\n",
            "data: {\"choices\":[{\"delta\":\n",
            "data: {\"content\":\"B\"},\"finish_reason\":\"stop\"}]}\n",
            "data: [DONE]"
        )
        .as_bytes()
        .to_vec();
        let (base, server) = serve_once("text/event-stream", body);
        let provider = compat_provider(&base);
        let mut stream = provider
            .stream(&ChatRequest::new("model", "hi"))
            .await
            .expect("stream established");
        let items = collect(&mut stream).await;
        server.join().unwrap();

        assert!(
            items.iter().all(|item| item.is_ok()),
            "single-newline JSON frames must parse: {items:?}"
        );
        let text: String = items
            .iter()
            .filter_map(|item| item.as_ref().ok())
            .map(|chunk| chunk.delta.as_str())
            .collect();
        assert_eq!(text, "AB", "both compatibility frames surface: {items:?}");
        assert!(
            items
                .iter()
                .any(|item| matches!(item, Ok(chunk) if chunk.done)),
            "EOF-flushed [DONE] still terminates: {items:?}"
        );
    }

    #[tokio::test]
    async fn audit_2026_stream_multiline_data_not_split_before_json_complete() {
        // Standard multi-line data: the first physical line is incomplete JSON,
        // so the next `data:` line must be appended rather than dispatched.
        let body = concat!(
            "data: {\"choices\":[{\"delta\":\n",
            "data: {\"content\":\"AB\"},\"finish_reason\":\"stop\"}]}"
        )
        .as_bytes()
        .to_vec();
        let (base, server) = serve_once("text/event-stream", body);
        let provider = compat_provider(&base);
        let mut stream = provider
            .stream(&ChatRequest::new("model", "hi"))
            .await
            .expect("stream established");
        let items = collect(&mut stream).await;
        server.join().unwrap();

        assert!(
            items.iter().all(|item| item.is_ok()),
            "incomplete JSON must remain one multi-line event: {items:?}"
        );
        let text: String = items
            .iter()
            .filter_map(|item| item.as_ref().ok())
            .map(|chunk| chunk.delta.as_str())
            .collect();
        assert_eq!(text, "AB", "joined data is parsed once: {items:?}");
        assert!(
            items
                .iter()
                .any(|item| matches!(item, Ok(chunk) if chunk.done)),
            "the complete EOF event is dispatched: {items:?}"
        );
    }
}
