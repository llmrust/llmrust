//! Unit tests for llmrust core functionality.

#[cfg(test)]
mod tests {
    use futures::stream::{self, BoxStream};
    use llmrust::{ChatRequest, ChatResponse, LmrsClient, Message, Provider, Role, StreamChunk};
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct RecordedRequest {
        model: Arc<Mutex<Option<String>>>,
        stream: Arc<Mutex<Option<bool>>>,
    }

    struct RecordingProvider {
        recorded: RecordedRequest,
    }

    #[async_trait::async_trait]
    impl Provider for RecordingProvider {
        async fn chat(&self, req: &ChatRequest) -> llmrust::Result<ChatResponse> {
            *self.recorded.model.lock().expect("lock not poisoned") = Some(req.model.clone());
            Ok(ChatResponse {
                content: "ok".to_string(),
                model: req.model.clone(),
                ..Default::default()
            })
        }

        async fn stream(
            &self,
            req: &ChatRequest,
        ) -> llmrust::Result<BoxStream<'static, llmrust::Result<StreamChunk>>> {
            *self.recorded.model.lock().expect("lock not poisoned") = Some(req.model.clone());
            *self.recorded.stream.lock().expect("lock not poisoned") = Some(req.stream);
            Ok(Box::pin(stream::once(async {
                Ok(StreamChunk {
                    done: true,
                    ..Default::default()
                })
            })))
        }
    }

    #[test]
    fn test_message_constructors() {
        let sys = Message::system("You are helpful.");
        let user = Message::user("Hello");
        let assistant = Message::assistant("Hi there!");

        assert_eq!(sys.role, Role::System);
        assert_eq!(sys.content.as_text(), "You are helpful.");
        assert_eq!(user.role, Role::User);
        assert_eq!(user.content.as_text(), "Hello");
        assert_eq!(assistant.role, Role::Assistant);
        assert_eq!(assistant.content.as_text(), "Hi there!");
    }

    #[test]
    fn test_chat_request_builder() {
        let req = ChatRequest::new("gpt-4o", "What is Rust?")
            .with_system("You are a Rust expert.")
            .with_temperature(0.7)
            .with_max_tokens(500)
            .with_stream();

        assert_eq!(req.model, "gpt-4o");
        assert_eq!(req.messages.len(), 2);
        assert_eq!(req.messages[0].role, Role::System);
        assert_eq!(req.messages[1].role, Role::User);
        assert_eq!(req.temperature, Some(0.7));
        assert_eq!(req.max_tokens, Some(500));
        assert!(req.stream);
    }

    #[test]
    fn test_chat_request_default_no_stream() {
        let req = ChatRequest::new("gpt-4o", "Hello");
        assert!(!req.stream);
        assert!(req.temperature.is_none());
        assert!(req.max_tokens.is_none());
    }

    #[tokio::test]
    async fn test_lmrs_client_parse_model_valid() {
        let llm = LmrsClient::new();
        // Test that chat fails with unknown provider (not parse error)
        let result = llm.chat("openai/gpt-4o", "test").await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Unknown provider"));
    }

    #[tokio::test]
    async fn test_lmrs_client_parse_model_invalid() {
        let llm = LmrsClient::new();
        let result = llm.chat("no-slash-here", "test").await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("provider/model"));
    }

    #[tokio::test]
    async fn test_lmrs_client_parse_model_rejects_empty_parts() {
        let llm = LmrsClient::new();
        let result = llm.chat("openai/", "test").await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("non-empty provider and model"));

        let result = llm.chat("/gpt-4o", "test").await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("non-empty provider and model"));
    }

    #[tokio::test]
    async fn test_lmrs_client_providers_empty() {
        let llm = LmrsClient::new();
        let providers = llm.providers().await;
        assert!(providers.is_empty());
    }

    #[tokio::test]
    async fn test_lmrs_client_set_openai() {
        let llm = LmrsClient::new();
        llm.set_openai("sk-test").await;
        let providers = llm.providers().await;
        assert!(providers.contains(&"openai".to_string()));
    }

    #[tokio::test]
    async fn test_lmrs_client_set_multiple_providers() {
        let llm = LmrsClient::new();
        llm.set_openai("sk-test").await;
        llm.set_anthropic("sk-ant-test").await;
        llm.set_deepseek("sk-ds-test").await;

        let mut providers = llm.providers().await;
        providers.sort();
        assert_eq!(providers, vec!["anthropic", "deepseek", "openai"]);
    }

    #[tokio::test]
    async fn test_lmrs_client_set_openai_compatible() {
        let llm = LmrsClient::new();
        llm.set_openai_compatible("sk-test", "http://localhost:8080/v1")
            .await;
        let providers = llm.providers().await;
        assert!(providers.contains(&"openai".to_string()));
    }

    #[tokio::test]
    async fn test_lmrs_client_set_google() {
        let llm = LmrsClient::new();
        llm.set_google("AIza-test").await;
        let providers = llm.providers().await;
        assert!(providers.contains(&"google".to_string()));
    }

    #[tokio::test]
    async fn test_lmrs_client_set_ollama_default() {
        let llm = LmrsClient::new();
        llm.set_ollama(None).await;
        let providers = llm.providers().await;
        assert!(providers.contains(&"ollama".to_string()));
    }

    #[tokio::test]
    async fn test_lmrs_client_set_ollama_custom_url() {
        let llm = LmrsClient::new();
        llm.set_ollama(Some("http://my-ollama.local:11434".to_string()))
            .await;
        let providers = llm.providers().await;
        assert!(providers.contains(&"ollama".to_string()));
    }

    #[tokio::test]
    async fn test_lmrs_client_set_moonshot() {
        let llm = LmrsClient::new();
        llm.set_moonshot("sk-moonshot-test").await;
        let providers = llm.providers().await;
        assert!(providers.contains(&"moonshot".to_string()));
    }

    #[tokio::test]
    async fn test_lmrs_client_set_openrouter() {
        let llm = LmrsClient::new();
        llm.set_openrouter("sk-or-test").await;
        let providers = llm.providers().await;
        assert!(providers.contains(&"openrouter".to_string()));
    }

    #[tokio::test]
    async fn test_convenience_chat_and_stream_set_internal_model() {
        let llm = LmrsClient::new();
        let recorded = RecordedRequest::default();
        llm.set_custom(
            "mock",
            Arc::new(RecordingProvider {
                recorded: recorded.clone(),
            }),
        )
        .await;

        let resp = llm
            .chat("mock/actual-model", "secret prompt")
            .await
            .unwrap();
        assert_eq!(resp.model, "actual-model");
        assert_eq!(
            recorded.model.lock().expect("lock not poisoned").as_deref(),
            Some("actual-model")
        );

        let mut chunks = llm
            .stream("mock/stream-model", "secret prompt")
            .await
            .unwrap();
        let chunk = futures::StreamExt::next(&mut chunks)
            .await
            .unwrap()
            .unwrap();
        assert!(chunk.done);
        assert_eq!(
            recorded.model.lock().expect("lock not poisoned").as_deref(),
            Some("stream-model")
        );
        assert_eq!(
            *recorded.stream.lock().expect("lock not poisoned"),
            Some(true)
        );
    }

    #[tokio::test]
    async fn test_lmrs_client_set_all_seven_providers() {
        let llm = LmrsClient::new();
        llm.set_openai("sk-1").await;
        llm.set_anthropic("sk-2").await;
        llm.set_deepseek("sk-3").await;
        llm.set_google("sk-4").await;
        llm.set_ollama(None).await;
        llm.set_moonshot("sk-5").await;
        llm.set_openrouter("sk-6").await;

        let mut providers = llm.providers().await;
        providers.sort();
        assert_eq!(
            providers,
            vec![
                "anthropic".to_string(),
                "deepseek".to_string(),
                "google".to_string(),
                "moonshot".to_string(),
                "ollama".to_string(),
                "openai".to_string(),
                "openrouter".to_string(),
            ]
        );
    }

    #[test]
    fn test_message_serialization() {
        let msg = Message::user("Hello, world!");
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"role\":\"user\""));
        assert!(json.contains("\"content\":\"Hello, world!\""));

        let deserialized: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.role, Role::User);
        assert_eq!(deserialized.content.as_text(), "Hello, world!");
    }

    #[test]
    fn test_role_serialization() {
        assert_eq!(serde_json::to_string(&Role::System).unwrap(), "\"system\"");
        assert_eq!(serde_json::to_string(&Role::User).unwrap(), "\"user\"");
        assert_eq!(
            serde_json::to_string(&Role::Assistant).unwrap(),
            "\"assistant\""
        );

        let system: Role = serde_json::from_str("\"system\"").unwrap();
        assert_eq!(system, Role::System);
    }

    #[tokio::test]
    async fn test_from_env_registers_ollama_always() {
        // Ollama should always be registered by from_env().
        // We don't set any API keys here, so only ollama should appear.
        let llm = LmrsClient::from_env().await;
        let providers = llm.providers().await;
        assert!(
            providers.contains(&"ollama".to_string()),
            "from_env() should always register ollama, got: {:?}",
            providers
        );
    }

    #[test]
    fn test_prelude_reexports_compile() {
        // Just verify the prelude compiles and re-exports key types.
        use llmrust::prelude::*;
        let _req = ChatRequest::new("model", "hi");
        let _msg = Message::user("hello");
        let _client = LmrsClient::new();
    }
}
