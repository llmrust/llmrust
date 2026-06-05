//! Multi-deployment routing with fallback and round-robin load balancing.
//!
//! Run with:
//!
//! ```bash
//! OPENAI_API_KEY=sk-... ANTHROPIC_API_KEY=sk-ant-... cargo run --example router
//! ```

use llmrust::{LmrsClient, Router, RoutingStrategy};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Arc::new(LmrsClient::new());

    let openai_key = std::env::var("OPENAI_API_KEY")?;
    let anthropic_key = std::env::var("ANTHROPIC_API_KEY")?;
    client.set_openai(openai_key).await;
    client.set_anthropic(anthropic_key).await;

    // `smart` tries gpt-4o first, falling back to Claude on a transient error.
    // `balanced` rotates its starting deployment on each call (round-robin).
    let router = Router::new(client)
        .with_strategy(RoutingStrategy::RoundRobin)
        .route(
            "smart",
            ["openai/gpt-4o", "anthropic/claude-sonnet-4-20250514"],
        )
        .route("balanced", ["openai/gpt-4o-mini", "openai/gpt-4o"]);

    let resp = router.chat("smart", "Explain Rust ownership.").await?;
    println!("smart -> {} ({})", resp.content, resp.model);

    for i in 0..3 {
        let resp = router.chat("balanced", "ping").await?;
        println!("balanced #{} -> {} ({})", i, resp.content, resp.model);
    }

    Ok(())
}
