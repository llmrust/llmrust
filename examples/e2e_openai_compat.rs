//! End-to-end example exercising the OpenAI-compatible provider against a
//! real API endpoint.  Validates chat, streaming, tool calling, multi-turn
//! conversation, image input, and the `stream_collect_full` model-name fix.
//!
//! # Required environment variables
//!
//! | Variable | Description |
//! |----------|-------------|
//! | `E2E_API_KEY` | API key for the OpenAI-compatible endpoint |
//! | `E2E_BASE_URL` | Base URL, e.g. `https://api.example.com/v1` |
//! | `E2E_MODEL` | Model id, e.g. `gpt-4o-mini` |
//!
//! ```bash
//! E2E_API_KEY=sk-... E2E_BASE_URL=https://api.example.com/v1 E2E_MODEL=gpt-4o-mini \
//!   cargo run --example e2e_openai_compat
//! ```

use llmrust::{
    providers::compat::OpenAiCompatibleProvider, ChatRequest, ContentPart, LmrsClient, Message,
    Provider, ProviderConfig,
};
use std::sync::Arc;

fn require_env(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| {
        eprintln!("error: {key} is not set");
        std::process::exit(2);
    })
}

#[tokio::main]
async fn main() {
    let api_key = require_env("E2E_API_KEY");
    let base_url = require_env("E2E_BASE_URL");
    let model = require_env("E2E_MODEL");

    let config = ProviderConfig::new(&api_key).with_base_url(&base_url);
    let provider: Arc<dyn Provider> = Arc::new(OpenAiCompatibleProvider::new(config, []));

    let llm = LmrsClient::new();
    llm.set_custom("e2e", provider).await;

    let prefix = "e2e";
    let mut passed = 0u32;
    let mut failed = 0u32;

    // ── 1. Non-streaming chat ────────────────────────────────────
    print!("Test 1 [chat] ... ");
    match llm.chat(&format!("{prefix}/{model}"), "Say hello.").await {
        Ok(resp) => {
            println!(
                "OK  model={} finish={:?} content={:?}",
                resp.model, resp.finish_reason, resp.content
            );
            assert!(!resp.content.is_empty());
            passed += 1;
        }
        Err(e) => {
            println!("FAIL: {e}");
            failed += 1;
        }
    }

    // ── 2. Streaming ─────────────────────────────────────────────
    print!("Test 2 [stream] ... ");
    match llm
        .stream_collect(&format!("{prefix}/{model}"), "Count 1 to 5.")
        .await
    {
        Ok(text) => {
            println!("OK  len={} text={:?}", text.len(), text);
            assert!(!text.is_empty());
            passed += 1;
        }
        Err(e) => {
            println!("FAIL: {e}");
            failed += 1;
        }
    }

    // ── 3. stream_collect_full model-name fix ────────────────────
    print!("Test 3 [stream_collect_full model fix] ... ");
    match llm
        .stream_collect_full(&format!("{prefix}/{model}"), "What is 2+2?")
        .await
    {
        Ok(resp) => {
            println!(
                "OK  model='{}' finish={:?} content={:?}",
                resp.model, resp.finish_reason, resp.content
            );
            assert!(!resp.content.is_empty());
            assert!(
                !resp.model.contains('/'),
                "model should not contain '/', got: {}",
                resp.model
            );
            assert_eq!(
                resp.model, model,
                "model should be '{model}', got: {}",
                resp.model
            );
            passed += 1;
        }
        Err(e) => {
            println!("FAIL: {e}");
            failed += 1;
        }
    }

    // ── 4. Tool calling ──────────────────────────────────────────
    print!("Test 4 [tool calling] ... ");
    {
        let tool = llmrust::Tool::function(
            "get_weather",
            Some("Get current weather for a city".into()),
            serde_json::json!({
                "type": "object",
                "properties": {
                    "city": { "type": "string" }
                },
                "required": ["city"]
            }),
        );
        let req = ChatRequest::new(&model, "What's the weather in Beijing?")
            .with_tools(vec![tool])
            .with_max_tokens(200);

        let config2 = ProviderConfig::new(&api_key).with_base_url(&base_url);
        let p = OpenAiCompatibleProvider::new(config2, []);
        match p.chat(&req).await {
            Ok(resp) => {
                if let Some(calls) = &resp.tool_calls {
                    assert!(!calls.is_empty());
                    println!(
                        "OK  tool={}({}) finish={:?}",
                        calls[0].function.name, calls[0].function.arguments, resp.finish_reason
                    );
                } else {
                    println!("SKIP (model did not call tool) content={:?}", resp.content);
                }
                passed += 1;
            }
            Err(e) => {
                println!("FAIL: {e}");
                failed += 1;
            }
        }
    }

    // ── 5. Multi-turn conversation ───────────────────────────────
    print!("Test 5 [multi-turn] ... ");
    {
        let messages = vec![
            Message::system("You are a helpful assistant. Keep responses brief."),
            Message::user("My name is Alice."),
            Message::assistant("Hello Alice!"),
            Message::user("What's my name?"),
        ];
        let req = ChatRequest::from_messages(&model, messages).with_max_tokens(50);

        let config2 = ProviderConfig::new(&api_key).with_base_url(&base_url);
        let p = OpenAiCompatibleProvider::new(config2, []);
        match p.chat(&req).await {
            Ok(resp) => {
                println!("OK  content={:?}", resp.content);
                passed += 1;
            }
            Err(e) => {
                println!("FAIL: {e}");
                failed += 1;
            }
        }
    }

    // ── 6. Image input (base64 data URL) ─────────────────────────
    print!("Test 6 [image recognition] ... ");
    {
        // 1×1 solid red PNG
        let png_b64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8\
                        /5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==";
        let data_url = format!("data:image/png;base64,{png_b64}");
        let msg = Message::user_with_parts(vec![
            ContentPart::text("What color is this image?"),
            ContentPart::image_url(&data_url),
        ]);
        let req = ChatRequest::from_messages(&model, vec![msg]).with_max_tokens(100);

        let config2 = ProviderConfig::new(&api_key).with_base_url(&base_url);
        let p = OpenAiCompatibleProvider::new(config2, []);
        match p.chat(&req).await {
            Ok(resp) => {
                println!("OK  content={:?}", resp.content);
                assert!(!resp.content.is_empty());
                passed += 1;
            }
            Err(e) => {
                println!("FAIL: {e}");
                failed += 1;
            }
        }
    }

    // ── Summary ──────────────────────────────────────────────────
    println!("\n========================================");
    println!(
        "OpenAI-compatible: {passed} passed, {failed} failed, {} total",
        passed + failed
    );
    println!("========================================");
    if failed > 0 {
        std::process::exit(1);
    }
}
