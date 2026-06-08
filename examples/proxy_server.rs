//! llmrust HTTP Proxy Server
//!
//! Run with:
//! ```bash
//! export OPENAI_API_KEY="sk-..."          # or LLMRUST_OPENAI_KEY
//! export ANTHROPIC_API_KEY="sk-ant-..."   # or LLMRUST_ANTHROPIC_KEY
//! export DEEPSEEK_API_KEY="sk-..."        # or LLMRUST_DEEPSEEK_KEY
//! export GOOGLE_API_KEY="AIza..."         # or LLMRUST_GOOGLE_KEY
//! export MOONSHOT_API_KEY="sk-..."        # or LLMRUST_MOONSHOT_KEY
//! export OPENROUTER_API_KEY="sk-or-..."   # or LLMRUST_OPENROUTER_KEY
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
    // from_env() auto-detects providers from OPENAI_API_KEY, ANTHROPIC_API_KEY,
    // DEEPSEEK_API_KEY, GOOGLE_API_KEY, MOONSHOT_API_KEY, OPENROUTER_API_KEY,
    // OLLAMA_HOST (also supports LLMRUST_* fallbacks).
    let llm = Arc::new(LmrsClient::from_env().await);

    let providers = llm.providers().await;
    if providers.is_empty() {
        eprintln!("No providers configured. Set at least one environment variable.");
        eprintln!("   Supported: OPENAI_API_KEY, ANTHROPIC_API_KEY, DEEPSEEK_API_KEY,");
        eprintln!("   GOOGLE_API_KEY, MOONSHOT_API_KEY, OPENROUTER_API_KEY");
        eprintln!("   Or LLMRUST_*_KEY fallbacks.");
        std::process::exit(1);
    }

    println!("\nllmrust proxy server starting on http://0.0.0.0:3000");
    println!("   Registered providers: {}", providers.join(", "));
    if providers.len() == 1 && providers.iter().any(|p| p == "ollama") {
        println!("   Only Ollama is registered; make sure the local Ollama server is running.");
    }
    println!("   Try: curl http://localhost:3000/v1/chat/completions ...");
    println!("   Health: curl http://localhost:3000/health");
    println!("   Press Ctrl+C to stop.\n");

    // proxy::serve binds, serves, and handles graceful shutdown on Ctrl+C/SIGTERM
    if let Err(e) = proxy::serve(llm, "0.0.0.0:3000").await {
        eprintln!("Server error: {e}");
        std::process::exit(1);
    }
}
