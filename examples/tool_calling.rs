//! Function / tool calling round-trip example.
//!
//! Demonstrates the full loop: advertise a tool, receive the model's tool
//! call, run it locally, feed the result back, then get a final answer.
//!
//! ```bash
//! export OPENAI_API_KEY="sk-..."
//! export OPENAI_BASE_URL="https://api.openai.com/v1"   # or any compatible endpoint
//! export OPENAI_MODEL="gpt-4o-mini"
//! cargo run --example tool_calling
//! ```

use llmrust::{ChatRequest, LmrsClient, Message, Tool, ToolChoice};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| "OPENAI_API_KEY required")?;
    let base_url = std::env::var("OPENAI_BASE_URL")
        .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
    let model_name = std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());

    let llm = LmrsClient::new();
    llm.set_openai_compatible(&api_key, &base_url).await;
    let model = format!("openai/{model_name}");

    // 1. Describe a tool the model is allowed to call.
    let weather_tool = Tool::function(
        "get_weather",
        Some("Get the current weather for a city".to_string()),
        serde_json::json!({
            "type": "object",
            "properties": {
                "location": {"type": "string", "description": "City name"}
            },
            "required": ["location"]
        }),
    );

    let mut messages = vec![Message::user("What's the weather in San Francisco?")];

    // 2. First round: let the model decide whether to call the tool.
    let req = ChatRequest::from_messages("", messages.clone())
        .with_tools(vec![weather_tool.clone()])
        .with_tool_choice(ToolChoice::auto());
    let resp = llm.chat_with(&model, req).await?;

    let Some(tool_calls) = resp.tool_calls else {
        println!("Model answered directly: {}", resp.content);
        return Ok(());
    };

    // 3. Record the assistant's tool-call turn, then answer each call.
    messages.push(Message::assistant_tool_calls(tool_calls.clone()));
    for call in &tool_calls {
        println!(
            "-> model called {}({})",
            call.function.name, call.function.arguments
        );
        // A real app would parse `call.function.arguments` and do real work.
        let result = r#"{"temperature_c": 21, "condition": "Sunny"}"#;
        messages.push(Message::tool(call.id.clone(), result));
    }

    // 4. Second round: the model produces a natural-language answer.
    let req = ChatRequest::from_messages("", messages).with_tools(vec![weather_tool]);
    let final_resp = llm.chat_with(&model, req).await?;
    println!("\nFinal answer: {}", final_resp.content);

    Ok(())
}
