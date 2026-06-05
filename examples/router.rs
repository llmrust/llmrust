//! Multi-deployment routing with automatic fallback and load balancing.
//!
//! Run with: `cargo run --example router` (requires real API keys in env).

use std::sync::Arc;

use llmrust::{LmrsClient, Router, RoutingStrategy};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Arc::new(LmrsClient::new());
    client.set_openai(std::env::var("OPENAI_API_KEY")?).await;
    client.set_anthropic(std::env::var("ANTHROPIC_API_KEY")?).await;

    let router = Router::new(client)
        .with_strategy(RoutingStrategy::RoundRobin)
        // "smart": try GPT-4o first, fall back to Claude on a transient failure.
        .route(
            "smart",
            ["openai/gpt-4o", "anthropic/claude-sonnet-4-20250514"],
        )
        // "balanced": round-robin across two deployments.
        .route("balanced", ["openai/gpt-4o-mini", "openai/gpt-4o"]);

    let resp = router.chat("smart", "Explain Rust ownership in one sentence.").await?;
    println!("smart -> {}", resp.content);

    for i in 0..3 {
        let resp = router.chat("balanced", "ping").await?;
        println!("balanced #{} -> {} ({})", i, resp.content, resp.model);
    }

    Ok(())
}
