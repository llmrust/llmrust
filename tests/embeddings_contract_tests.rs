//! Contract tests for the embeddings API foundation.
//!
//! These tests verify embeddings routing, prefix stripping, error handling,
//! and batch ordering without calling any real provider API.

#[cfg(test)]
mod embeddings_contracts {
    use llmrust::{Embedding, EmbeddingRequest, EmbeddingResponse, LlmError, LmrsClient, Provider};
    use std::sync::{Arc, Mutex};

    // ── Fake embedding providers ───────────────────────────────────

    /// A provider that records the `req.model` it receives.
    struct CapturingEmbedProvider {
        seen_model: Arc<Mutex<Option<String>>>,
    }

    #[async_trait::async_trait]
    impl Provider for CapturingEmbedProvider {
        async fn chat(
            &self,
            _req: &llmrust::ChatRequest,
        ) -> llmrust::Result<llmrust::ChatResponse> {
            unimplemented!()
        }

        async fn stream(
            &self,
            _req: &llmrust::ChatRequest,
        ) -> llmrust::Result<
            futures::stream::BoxStream<'static, llmrust::Result<llmrust::StreamChunk>>,
        > {
            unimplemented!()
        }

        async fn embed(&self, req: &EmbeddingRequest) -> llmrust::Result<EmbeddingResponse> {
            *self.seen_model.lock().unwrap() = Some(req.model.clone());
            Ok(EmbeddingResponse {
                model: req.model.clone(),
                data: Vec::new(),
                usage: None,
            })
        }
    }

    /// A provider that mirrors `input` as `data[].index`.
    struct MirrorEmbedProvider;

    #[async_trait::async_trait]
    impl Provider for MirrorEmbedProvider {
        async fn chat(
            &self,
            _req: &llmrust::ChatRequest,
        ) -> llmrust::Result<llmrust::ChatResponse> {
            unimplemented!()
        }

        async fn stream(
            &self,
            _req: &llmrust::ChatRequest,
        ) -> llmrust::Result<
            futures::stream::BoxStream<'static, llmrust::Result<llmrust::StreamChunk>>,
        > {
            unimplemented!()
        }

        async fn embed(&self, req: &EmbeddingRequest) -> llmrust::Result<EmbeddingResponse> {
            Ok(EmbeddingResponse {
                model: req.model.clone(),
                data: req
                    .input
                    .iter()
                    .enumerate()
                    .map(|(i, _)| Embedding {
                        index: i,
                        embedding: vec![0.0],
                    })
                    .collect(),
                usage: None,
            })
        }
    }

    /// A provider that only implements chat/stream, NOT embed.
    /// Uses the default unsupported error.
    struct ChatOnlyProvider;

    #[async_trait::async_trait]
    impl Provider for ChatOnlyProvider {
        async fn chat(
            &self,
            _req: &llmrust::ChatRequest,
        ) -> llmrust::Result<llmrust::ChatResponse> {
            Ok(llmrust::ChatResponse::default())
        }

        async fn stream(
            &self,
            _req: &llmrust::ChatRequest,
        ) -> llmrust::Result<
            futures::stream::BoxStream<'static, llmrust::Result<llmrust::StreamChunk>>,
        > {
            Ok(Box::pin(futures::stream::empty()))
        }
    }

    // ── Routing tests ─────────────────────────────────────────────

    #[tokio::test]
    async fn embed_rejects_model_without_provider_prefix() {
        let llm = LmrsClient::new();
        let err = llm
            .embed("text-embedding-3-small", "hello")
            .await
            .unwrap_err();
        assert!(
            matches!(err, LlmError::Parse(_)),
            "expected Parse, got: {err:?}"
        );
        assert!(
            err.to_string().contains("provider/model"),
            "expected provider/model format error"
        );
    }

    #[tokio::test]
    async fn embed_rejects_empty_provider() {
        let llm = LmrsClient::new();
        let err = llm.embed("/model", "hello").await.unwrap_err();
        assert!(matches!(err, LlmError::Parse(_)));
        assert!(err.to_string().contains("non-empty provider and model"));
    }

    #[tokio::test]
    async fn embed_rejects_empty_model() {
        let llm = LmrsClient::new();
        let err = llm.embed("openai/", "hello").await.unwrap_err();
        assert!(matches!(err, LlmError::Parse(_)));
        assert!(err.to_string().contains("non-empty provider and model"));
    }

    #[tokio::test]
    async fn embed_unknown_provider_returns_unknown_provider() {
        let llm = LmrsClient::new();
        let err = llm.embed("missing/model", "hello").await.unwrap_err();
        match err {
            LlmError::UnknownProvider(name) => assert_eq!(name, "missing"),
            other => panic!("expected UnknownProvider, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn embed_with_strips_provider_prefix() {
        let llm = LmrsClient::new();
        let seen_model = Arc::new(Mutex::new(None));
        llm.set_custom(
            "fake",
            Arc::new(CapturingEmbedProvider {
                seen_model: Arc::clone(&seen_model),
            }),
        )
        .await;

        llm.embed_with(
            "fake/my-embed-model",
            EmbeddingRequest::new("ignored", "hello"),
        )
        .await
        .expect("embed_with should succeed");

        assert_eq!(
            seen_model.lock().unwrap().as_deref(),
            Some("my-embed-model"),
            "provider should see model name without prefix"
        );
    }

    #[tokio::test]
    async fn embed_batch_preserves_input_order() {
        let llm = LmrsClient::new();
        llm.set_custom("mirror", Arc::new(MirrorEmbedProvider))
            .await;

        let resp = llm
            .embed_batch("mirror/model", ["a", "b", "c"])
            .await
            .expect("embed_batch should succeed");

        assert_eq!(resp.data.len(), 3);
        for (i, emb) in resp.data.iter().enumerate() {
            assert_eq!(
                emb.index, i,
                "embedding index {i} must match input position"
            );
        }
    }

    #[tokio::test]
    async fn unsupported_provider_returns_unsupported() {
        let llm = LmrsClient::new();
        llm.set_custom("chat-only", Arc::new(ChatOnlyProvider))
            .await;

        let err = llm.embed("chat-only/model", "hello").await.unwrap_err();
        match err {
            LlmError::Unsupported { feature, .. } => {
                assert_eq!(feature, "embeddings");
            }
            other => panic!("expected LlmError::Unsupported, got: {other:?}"),
        }
    }

    // ── Builder tests ─────────────────────────────────────────────

    #[test]
    fn embedding_request_new_sets_single_input() {
        let req = EmbeddingRequest::new("model", "hello");
        assert_eq!(req.model, "model");
        assert_eq!(req.input, vec!["hello"]);
        assert!(req.dimensions.is_none());
    }

    #[test]
    fn embedding_request_batch_sets_multiple_inputs() {
        let req = EmbeddingRequest::batch("model", vec!["a", "b", "c"]);
        assert_eq!(req.input.len(), 3);
        assert_eq!(req.input[1], "b");
    }

    #[test]
    fn embedding_request_builders_set_optional_fields() {
        let req = EmbeddingRequest::new("m", "hi")
            .with_dimensions(1536)
            .with_user("user-1")
            .with_extra("task", "search");

        assert_eq!(req.dimensions, Some(1536));
        assert_eq!(req.user.as_deref(), Some("user-1"));
        assert_eq!(
            req.extra.get("task").and_then(|v| v.as_str()),
            Some("search")
        );
    }
}

mod provider_embedding_integrity {
    use llmrust::providers::compat::OpenAiCompatibleProvider;
    use llmrust::providers::ollama::OllamaProvider;
    use llmrust::{EmbeddingRequest, LlmError, Provider, ProviderConfig};
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn serve_responses(bodies: Vec<String>) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for body in bodies {
                let (mut stream, _) = listener.accept().unwrap();
                let mut bytes = Vec::new();
                let header_end = loop {
                    if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                        break index + 4;
                    }
                    let mut buffer = [0_u8; 4096];
                    let count = stream.read(&mut buffer).unwrap();
                    if count == 0 {
                        panic!("request headers ended early");
                    }
                    bytes.extend_from_slice(&buffer[..count]);
                };
                let headers = String::from_utf8_lossy(&bytes[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                while bytes.len() < header_end + content_length {
                    let mut buffer = [0_u8; 4096];
                    let count = stream.read(&mut buffer).unwrap();
                    if count == 0 {
                        panic!("request body ended early");
                    }
                    bytes.extend_from_slice(&buffer[..count]);
                }
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(response.as_bytes()).unwrap();
                stream.flush().unwrap();
            }
        });
        (format!("http://{address}"), server)
    }

    #[tokio::test]
    async fn compat_embedding_requires_complete_unique_index_set() {
        let (url, server) = serve_responses(vec![
            r#"{"model":"m","data":[{"index":0,"embedding":[0.1]}]}"#.into(),
            r#"{"model":"m","data":[{"index":1,"embedding":[0.2]},{"index":0,"embedding":[0.1]}]}"#
                .into(),
        ]);
        let provider = OpenAiCompatibleProvider::new(
            ProviderConfig::new("test").with_base_url(format!("{url}/v1")),
            [],
        );
        let request = EmbeddingRequest::batch("m", ["a", "b"]);
        assert!(matches!(
            provider.embed(&request).await,
            Err(LlmError::Parse(_))
        ));
        let response = provider.embed(&request).await.unwrap();
        assert_eq!(response.data.len(), 2);
        assert_eq!(response.data[0].index, 1);
        assert_eq!(response.data[1].index, 0);
        server.join().unwrap();
    }

    #[tokio::test]
    async fn ollama_embedding_requires_one_vector_per_input() {
        let (url, server) = serve_responses(vec![
            r#"{"model":"m","embeddings":[[0.1]]}"#.into(),
            r#"{"model":"m","embeddings":[[0.1],[0.2]]}"#.into(),
        ]);
        let provider = OllamaProvider::new(ProviderConfig::new("").with_base_url(url));
        let request = EmbeddingRequest::batch("m", ["a", "b"]);
        assert!(matches!(
            provider.embed(&request).await,
            Err(LlmError::Parse(_))
        ));
        let response = provider.embed(&request).await.unwrap();
        assert_eq!(response.data.len(), 2);
        assert_eq!(response.data[0].index, 0);
        assert_eq!(response.data[1].index, 1);
        server.join().unwrap();
    }
}
