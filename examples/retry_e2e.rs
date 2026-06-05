//! End-to-end test: `LmrsClient::with_retry()` wraps the real OpenAI-compatible
//! provider and makes a real upstream call. If retry wiring were broken
//! (e.g., `Arc<dyn Provider>` ownership issues) this would not even compile
//! or would panic at runtime.

use llmrust::{LmrsClient, RetryProvider};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| "OPENAI_API_KEY required")?;
    let base_url = std::env::var("OPENAI_BASE_URL")
        .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
    let model_name = std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());

    println!("=== Test: LmrsClient::with_retry() wraps real provider ===");
    let llm = LmrsClient::new();
    llm.set_openai_compatible(&api_key, &base_url).await;
    llm.with_retry(2).await;
    println!("[ok] registered provider and wrapped with retry(2)");

    // Verify the provider is still usable through the wrapped Arc<dyn Provider>
    let providers = llm.providers().await;
    println!("[ok] providers registered: {providers:?}");

    let model = format!("openai/{model_name}");
    let resp = llm.chat(&model, "用一句话回答：2+2 等于几？").await?;
    println!("[ok] chat succeeded: {}", resp.content);
    println!("[ok] tokens: {:?}", resp.usage);

    // Also verify RetryProvider works standalone (not through LmrsClient)
    println!();
    println!("=== Test: RetryProvider standalone wrapping ===");
    use llmrust::providers::openai::OpenAIProvider;
    use llmrust::providers::{Provider, ProviderConfig};
    use llmrust::ChatRequest;

    let config = ProviderConfig::new(api_key).with_base_url(base_url);
    let inner: Arc<dyn Provider> = Arc::new(OpenAIProvider::new(config));
    let retrying = RetryProvider::new(inner, 2);
    let req = ChatRequest::new(&model_name, "用一句话回答：3+5 等于几？");
    let resp = retrying.chat(&req).await?;
    println!("[ok] RetryProvider.chat succeeded: {}", resp.content);

    println!();
    println!("=== ALL E2E RETRY TESTS PASSED ===");
    Ok(())
}
