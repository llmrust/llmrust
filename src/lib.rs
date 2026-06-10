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
//!
//! ## Logging
//!
//! `llmrust` emits structured [`tracing`](https://docs.rs/tracing) events for
//! provider registration, request lifecycle, proxy requests, retries, router
//! failover, and upstream API errors. It does **not** install a global
//! subscriber; applications remain in control of how logs are collected.
//!
//! The built-in events include operational fields such as `provider`, `model`,
//! HTTP `status`, retry `attempt`, and router `group`. They intentionally avoid
//! logging API keys, request bodies, prompts, message content, and response
//! text.
//!
//! ```rust,ignore
//! // In your application, not inside llmrust:
//! tracing_subscriber::fmt()
//!     .with_env_filter("llmrust=debug")
//!     .init();
//! ```

pub mod providers;
#[cfg(feature = "proxy")]
pub mod proxy;
pub mod router;
pub mod types;

pub mod prelude;

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

    /// Create a client with providers auto-detected from environment variables.
    ///
    /// Checks industry-standard variable names first, then `LLMRUST_*` fallbacks:
    ///
    /// | Provider   | Primary env var       | Fallback                 |
    /// |------------|-----------------------|--------------------------|
    /// | OpenAI     | `OPENAI_API_KEY`      | `LLMRUST_OPENAI_KEY`     |
    /// | Anthropic  | `ANTHROPIC_API_KEY`   | `LLMRUST_ANTHROPIC_KEY`  |
    /// | DeepSeek   | `DEEPSEEK_API_KEY`    | `LLMRUST_DEEPSEEK_KEY`   |
    /// | Google     | `GOOGLE_API_KEY`      | `LLMRUST_GOOGLE_KEY`     |
    /// | Moonshot   | `MOONSHOT_API_KEY`    | `LLMRUST_MOONSHOT_KEY`   |
    /// | OpenRouter | `OPENROUTER_API_KEY`  | `LLMRUST_OPENROUTER_KEY` |
    /// | Ollama     | `OLLAMA_HOST`         | `LLMRUST_OLLAMA_HOST`    |
    ///
    /// Ollama is always registered (no API key required). If neither host
    /// variable is set, it defaults to `http://localhost:11434`.
    ///
    /// ```rust,no_run
    /// use llmrust::LmrsClient;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     // Reads OPENAI_API_KEY, ANTHROPIC_API_KEY, etc. from environment
    ///     let llm = LmrsClient::from_env().await;
    ///     println!("Registered: {:?}", llm.providers().await);
    /// }
    /// ```
    pub async fn from_env() -> Self {
        let client = Self::new();

        // Helper: resolve first non-empty env var from a list.
        fn resolve_env(vars: &[&str]) -> Option<String> {
            vars.iter()
                .find_map(|v| std::env::var(v).ok().filter(|s| !s.is_empty()))
        }

        if let Some(key) = resolve_env(&["OPENAI_API_KEY", "LLMRUST_OPENAI_KEY"]) {
            client.set_openai(key).await;
        }
        if let Some(key) = resolve_env(&["ANTHROPIC_API_KEY", "LLMRUST_ANTHROPIC_KEY"]) {
            client.set_anthropic(key).await;
        }
        if let Some(key) = resolve_env(&["DEEPSEEK_API_KEY", "LLMRUST_DEEPSEEK_KEY"]) {
            client.set_deepseek(key).await;
        }
        if let Some(key) = resolve_env(&["GOOGLE_API_KEY", "LLMRUST_GOOGLE_KEY"]) {
            client.set_google(key).await;
        }
        if let Some(key) = resolve_env(&["MOONSHOT_API_KEY", "LLMRUST_MOONSHOT_KEY"]) {
            client.set_moonshot(key).await;
        }
        if let Some(key) = resolve_env(&["OPENROUTER_API_KEY", "LLMRUST_OPENROUTER_KEY"]) {
            client.set_openrouter(key).await;
        }

        // Ollama: always registered, host is optional.
        let ollama_host = resolve_env(&["OLLAMA_HOST", "LLMRUST_OLLAMA_HOST"]);
        client.set_ollama(ollama_host).await;

        client
    }

    /// Register the OpenAI provider.
    pub async fn set_openai(&self, api_key: impl Into<String>) {
        let config = ProviderConfig::new(api_key);
        let provider: Arc<dyn Provider> = Arc::new(OpenAIProvider::new(config));
        let prev = self
            .providers
            .write()
            .await
            .insert("openai".to_string(), provider);
        if prev.is_some() {
            tracing::warn!("overwriting existing 'openai' provider registration");
        }
        tracing::debug!(provider = "openai", "registered provider");
    }

    /// Register the OpenAI provider with a custom base URL (for compatible APIs).
    ///
    /// **Note:** This registers under the same `"openai"` key as [`set_openai`](Self::set_openai).
    /// Calling both will silently replace the earlier registration.
    pub async fn set_openai_compatible(
        &self,
        api_key: impl Into<String>,
        base_url: impl Into<String>,
    ) {
        let config = ProviderConfig::new(api_key).with_base_url(base_url);
        let provider: Arc<dyn Provider> = Arc::new(OpenAIProvider::new(config));
        let prev = self
            .providers
            .write()
            .await
            .insert("openai".to_string(), provider);
        if prev.is_some() {
            tracing::warn!("overwriting existing 'openai' provider registration");
        }
        tracing::debug!(
            provider = "openai",
            custom_base_url = true,
            "registered provider"
        );
    }

    /// Register the Anthropic provider.
    pub async fn set_anthropic(&self, api_key: impl Into<String>) {
        let config = ProviderConfig::new(api_key);
        let provider: Arc<dyn Provider> = Arc::new(AnthropicProvider::new(config));
        let prev = self
            .providers
            .write()
            .await
            .insert("anthropic".to_string(), provider);
        if prev.is_some() {
            tracing::warn!("overwriting existing 'anthropic' provider registration");
        }
        tracing::debug!(provider = "anthropic", "registered provider");
    }

    /// Register the DeepSeek provider.
    pub async fn set_deepseek(&self, api_key: impl Into<String>) {
        let config = ProviderConfig::new(api_key);
        let provider: Arc<dyn Provider> = Arc::new(DeepSeekProvider::new(config));
        let prev = self
            .providers
            .write()
            .await
            .insert("deepseek".to_string(), provider);
        if prev.is_some() {
            tracing::warn!("overwriting existing 'deepseek' provider registration");
        }
        tracing::debug!(provider = "deepseek", "registered provider");
    }

    /// Register the Google Gemini provider.
    pub async fn set_google(&self, api_key: impl Into<String>) {
        let config = ProviderConfig::new(api_key);
        let provider: Arc<dyn Provider> = Arc::new(GoogleProvider::new(config));
        let prev = self
            .providers
            .write()
            .await
            .insert("google".to_string(), provider);
        if prev.is_some() {
            tracing::warn!("overwriting existing 'google' provider registration");
        }
        tracing::debug!(provider = "google", "registered provider");
    }

    /// Register the Ollama provider.
    /// Pass `None` to use the default `http://localhost:11434`.
    pub async fn set_ollama(&self, base_url: Option<String>) {
        let config = ProviderConfig::new("");
        let custom_base_url = base_url.is_some();
        let config = match base_url {
            Some(url) => config.with_base_url(url),
            None => config,
        };
        let provider: Arc<dyn Provider> = Arc::new(OllamaProvider::new(config));
        let prev = self
            .providers
            .write()
            .await
            .insert("ollama".to_string(), provider);
        if prev.is_some() {
            tracing::warn!("overwriting existing 'ollama' provider registration");
        }
        tracing::debug!(provider = "ollama", custom_base_url, "registered provider");
    }

    /// Register the Moonshot/Kimi provider.
    pub async fn set_moonshot(&self, api_key: impl Into<String>) {
        let config = ProviderConfig::new(api_key);
        let provider: Arc<dyn Provider> = Arc::new(MoonshotProvider::new(config));
        let prev = self
            .providers
            .write()
            .await
            .insert("moonshot".to_string(), provider);
        if prev.is_some() {
            tracing::warn!("overwriting existing 'moonshot' provider registration");
        }
        tracing::debug!(provider = "moonshot", "registered provider");
    }

    /// Register the OpenRouter provider.
    pub async fn set_openrouter(&self, api_key: impl Into<String>) {
        let config = ProviderConfig::new(api_key);
        let provider: Arc<dyn Provider> = Arc::new(OpenRouterProvider::new(config));
        let prev = self
            .providers
            .write()
            .await
            .insert("openrouter".to_string(), provider);
        if prev.is_some() {
            tracing::warn!("overwriting existing 'openrouter' provider registration");
        }
        tracing::debug!(provider = "openrouter", "registered provider");
    }

    /// Register a custom provider under a name.
    pub async fn set_custom(&self, name: impl Into<String>, provider: Arc<dyn Provider>) {
        let name = name.into();
        let prev = self.providers.write().await.insert(name.clone(), provider);
        if prev.is_some() {
            tracing::warn!(provider = %name, "overwriting existing provider registration");
        }
        tracing::debug!(provider = %name, "registered custom provider");
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
        let provider_count = map.len();
        let keys: Vec<String> = map.keys().cloned().collect();
        for key in keys {
            if let Some(provider) = map.remove(&key) {
                let wrapped =
                    Arc::new(RetryProvider::new(provider, max_retries)) as Arc<dyn Provider>;
                map.insert(key, wrapped);
            }
        }
        tracing::debug!(
            providers = provider_count,
            max_retries,
            "wrapped providers with retry"
        );
    }

    /// Parse a "provider/model" string into (provider_name, model_name).
    fn parse_model(model: &str) -> Result<(&str, &str)> {
        let (provider, model_name) = model.split_once('/').ok_or_else(|| {
            LlmError::Parse(format!(
                "Model must be in 'provider/model' format, got: {}",
                model
            ))
        })?;
        if provider.is_empty() || model_name.is_empty() {
            return Err(LlmError::Parse(format!(
                "Model must be in 'provider/model' format with non-empty provider and model, got: {}",
                model
            )));
        }
        Ok((provider, model_name))
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
        self.chat_with(model, ChatRequest::new("", prompt)).await
    }

    /// Send a chat request with full control over parameters.
    pub async fn chat_with(&self, model: &str, req: ChatRequest) -> Result<ChatResponse> {
        let (provider_name, model_name) = Self::parse_model(model)?;
        tracing::debug!(
            provider = provider_name,
            model = model_name,
            "sending chat request"
        );
        let provider = self.get_provider(provider_name).await?;
        let mut req = req;
        req.model = model_name.to_string();
        let resp = provider.chat(&req).await?;
        tracing::debug!(
            provider = provider_name,
            model = model_name,
            finish_reason = ?resp.finish_reason,
            "chat response received"
        );
        Ok(resp)
    }

    /// Send a streaming chat request with a single user message.
    pub async fn stream(
        &self,
        model: &str,
        prompt: &str,
    ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        self.stream_with(model, ChatRequest::new("", prompt)).await
    }

    /// Send a streaming chat request with full control over parameters
    /// (used internally for multi-turn REPLs).
    pub async fn stream_with(
        &self,
        model: &str,
        mut req: ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        let (provider_name, model_name) = Self::parse_model(model)?;
        tracing::debug!(
            provider = provider_name,
            model = model_name,
            "opening stream"
        );
        let provider = self.get_provider(provider_name).await?;
        req.model = model_name.to_string();
        req.stream = true;
        let stream = provider.stream(&req).await?;
        tracing::debug!(
            provider = provider_name,
            model = model_name,
            "stream opened"
        );
        Ok(stream)
    }

    /// Send a streaming request and collect the full text.
    ///
    /// Returns only the concatenated text. Use [`LmrsClient::stream_collect_full`]
    /// if you also need `usage`, `tool_calls`, or `finish_reason`.
    pub async fn stream_collect(&self, model: &str, prompt: &str) -> Result<String> {
        let mut stream = self.stream(model, prompt).await?;
        let mut text = String::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            text.push_str(&chunk.delta);
        }
        Ok(text)
    }

    /// Send a streaming request and collect the full response.
    ///
    /// Returns a [`ChatResponse`] with concatenated text, the last reported
    /// `usage`, `tool_calls`, and `finish_reason` from the terminal chunk(s).
    pub async fn stream_collect_full(&self, model: &str, prompt: &str) -> Result<ChatResponse> {
        let mut stream = self.stream(model, prompt).await?;
        let mut text = String::new();
        let mut usage = None;
        let mut tool_calls = None;
        let mut finish_reason = None;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            text.push_str(&chunk.delta);
            if chunk.usage.is_some() {
                usage = chunk.usage;
            }
            if chunk.tool_calls.is_some() {
                tool_calls = chunk.tool_calls;
            }
            if chunk.finish_reason.is_some() {
                finish_reason = chunk.finish_reason;
            }
        }
        Ok(ChatResponse {
            content: text,
            model: model.to_string(),
            usage,
            tool_calls,
            finish_reason,
            ..Default::default()
        })
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
