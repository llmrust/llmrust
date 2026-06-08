//! # llmrust
//!
//! Call multiple LLM APIs with one unified Rust interface.
//!
//! ## Installation
//!
//! ```toml
//! # Cargo.toml
//!
//! # LLM client only (recommended for most users)
//! [dependencies]
//! llmrust = "0.1"
//!
//! # With the built-in HTTP proxy server
//! [dependencies]
//! llmrust = { version = "0.1", features = ["proxy"] }
//! ```
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use llmrust::{LmrsClient, Message};
//!
//! #[tokio::main]
//! async fn main() {
//!     let llm = LmrsClient::new();
//!
//!     // OpenAI
//!     llm.set_openai("sk-...").await;
//!     let resp = llm.chat("openai/gpt-4o", "Hello!").await.unwrap();
//!     println!("{}", resp.content);
//!
//!     // Anthropic
//!     llm.set_anthropic("sk-ant-...").await;
//!     let resp = llm.chat("anthropic/claude-sonnet-4-20250514", "Hello!").await.unwrap();
//!     println!("{}", resp.content);
//!
//!     // DeepSeek
//!     llm.set_deepseek("sk-...").await;
//!     let resp = llm.chat("deepseek/deepseek-chat", "Hello!").await.unwrap();
//!     println!("{}", resp.content);
//! }
//! ```

pub mod providers;
#[cfg(feature = "proxy")]
pub mod proxy;
pub mod router;
pub mod types;

use std::collections::HashMap;
use std::sync::Arc;

use futures::StreamExt;
use tokio::sync::RwLock;

pub use providers::retry::RetryProvider;
pub use providers::{LlmError, Provider, ProviderConfig, Result};
pub use router::{Router, RoutingStrategy};
pub use types::{
    ChatRequest, ChatResponse, Content, ContentPart, FunctionCall, FunctionDef, ImageUrl, LogProbs,
    Message, ResponseFormat, Role, StreamChunk, TokenLogProb, Tool, ToolCall, ToolChoice,
    TopLogProb, Usage,
};

pub(crate) use futures::stream::BoxStream;

use providers::anthropic::AnthropicProvider;
use providers::deepseek::DeepSeekProvider;
use providers::google::GoogleProvider;
use providers::moonshot::MoonshotProvider;
use providers::ollama::OllamaProvider;
use providers::openai::OpenAIProvider;
use providers::openrouter::OpenRouterProvider;

/// The unified LLM client. Routes `provider/model` strings to the right backend.
pub struct LmrsClient {
    providers: Arc<RwLock<HashMap<String, Arc<dyn Provider>>>>,
}

impl LmrsClient {
    /// Create a new empty client. Register providers with `set_*` methods.
    pub fn new() -> Self {
        Self {
            providers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register the OpenAI provider.
    pub async fn set_openai(&self, api_key: impl Into<String>) {
        let config = ProviderConfig::new(api_key);
        let provider: Arc<dyn Provider> = Arc::new(OpenAIProvider::new(config));
        self.providers
            .write()
            .await
            .insert("openai".to_string(), provider);
    }

    /// Register the OpenAI provider with a custom base URL (for compatible APIs).
    pub async fn set_openai_compatible(
        &self,
        api_key: impl Into<String>,
        base_url: impl Into<String>,
    ) {
        let config = ProviderConfig::new(api_key).with_base_url(base_url);
        let provider: Arc<dyn Provider> = Arc::new(OpenAIProvider::new(config));
        self.providers
            .write()
            .await
            .insert("openai".to_string(), provider);
    }

    /// Register the Anthropic provider.
    pub async fn set_anthropic(&self, api_key: impl Into<String>) {
        let config = ProviderConfig::new(api_key);
        let provider: Arc<dyn Provider> = Arc::new(AnthropicProvider::new(config));
        self.providers
            .write()
            .await
            .insert("anthropic".to_string(), provider);
    }

    /// Register the DeepSeek provider.
    pub async fn set_deepseek(&self, api_key: impl Into<String>) {
        let config = ProviderConfig::new(api_key);
        let provider: Arc<dyn Provider> = Arc::new(DeepSeekProvider::new(config));
        self.providers
            .write()
            .await
            .insert("deepseek".to_string(), provider);
    }

    /// Register the Google Gemini provider.
    pub async fn set_google(&self, api_key: impl Into<String>) {
        let config = ProviderConfig::new(api_key);
        let provider: Arc<dyn Provider> = Arc::new(GoogleProvider::new(config));
        self.providers
            .write()
            .await
            .insert("google".to_string(), provider);
    }

    /// Register the Ollama provider.
    /// Pass `None` to use the default `http://localhost:11434`.
    pub async fn set_ollama(&self, base_url: Option<String>) {
        let config = ProviderConfig::new("");
        let config = match base_url {
            Some(url) => config.with_base_url(url),
            None => config,
        };
        let provider: Arc<dyn Provider> = Arc::new(OllamaProvider::new(config));
        self.providers
            .write()
            .await
            .insert("ollama".to_string(), provider);
    }

    /// Register the Moonshot/Kimi provider.
    pub async fn set_moonshot(&self, api_key: impl Into<String>) {
        let config = ProviderConfig::new(api_key);
        let provider: Arc<dyn Provider> = Arc::new(MoonshotProvider::new(config));
        self.providers
            .write()
            .await
            .insert("moonshot".to_string(), provider);
    }

    /// Register the OpenRouter provider.
    pub async fn set_openrouter(&self, api_key: impl Into<String>) {
        let config = ProviderConfig::new(api_key);
        let provider: Arc<dyn Provider> = Arc::new(OpenRouterProvider::new(config));
        self.providers
            .write()
            .await
            .insert("openrouter".to_string(), provider);
    }

    /// Register a custom provider under a name.
    pub async fn set_custom(&self, name: impl Into<String>, provider: Arc<dyn Provider>) {
        self.providers.write().await.insert(name.into(), provider);
    }

    /// Wrap every registered provider with [`RetryProvider`].
    ///
    /// All future `chat` / `stream` calls through this `LmrsClient` instance
    /// will automatically retry transient failures (HTTP 5xx, network errors)
    /// up to `max_retries` times with exponential back-off.
    ///
    /// Call this **after** all `set_*` calls.
    pub async fn with_retry(&self, max_retries: u32) {
        let mut map = self.providers.write().await;
        let keys: Vec<String> = map.keys().cloned().collect();
        for key in keys {
            if let Some(provider) = map.remove(&key) {
                let wrapped =
                    Arc::new(RetryProvider::new(provider, max_retries)) as Arc<dyn Provider>;
                map.insert(key, wrapped);
            }
        }
    }

    /// Parse a "provider/model" string into (provider_name, model_name).
    fn parse_model(model: &str) -> Result<(&str, &str)> {
        model.split_once('/').ok_or_else(|| {
            LlmError::Parse(format!(
                "Model must be in 'provider/model' format, got: {}",
                model
            ))
        })
    }

    /// Get the provider for a given model string.
    pub async fn get_provider(&self, provider_name: &str) -> Result<Arc<dyn Provider>> {
        self.providers
            .read()
            .await
            .get(provider_name)
            .cloned()
            .ok_or_else(|| LlmError::UnknownProvider(provider_name.to_string()))
    }

    /// Send a simple chat request with a single user message.
    pub async fn chat(&self, model: &str, prompt: &str) -> Result<ChatResponse> {
        let (provider_name, model_name) = Self::parse_model(model)?;
        let provider = self.get_provider(provider_name).await?;
        let req = ChatRequest::new(model_name, prompt);
        provider.chat(&req).await
    }

    /// Send a chat request with full control over parameters.
    pub async fn chat_with(&self, model: &str, req: ChatRequest) -> Result<ChatResponse> {
        let (provider_name, model_name) = Self::parse_model(model)?;
        let provider = self.get_provider(provider_name).await?;
        let mut req = req;
        req.model = model_name.to_string();
        provider.chat(&req).await
    }

    /// Send a streaming chat request with a single user message.
    pub async fn stream(
        &self,
        model: &str,
        prompt: &str,
    ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        let (provider_name, model_name) = Self::parse_model(model)?;
        let provider = self.get_provider(provider_name).await?;
        let req = ChatRequest::new(model_name, prompt).with_stream();
        provider.stream(&req).await
    }

    /// Send a streaming chat request with full control over parameters
    /// (used internally for multi-turn REPLs).
    pub async fn stream_with(
        &self,
        model: &str,
        mut req: ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        let (provider_name, model_name) = Self::parse_model(model)?;
        let provider = self.get_provider(provider_name).await?;
        req.model = model_name.to_string();
        req.stream = true;
        provider.stream(&req).await
    }

    /// Send a streaming request and collect the full text.
    pub async fn stream_collect(&self, model: &str, prompt: &str) -> Result<String> {
        let mut stream = self.stream(model, prompt).await?;
        let mut text = String::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            text.push_str(&chunk.delta);
        }
        Ok(text)
    }

    /// List registered provider names.
    pub async fn providers(&self) -> Vec<String> {
        self.providers.read().await.keys().cloned().collect()
    }
}

impl Default for LmrsClient {
    fn default() -> Self {
        Self::new()
    }
}
