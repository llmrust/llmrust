//! End-to-end example exercising the Anthropic provider against a real API
//! endpoint.  Validates chat, streaming (including thinking-block handling),
//! image input, and streaming tool calls.
//!
//! # Required environment variables
//!
//! | Variable | Description |
//! |----------|-------------|
//! | `E2E_API_KEY` | API key for the Anthropic-compatible endpoint |
//! | `E2E_BASE_URL` | Base URL **including** version path, e.g. `https://api.anthropic.com/v1` |
//! | `E2E_MODEL` | Model id, e.g. `claude-sonnet-4-20250514` |
//!
//! > **Note:** The Anthropic provider appends `/messages` to the base URL,
//! > so `E2E_BASE_URL` must point to the version prefix, not the root.
//!
//! ```bash
//! E2E_API_KEY=sk-... E2E_BASE_URL=https://api.anthropic.com/v1 E2E_MODEL=claude-sonnet-4-20250514 \
//!   cargo run --example e2e_anthropic_compat
//! ```

use futures::StreamExt;
use llmrust::{
    providers::anthropic::AnthropicProvider, ChatRequest, ContentPart, LmrsClient, Message,
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
    let provider: Arc<dyn Provider> = Arc::new(AnthropicProvider::new(config));

    let llm = LmrsClient::new();
    llm.set_custom("e2e", provider).await;

    let prefix = "e2e";
    let mut passed = 0u32;
    let mut failed = 0u32;

    // ── 1. Non-streaming chat ────────────────────────────────────
    print!("Test 1 [chat] ... ");
    match llm
        .chat(&format!("{prefix}/{model}"), "Say hi in one sentence.")
        .await
    {
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

    // ── 2. Streaming (thinking blocks are silently skipped) ──────
    print!("Test 2 [stream] ... ");
    {
        let result = llm
            .stream(&format!("{prefix}/{model}"), "What is 3+4? Be brief.")
            .await;
        match result {
            Ok(mut stream) => {
                let mut text = String::new();
                let mut chunks = 0u32;
                let mut finish_reason = None;
                let mut has_error = false;

                while let Some(chunk) = stream.next().await {
                    match chunk {
                        Ok(c) => {
                            chunks += 1;
                            text.push_str(&c.delta);
                            if c.finish_reason.is_some() {
                                finish_reason = c.finish_reason.clone();
                            }
                        }
                        Err(e) => {
                            println!("FAIL: stream error at chunk {chunks}: {e}");
                            has_error = true;
                            break;
                        }
                    }
                }

                if !has_error {
                    println!(
                        "OK  chunks={chunks} finish={:?} text={:?}",
                        finish_reason, text
                    );
                    assert!(!text.is_empty(), "text should not be empty");
                    assert!(finish_reason.is_some(), "should have finish_reason");
                    passed += 1;
                } else {
                    failed += 1;
                }
            }
            Err(e) => {
                println!("FAIL: {e}");
                failed += 1;
            }
        }
    }

    // ── 3. Image input (base64 data URL → Anthropic base64 source)
    print!("Test 3 [image recognition] ... ");
    {
        let png_b64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8\
                        /5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==";
        let data_url = format!("data:image/png;base64,{png_b64}");
        let msg = Message::user_with_parts(vec![
            ContentPart::text("What color is this image?"),
            ContentPart::image_url(&data_url),
        ]);
        let req = ChatRequest::from_messages(&model, vec![msg]).with_max_tokens(100);

        let config2 = ProviderConfig::new(&api_key).with_base_url(&base_url);
        let p = AnthropicProvider::new(config2);
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

    // ── 4. Streaming tool calls ──────────────────────────────────
    print!("Test 4 [stream tool calls] ... ");
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
        let req = ChatRequest::new(&model, "What's the weather in Shanghai?")
            .with_tools(vec![tool])
            .with_max_tokens(300);

        let config2 = ProviderConfig::new(&api_key).with_base_url(&base_url);
        let p = AnthropicProvider::new(config2);

        match p.stream(&req).await {
            Ok(mut stream) => {
                let mut text = String::new();
                let mut tool_calls = None;
                let mut finish_reason = None;
                let mut has_error = false;

                while let Some(chunk) = stream.next().await {
                    match chunk {
                        Ok(c) => {
                            text.push_str(&c.delta);
                            if c.tool_calls.is_some() {
                                tool_calls = c.tool_calls.clone();
                            }
                            if c.finish_reason.is_some() {
                                finish_reason = c.finish_reason.clone();
                            }
                        }
                        Err(e) => {
                            println!("FAIL: stream error: {e}");
                            has_error = true;
                            break;
                        }
                    }
                }

                if !has_error {
                    println!(
                        "OK  text={:?} tool_calls={:?} finish={:?}",
                        text, tool_calls, finish_reason
                    );
                    assert!(finish_reason.is_some());
                    passed += 1;
                } else {
                    failed += 1;
                }
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
        "Anthropic-compatible: {passed} passed, {failed} failed, {} total",
        passed + failed
    );
    println!("========================================");
    if failed > 0 {
        std::process::exit(1);
    }
}
