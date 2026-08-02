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
//! # Optional: enable bearer-token authentication
//! export LLMRUST_PROXY_KEY="dev-secret"
//! # Optional: override the listen address (default: 127.0.0.1:3000).
//! # A non-loopback address requires LLMRUST_PROXY_KEY to be set (SPCC §7.1).
//! export LLMRUST_PROXY_ADDR="127.0.0.1:3000"
//! cargo run --features proxy --example proxy_server
//! ```
//!
//! Call the OpenAI Chat Completions endpoint:
//! ```bash
//! curl http://localhost:3000/v1/chat/completions \
//!   -H "Content-Type: application/json" \
//!   -H "Authorization: Bearer dev-secret" \
//!   -d '{"model": "openai/gpt-4o-mini", "messages": [{"role": "user", "content": "Hello!"}]}'
//! ```
//!
//! Call the Anthropic Messages endpoint:
//! ```bash
//! curl http://localhost:3000/v1/messages \
//!   -H "Content-Type: application/json" \
//!   -H "Authorization: Bearer dev-secret" \
//!   -d '{"model": "anthropic/claude-3-5-sonnet-latest", "max_tokens": 256, "messages": [{"role": "user", "content": "Hello!"}]}'
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

    // proxy::serve binds, serves, and handles graceful shutdown on Ctrl+C/SIGTERM.
    //
    // Secure defaults (SPCC §7.1): bind to the loopback interface by default so
    // the proxy starts without a token. To listen on a specific address, set
    // LLMRUST_PROXY_ADDR (e.g. "0.0.0.0:3000") — a non-loopback address
    // requires LLMRUST_PROXY_KEY to be set.
    let addr = std::env::var("LLMRUST_PROXY_ADDR").unwrap_or_else(|_| "127.0.0.1:3000".to_string());

    println!("\nllmrust proxy server starting on http://{addr}");
    println!("   Registered providers: {}", providers.join(", "));
    if providers.len() == 1 && providers.iter().any(|p| p == "ollama") {
        println!("   Only Ollama is registered; make sure the local Ollama server is running.");
    }
    println!("   Try: curl http://localhost:3000/v1/chat/completions ...");
    println!("   Health: curl http://localhost:3000/health");
    println!("   Press Ctrl+C to stop.\n");

    if let Err(e) = proxy::serve(llm, &addr).await {
        eprintln!("Server error: {e}");
        std::process::exit(1);
    }
}
