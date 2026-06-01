//! Demo: call all 7 supported providers through llmrust.
//!
//! Set API keys as environment variables before running. The demo will only
//! call providers whose key is set:
//!
//! ```bash
//! export OPENAI_API_KEY="sk-..."
//! export ANTHROPIC_API_KEY="sk-ant-..."
//! export DEEPSEEK_API_KEY="sk-..."
//! export GOOGLE_API_KEY="AIza..."
//! export MOONSHOT_API_KEY="sk-..."
//! export OPENROUTER_API_KEY="sk-or-..."
//! # OLLAMA_HOST is optional. If unset, defaults to http://localhost:11434
//! export OLLAMA_HOST="http://localhost:11434"
//! cargo run --example demo
//! ```

use futures::StreamExt;
use llmrust::LiteLLM;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let llm = LiteLLM::new();

    // Register providers from environment variables. Each is silently skipped
    // if the corresponding env var is missing or empty.
    if let Ok(key) = std::env::var("OPENAI_API_KEY") {
        if !key.is_empty() {
            llm.set_openai(&key).await;
        }
    }
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        if !key.is_empty() {
            llm.set_anthropic(&key).await;
        }
    }
    if let Ok(key) = std::env::var("DEEPSEEK_API_KEY") {
        if !key.is_empty() {
            llm.set_deepseek(&key).await;
        }
    }
    if let Ok(key) = std::env::var("GOOGLE_API_KEY") {
        if !key.is_empty() {
            llm.set_google(&key).await;
        }
    }
    if let Ok(key) = std::env::var("MOONSHOT_API_KEY") {
        if !key.is_empty() {
            llm.set_moonshot(&key).await;
        }
    }
    if let Ok(key) = std::env::var("OPENROUTER_API_KEY") {
        if !key.is_empty() {
            llm.set_openrouter(&key).await;
        }
    }

    // Ollama is always registered — it needs no key, just a local server.
    // If OLLAMA_HOST is set we use it; otherwise the provider falls back to
    // its built-in default of http://localhost:11434.
    let ollama_host = std::env::var("OLLAMA_HOST").ok().filter(|s| !s.is_empty());
    llm.set_ollama(ollama_host).await;

    let providers = llm.providers().await;
    if providers.is_empty() {
        println!("No API keys found. Set at least one of:");
        println!("  OPENAI_API_KEY, ANTHROPIC_API_KEY, DEEPSEEK_API_KEY,");
        println!("  GOOGLE_API_KEY, MOONSHOT_API_KEY, OPENROUTER_API_KEY");
        println!("Ollama runs locally without a key (set OLLAMA_HOST if non-default).");
        return Ok(());
    }
    println!("Registered providers: {:?}\n", providers);

    // --- Non-streaming example ---
    // Each entry: (provider key, model string, prompt). Providers without a
    // key are skipped gracefully.
    let models = [
        ("openai", "openai/gpt-4o-mini", "Say hello in one sentence."),
        (
            "anthropic",
            "anthropic/claude-sonnet-4-20250514",
            "Say hello in one sentence.",
        ),
        (
            "deepseek",
            "deepseek/deepseek-chat",
            "Say hello in one sentence.",
        ),
        (
            "google",
            "google/gemini-2.0-flash",
            "Say hello in one sentence.",
        ),
        (
            "moonshot",
            "moonshot/moonshot-v1-8k",
            "Say hello in one sentence.",
        ),
        (
            "openrouter",
            "openrouter/anthropic/claude-3.5-sonnet",
            "Say hello in one sentence.",
        ),
        ("ollama", "ollama/llama3.2", "Say hello in one sentence."),
    ];

    for (name, model, prompt) in &models {
        if !providers.contains(&name.to_string()) {
            println!("[{}] skipped (no API key or not running locally)\n", name);
            continue;
        }
        print!("[{}] {} => ", name, model);
        match llm.chat(model, prompt).await {
            Ok(resp) => {
                println!("{}", resp.content);
                if let Some(usage) = &resp.usage {
                    println!(
                        "  (tokens: {} in / {} out)",
                        usage.prompt_tokens, usage.completion_tokens
                    );
                }
            }
            Err(e) => println!("ERROR: {}", e),
        }
        println!();
    }

    // --- Streaming example ---
    // Pick any provider that's registered, preferring cheap/fast ones.
    let stream_model = if providers.contains(&"deepseek".to_string()) {
        Some("deepseek/deepseek-chat")
    } else if providers.contains(&"openai".to_string()) {
        Some("openai/gpt-4o-mini")
    } else if providers.contains(&"anthropic".to_string()) {
        Some("anthropic/claude-sonnet-4-20250514")
    } else if providers.contains(&"google".to_string()) {
        Some("google/gemini-2.0-flash")
    } else if providers.contains(&"ollama".to_string()) {
        Some("ollama/llama3.2")
    } else if providers.contains(&"moonshot".to_string()) {
        Some("moonshot/moonshot-v1-8k")
    } else if providers.contains(&"openrouter".to_string()) {
        Some("openrouter/anthropic/claude-3.5-sonnet")
    } else {
        None
    };

    if let Some(stream_model) = stream_model {
        println!("--- Streaming from {} ---", stream_model);
        let mut stream = llm
            .stream(stream_model, "Write a haiku about Rust programming.")
            .await?;
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(c) => print!("{}", c.delta),
                Err(e) => eprintln!("\nStream error: {}", e),
            }
        }
        println!("\n--- Done ---");
    } else {
        println!("(skipping streaming example: no providers registered)");
    }

    Ok(())
}
