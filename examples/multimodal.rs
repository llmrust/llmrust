//! Multimodal (text + image) chat example.
//!
//! Demonstrates sending an image alongside text using the OpenAI-compatible
//! vision API. Run with:
//!
//! ```bash
//! OPENAI_API_KEY=sk-... cargo run --example multimodal
//! ```

use llmrust::{ChatRequest, LmrsClient, Message};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api_key = std::env::var("OPENAI_API_KEY")
        .expect("set the OPENAI_API_KEY environment variable to run this example");

    let llm = LmrsClient::new();
    llm.set_openai(api_key).await;

    // A publicly reachable image URL. Providers that support vision will fetch
    // and analyze it; data URLs ("data:image/png;base64,...") work too.
    let image_url = "https://upload.wikimedia.org/wikipedia/commons/thumb/d/dd/Gull_portrait_ca_usa.jpg/640px-Gull_portrait_ca_usa.jpg";

    let messages = vec![Message::user_with_image(
        "What is in this image? Answer in one sentence.",
        image_url,
    )];

    let req = ChatRequest {
        model: String::new(),
        messages,
        temperature: None,
        max_tokens: Some(300),
        stream: false,
        top_p: None,
        tools: None,
        tool_choice: None,
        ..Default::default()
    };

    let resp = llm.chat_with("openai/gpt-4o", req).await?;
    println!("{}", resp.content);

    Ok(())
}
