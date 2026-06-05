//! Quick multi-turn conversation test — exercises the library directly.
//! Usage: `cargo run --example multiturn`

use llmrust::{ChatRequest, LmrsClient, Message};
use std::io::Write;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| "OPENAI_API_KEY required")?;
    let base_url = std::env::var("OPENAI_BASE_URL")
        .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
    let model_name = std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());

    let llm = LmrsClient::new();
    llm.set_openai_compatible(&api_key, &base_url).await;
    let model = format!("openai/{model_name}");

    println!("=== Turn 1: greeting ===");
    let history = vec![
        Message::system("你是一个 Rust 专家，回答必须用中文，且不超过两句话。"),
        Message::user("你好，我叫小明。"),
    ];
    let req = ChatRequest {
        model: String::new(),
        messages: history.clone(),
        temperature: None,
        max_tokens: None,
        stream: false,
        top_p: None,
    };
    let resp1 = llm.chat_with(&model, req).await?;
    println!("assistant: {}", resp1.content);
    println!("tokens: {:?}\n", resp1.usage);

    println!("=== Turn 2: follow-up with full history ===");
    let mut history = history;
    history.push(Message::assistant(&resp1.content));
    history.push(Message::user("我下一步该学什么？"));
    let req = ChatRequest {
        model: String::new(),
        messages: history.clone(),
        temperature: None,
        max_tokens: None,
        stream: false,
        top_p: None,
    };
    let resp2 = llm.chat_with(&model, req).await?;
    println!("assistant: {}", resp2.content);
    println!("tokens: {:?}\n", resp2.usage);

    println!("=== Turn 3: streaming with full history ===");
    history.push(Message::assistant(&resp2.content));
    history.push(Message::user("能用一句话总结吗？"));
    let req = ChatRequest {
        model: String::new(),
        messages: history.clone(),
        temperature: None,
        max_tokens: None,
        stream: true,
        top_p: None,
    };

    use futures::StreamExt;
    let mut s = llm.stream_with(&model, req).await?;
    print!("assistant: ");
    let mut full = String::new();
    while let Some(chunk) = s.next().await {
        let chunk = chunk?;
        print!("{}", chunk.delta);
        std::io::stdout().flush().ok();
        full.push_str(&chunk.delta);
    }
    println!();
    println!("[streamed {} chars]", full.chars().count());

    Ok(())
}
