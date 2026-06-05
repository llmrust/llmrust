//! llmrust HTTP Proxy Server
//!
//! Run with:
//! ```bash
//! export LLMRUST_OPENAI_KEY="sk-..."
//! export LLMRUST_ANTHROPIC_KEY="sk-ant-..."
//! export LLMRUST_DEEPSEEK_KEY="sk-..."
//! export LLMRUST_GOOGLE_KEY="AIza..."
//! export LLMRUST_MOONSHOT_KEY="sk-..."
//! export LLMRUST_OPENROUTER_KEY="sk-or-..."
//! cargo run --example proxy_server
//! ```
//!
//! Then call:
//! ```bash
//! curl http://localhost:3000/v1/chat/completions \
//!   -H "Content-Type: application/json" \
//!   -d '{"model": "openai/gpt-4o-mini", "messages": [{"role": "user", "content": "Hello!"}]}'
//! ```
//!
//! Press Ctrl+C to stop gracefully.

use std::sync::Arc;

use llmrust::proxy;
use llmrust::LmrsClient;

#[tokio::main]
async fn main() {
    let llm = Arc::new(LmrsClient::new());

    // Register providers from environment variables
    if let Ok(key) = std::env::var("LLMRUST_OPENAI_KEY") {
        if !key.is_empty() {
            llm.set_openai(&key).await;
            println!("  ✓ OpenAI registered");
        }
    }
    if let Ok(key) = std::env::var("LLMRUST_ANTHROPIC_KEY") {
        if !key.is_empty() {
            llm.set_anthropic(&key).await;
            println!("  ✓ Anthropic registered");
        }
    }
    if let Ok(key) = std::env::var("LLMRUST_DEEPSEEK_KEY") {
        if !key.is_empty() {
            llm.set_deepseek(&key).await;
            println!("  ✓ DeepSeek registered");
        }
    }
    if let Ok(key) = std::env::var("LLMRUST_GOOGLE_KEY") {
        if !key.is_empty() {
            llm.set_google(&key).await;
            println!("  ✓ Google Gemini registered");
        }
    }
    if let Ok(key) = std::env::var("LLMRUST_MOONSHOT_KEY") {
        if !key.is_empty() {
            llm.set_moonshot(&key).await;
            println!("  ✓ Moonshot/Kimi registered");
        }
    }
    if let Ok(key) = std::env::var("LLMRUST_OPENROUTER_KEY") {
        if !key.is_empty() {
            llm.set_openrouter(&key).await;
            println!("  ✓ OpenRouter registered");
        }
    }

    // Always register Ollama (local, no key needed)
    llm.set_ollama(None).await;
    println!("  ✓ Ollama registered (http://localhost:11434)");

    let providers = llm.providers().await;
    if providers.is_empty() {
        eprintln!(
            "⚠  No providers configured. Set at least one LLMRUST_*_KEY environment variable."
        );
        eprintln!("   Example: $env:LLMRUST_OPENAI_KEY='sk-...'");
        std::process::exit(1);
    }

    println!("\n🚀 llmrust proxy server starting on http://0.0.0.0:3000");
    println!("   Registered providers: {}", providers.join(", "));
    println!("   Try: curl http://localhost:3000/v1/chat/completions ...");
    println!("   Health: curl http://localhost:3000/health");
    println!("   Press Ctrl+C to stop.\n");

    // proxy::serve binds, serves, and handles graceful shutdown on Ctrl+C/SIGTERM
    if let Err(e) = proxy::serve(llm, "0.0.0.0:3000").await {
        eprintln!("Server error: {e}");
        std::process::exit(1);
    }
}
