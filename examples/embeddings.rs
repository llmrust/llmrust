//! Embeddings quickstart — demonstrates `LmrsClient::embed` and `embed_batch`.
//!
//! ## Usage
//!
//! ### OpenAI-compatible (default)
//!
//! ```bash
//! export OPENAI_API_KEY="sk-..."
//! cargo run --example embeddings
//! ```
//!
//! ### Ollama (local)
//!
//! ```bash
//! ollama pull nomic-embed-text
//! export LLMRUST_EMBEDDINGS_MODEL="ollama/nomic-embed-text"
//! cargo run --example embeddings
//! ```
//!
//! You can also override the model via `LLMRUST_EMBEDDINGS_MODEL` — the example
//! will use whatever provider prefix + model name you pass (e.g.
//! `openai/text-embedding-3-small`, `ollama/nomic-embed-text`, etc.).

use llmrust::LmrsClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ----- setup --------------------------------------------------------
    let model = std::env::var("LLMRUST_EMBEDDINGS_MODEL")
        .unwrap_or_else(|_| "openai/text-embedding-3-small".to_string());

    let llm = LmrsClient::from_env().await;

    eprintln!("Using model: {model}\n");

    // ----- single embed -------------------------------------------------
    eprintln!("=== single embed ===");

    let response = llm.embed(&model, "hello from llmrust").await?;

    eprintln!("model:          {}", response.model);
    eprintln!("vector count:   {}", response.data.len());
    if let Some(first) = response.data.first() {
        eprintln!("dimension:      {}", first.embedding.len());
    }
    if let Some(usage) = &response.usage {
        eprintln!(
            "usage:          {} prompt tokens, {} total tokens",
            usage.prompt_tokens, usage.total_tokens
        );
    }

    // ----- batch embed --------------------------------------------------
    eprintln!("\n=== batch embed ===");

    let batch_resp = llm
        .embed_batch(
            &model,
            [
                "hello from llmrust",
                "embeddings are useful",
                "rust is fast",
            ],
        )
        .await?;

    eprintln!("model:          {}", batch_resp.model);
    eprintln!("vector count:   {}", batch_resp.data.len());
    if let Some(first) = batch_resp.data.first() {
        eprintln!("dimension:      {}", first.embedding.len());
    }
    if let Some(usage) = &batch_resp.usage {
        eprintln!(
            "usage:          {} prompt tokens, {} total tokens",
            usage.prompt_tokens, usage.total_tokens
        );
    }

    // Confirm index order matches input order
    for emb in &batch_resp.data {
        eprintln!(
            "  data[{}]:  vector of length {}",
            emb.index,
            emb.embedding.len()
        );
    }

    eprintln!("\nDone.");
    Ok(())
}
