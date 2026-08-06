//! Contract tests for the Responses API (`/v1/responses`) proxy endpoint.
//!
//! Verifies the wire contract defined for the Responses-API handler:
//! - request conversion (`input`/`instructions`/`tools` → ChatRequest)
//! - non-streaming response object shape
//! - streaming SSE event sequence (created → output_item → delta → done → completed)
//! - tool-call surfacing (function_call items + arguments deltas)
//!
//! No real API calls — only fake providers.

use std::sync::Arc;

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use futures::stream::{self, BoxStream};
use llmrust::{
    ChatRequest, ChatResponse, FinishReason, FunctionCall, LlmError, LmrsClient, Provider,
    StreamChunk, ToolCall, Usage,
};
use tower::ServiceExt;

// ── Fake providers ─────────────────────────────

/// Provider that returns a fixed text reply.
struct MockProvider;

#[async_trait::async_trait]
impl Provider for MockProvider {
    async fn chat(&self, _req: &ChatRequest) -> llmrust::Result<ChatResponse> {
        Ok(ChatResponse {
            content: "mocked reply".to_string(),
            model: "mock-model".to_string(),
            usage: Some(Usage {
                prompt_tokens: 3,
                completion_tokens: 5,
                total_tokens: 8,
                ..Default::default()
            }),
            ..Default::default()
        })
    }

    async fn stream(
        &self,
        _req: &ChatRequest,
    ) -> llmrust::Result<BoxStream<'static, llmrust::Result<StreamChunk>>> {
        let chunks: Vec<llmrust::Result<StreamChunk>> = vec![
            Ok(StreamChunk {
                delta: "mock ".to_string(),
                ..Default::default()
            }),
            Ok(StreamChunk {
                delta: "stream".to_string(),
                ..Default::default()
            }),
            Ok(StreamChunk {
                done: true,
                finish_reason: Some(FinishReason::Stop),
                ..Default::default()
            }),
            Ok(StreamChunk {
                usage: Some(Usage {
                    prompt_tokens: 3,
                    completion_tokens: 5,
                    total_tokens: 8,
                    ..Default::default()
                }),
                ..Default::default()
            }),
        ];
        Ok(Box::pin(stream::iter(chunks)))
    }
}

/// Provider that returns a single tool call.
struct ToolCallProvider;

#[async_trait::async_trait]
impl Provider for ToolCallProvider {
    async fn chat(&self, _req: &ChatRequest) -> llmrust::Result<ChatResponse> {
        Ok(ChatResponse {
            content: String::new(),
            model: "tool-model".to_string(),
            tool_calls: Some(vec![ToolCall {
                id: "call_1".to_string(),
                call_type: "function".to_string(),
                function: FunctionCall {
                    name: "get_weather".to_string(),
                    arguments: r#"{"city":"SF"}"#.to_string(),
                },
            }]),
            finish_reason: Some(FinishReason::ToolCalls),
            ..Default::default()
        })
    }

    async fn stream(
        &self,
        _req: &ChatRequest,
    ) -> llmrust::Result<BoxStream<'static, llmrust::Result<StreamChunk>>> {
        let chunks: Vec<llmrust::Result<StreamChunk>> = vec![Ok(StreamChunk {
            done: true,
            finish_reason: Some(FinishReason::ToolCalls),
            tool_calls: Some(vec![ToolCall {
                id: "call_1".to_string(),
                call_type: "function".to_string(),
                function: FunctionCall {
                    name: "get_weather".to_string(),
                    arguments: r#"{"city":"SF"}"#.to_string(),
                },
            }]),
            ..Default::default()
        })];
        Ok(Box::pin(stream::iter(chunks)))
    }
}

/// Provider that returns an upstream API error.
struct ApiErrorProvider;

#[async_trait::async_trait]
impl Provider for ApiErrorProvider {
    async fn chat(&self, _req: &ChatRequest) -> llmrust::Result<ChatResponse> {
        Err(LlmError::Api {
            status: StatusCode::TOO_MANY_REQUESTS.as_u16(),
            message: "rate limited".to_string(),
        })
    }

    async fn stream(
        &self,
        _req: &ChatRequest,
    ) -> llmrust::Result<BoxStream<'static, llmrust::Result<StreamChunk>>> {
        Err(LlmError::Api {
            status: StatusCode::TOO_MANY_REQUESTS.as_u16(),
            message: "rate limited".to_string(),
        })
    }
}

// ── Helpers ─────────────────────────────

fn build_responses_request(body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("failed to build responses test request")
}

fn sse_json_events(text: &str) -> Vec<serde_json::Value> {
    text.lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim)
        .filter(|data| !data.is_empty() && *data != "[DONE]")
        .map(|data| serde_json::from_str(data).expect("SSE data is valid JSON"))
        .collect()
}

// ── Tests ─────────────────────────────

#[tokio::test]
async fn responses_non_stream_with_mock_provider() {
    let llm = Arc::new(LmrsClient::new());
    llm.set_custom("mock", Arc::new(MockProvider)).await;
    let app = llmrust::proxy::router(llm);

    let body = serde_json::json!({
        "model": "mock/test-model",
        "input": [{"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hi"}]}],
    })
    .to_string();

    let response = app
        .oneshot(build_responses_request(&body))
        .await
        .expect("request failed");
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body read failed");
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("body is not valid JSON");

    assert_eq!(json["object"], "response");
    assert_eq!(json["status"], "completed");
    assert_eq!(json["model"], "test-model");
    assert_eq!(json["output"][0]["type"], "message");
    assert_eq!(json["output"][0]["role"], "assistant");
    assert_eq!(json["output"][0]["content"][0]["type"], "output_text");
    assert_eq!(json["output"][0]["content"][0]["text"], "mocked reply");
    assert_eq!(json["usage"]["input_tokens"], 3);
    assert_eq!(json["usage"]["output_tokens"], 5);
}

#[tokio::test]
async fn responses_non_stream_accepts_string_input() {
    let llm = Arc::new(LmrsClient::new());
    llm.set_custom("mock", Arc::new(MockProvider)).await;
    let app = llmrust::proxy::router(llm);

    let body = serde_json::json!({
        "model": "mock/test-model",
        "input": "hello there",
    })
    .to_string();

    let response = app
        .oneshot(build_responses_request(&body))
        .await
        .expect("request failed");
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body read failed");
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("body is not valid JSON");
    assert_eq!(json["output"][0]["content"][0]["text"], "mocked reply");
}

#[tokio::test]
async fn responses_non_stream_forwards_tool_calls() {
    let llm = Arc::new(LmrsClient::new());
    llm.set_custom("tool", Arc::new(ToolCallProvider)).await;
    let app = llmrust::proxy::router(llm);

    let body = serde_json::json!({
        "model": "tool/tool-model",
        "input": [{"type": "message", "role": "user", "content": [{"type": "input_text", "text": "weather?"}]}],
        "tools": [{"type": "function", "name": "get_weather", "description": "weather", "parameters": {"type": "object", "properties": {"city": {"type": "string"}}}}],
    })
    .to_string();

    let response = app
        .oneshot(build_responses_request(&body))
        .await
        .expect("request failed");
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body read failed");
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("body is not valid JSON");

    let mut found_call = false;
    if let Some(output) = json["output"].as_array() {
        for item in output {
            if item["type"] == "function_call" {
                found_call = true;
                assert_eq!(item["name"], "get_weather");
                assert_eq!(item["arguments"], r#"{"city":"SF"}"#);
            }
        }
    }
    assert!(
        found_call,
        "expected a function_call output item, got: {json}"
    );
}

#[tokio::test]
async fn responses_stream_with_mock_provider() {
    let llm = Arc::new(LmrsClient::new());
    llm.set_custom("mock", Arc::new(MockProvider)).await;
    let app = llmrust::proxy::router(llm);

    let body = serde_json::json!({
        "model": "mock/test-model",
        "input": [{"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hi"}]}],
        "stream": true,
    })
    .to_string();

    let response = app
        .oneshot(build_responses_request(&body))
        .await
        .expect("request failed");
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body read failed");
    let text = String::from_utf8(bytes.to_vec()).expect("body is not UTF-8");
    let events = sse_json_events(&text);

    let types: Vec<&str> = events.iter().filter_map(|e| e["type"].as_str()).collect();
    assert!(
        types.contains(&"response.created"),
        "missing response.created, got events: {types:?}"
    );
    assert!(
        types.contains(&"response.output_item.added"),
        "missing output_item.added, got events: {types:?}"
    );
    assert!(
        types.contains(&"response.content_part.added"),
        "missing content_part.added, got events: {types:?}"
    );
    assert!(
        types.contains(&"response.output_text.delta"),
        "missing output_text.delta, got events: {types:?}"
    );
    assert!(
        types.contains(&"response.output_item.done"),
        "missing output_item.done, got events: {types:?}"
    );
    assert!(
        types.contains(&"response.completed"),
        "missing response.completed, got events: {types:?}"
    );
    assert!(
        text.contains("[DONE]"),
        "stream must end with [DONE], got: {text}"
    );

    // Delta payloads must carry item_id/output_index/content_index for
    // client-side assembly (Codex requires these).
    let deltas: Vec<&serde_json::Value> = events
        .iter()
        .filter(|e| e["type"] == "response.output_text.delta")
        .collect();
    assert!(!deltas.is_empty());
    for d in deltas {
        assert!(d["item_id"].is_string(), "delta missing item_id: {d}");
        assert_eq!(d["output_index"], 0);
        assert_eq!(d["content_index"], 0);
    }
}

#[tokio::test]
async fn responses_stream_forwards_tool_calls() {
    let llm = Arc::new(LmrsClient::new());
    llm.set_custom("tool", Arc::new(ToolCallProvider)).await;
    let app = llmrust::proxy::router(llm);

    let body = serde_json::json!({
        "model": "tool/tool-model",
        "input": [{"type": "message", "role": "user", "content": [{"type": "input_text", "text": "weather?"}]}],
        "stream": true,
    })
    .to_string();

    let response = app
        .oneshot(build_responses_request(&body))
        .await
        .expect("request failed");
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body read failed");
    let text = String::from_utf8(bytes.to_vec()).expect("body is not UTF-8");
    let events = sse_json_events(&text);

    let mut saw_func_item = false;
    let mut saw_args_delta = false;
    for e in &events {
        match e["type"].as_str() {
            Some("response.output_item.added") => {
                if e["item"]["type"] == "function_call" {
                    saw_func_item = true;
                    assert_eq!(e["item"]["name"], "get_weather");
                }
            }
            Some("response.function_call_arguments.delta") => {
                saw_args_delta = true;
            }
            Some("response.output_item.done") if e["item"]["type"] == "function_call" => {
                assert_eq!(e["item"]["name"], "get_weather");
                assert_eq!(e["item"]["arguments"], r#"{"city":"SF"}"#);
            }
            Some("response.output_item.done") => {}
            _ => {}
        }
    }
    assert!(
        saw_func_item,
        "expected a function_call output_item, got events: {text}"
    );
    assert!(
        saw_args_delta,
        "expected function_call_arguments.delta, got events: {text}"
    );
}

#[tokio::test]
async fn responses_missing_model_returns_400() {
    let llm = Arc::new(LmrsClient::new());
    llm.set_custom("mock", Arc::new(MockProvider)).await;
    let app = llmrust::proxy::router(llm);

    let body = serde_json::json!({
        "input": [{"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hi"}]}],
    })
    .to_string();

    let response = app
        .oneshot(build_responses_request(&body))
        .await
        .expect("request failed");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn responses_unknown_provider_returns_error() {
    let llm = Arc::new(LmrsClient::new());
    let app = llmrust::proxy::router(llm);

    let body = serde_json::json!({
        "model": "nope/test",
        "input": "hi",
    })
    .to_string();

    let response = app
        .oneshot(build_responses_request(&body))
        .await
        .expect("request failed");
    // Unknown provider must not be 200; llmrust maps it to an error status.
    assert_ne!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn responses_upstream_error_maps_to_error_body() {
    let llm = Arc::new(LmrsClient::new());
    llm.set_custom("err", Arc::new(ApiErrorProvider)).await;
    let app = llmrust::proxy::router(llm);

    let body = serde_json::json!({
        "model": "err/test",
        "input": "hi",
    })
    .to_string();

    let response = app
        .oneshot(build_responses_request(&body))
        .await
        .expect("request failed");
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);

    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body read failed");
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("body is not valid JSON");
    assert!(
        json["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("rate limited")),
        "error message should contain upstream message, got: {json}"
    );
}

#[tokio::test]
async fn responses_auth_router_requires_token() {
    let llm = Arc::new(LmrsClient::new());
    llm.set_custom("mock", Arc::new(MockProvider)).await;
    let app = llmrust::proxy::router_with_auth(llm, "secret".to_string());

    let body = serde_json::json!({
        "model": "mock/test",
        "input": "hi",
    })
    .to_string();

    // No auth header → 401.
    let response = app
        .clone()
        .oneshot(build_responses_request(&body))
        .await
        .expect("request failed");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // Valid bearer → OK.
    let req = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("content-type", "application/json")
        .header("authorization", "Bearer secret")
        .body(Body::from(body.clone()))
        .expect("failed to build request");
    let response = app.oneshot(req).await.expect("request failed");
    assert_eq!(response.status(), StatusCode::OK);
}
