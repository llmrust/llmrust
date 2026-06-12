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
