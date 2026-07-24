//! API-002 Track ② (snapshot half): pin the exact JSON wire shapes emitted by
//! the consumer-facing response types so a shape change (add/remove/rename
//! field, change JSON key) fails CI. The classification half lives in
//! `tests/api_freeze.rs`.
//!
//! These tests live here (not in `src/types.rs`) on purpose: `src/types.rs` is a
//! CI-003 hotspot file whose line count is frozen, and tests are public-API
//! coverage that belongs in the `tests/` tree anyway. All types exercised here
//! are `pub`, so an integration test can construct and serialize them.

use llmrust::types::{ChatResponse, FinishReason, StreamChunk, Usage};
use serde_json::json;

#[test]
fn finish_reason_known_variants_serialize_to_wire_strings() {
    let cases = [
        (FinishReason::Stop, "stop"),
        (FinishReason::Length, "length"),
        (FinishReason::ToolCalls, "tool_calls"),
        (FinishReason::ContentFilter, "content_filter"),
        (FinishReason::EndTurn, "end_turn"),
        (FinishReason::MaxTokens, "max_tokens"),
        (FinishReason::StopSequence, "stop_sequence"),
        (FinishReason::ToolUse, "tool_use"),
    ];
    for (reason, wire) in cases {
        assert_eq!(
            serde_json::to_value(&reason).unwrap(),
            json!(wire),
            "FinishReason variant must serialize to its wire string"
        );
    }
}

#[test]
fn finish_reason_unknown_round_trips_through_other() {
    // §5.1 wire escape hatch: unknown finish_reason must survive a round trip
    // verbatim. This is the contract that lets 0.1.x tolerate provider strings
    // not yet in the enum without a breaking change.
    for wire in ["custom_stop", "end_turn_v2", ""] {
        let value = json!(wire);
        let parsed: FinishReason = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(parsed, FinishReason::Other(wire.to_string()));
        assert_eq!(serde_json::to_value(&parsed).unwrap(), value);
    }
}

#[test]
fn usage_option_token_counters_distinguish_none_from_some_zero() {
    // `None` vs `Some(0)` must be distinguishable on the wire: `None` omits the
    // key, `Some(0)` emits `"key": 0`. Tolerating a zero cache/reasoning count
    // matters (a cache hit can legitimately cost 0 tokens).
    let none = Usage {
        prompt_tokens: 10,
        completion_tokens: 5,
        total_tokens: 15,
        ..Default::default()
    };
    let v_none = serde_json::to_value(&none).unwrap();
    assert!(v_none.get("cache_read_tokens").is_none());
    assert!(v_none.get("cache_write_tokens").is_none());
    assert!(v_none.get("reasoning_tokens").is_none());

    let some_zero = Usage {
        prompt_tokens: 10,
        completion_tokens: 5,
        total_tokens: 15,
        cache_read_tokens: Some(0),
        cache_write_tokens: Some(0),
        reasoning_tokens: Some(0),
    };
    let v_some = serde_json::to_value(&some_zero).unwrap();
    assert_eq!(v_some["cache_read_tokens"], 0);
    assert_eq!(v_some["cache_write_tokens"], 0);
    assert_eq!(v_some["reasoning_tokens"], 0);
}

#[test]
fn usage_serializes_known_shape() {
    let usage = Usage {
        prompt_tokens: 10,
        completion_tokens: 5,
        total_tokens: 15,
        cache_read_tokens: Some(3),
        cache_write_tokens: Some(1),
        reasoning_tokens: Some(2),
    };
    assert_eq!(
        serde_json::to_value(&usage).unwrap(),
        json!({
            "prompt_tokens": 10,
            "completion_tokens": 5,
            "total_tokens": 15,
            "cache_read_tokens": 3,
            "cache_write_tokens": 1,
            "reasoning_tokens": 2
        })
    );
}

#[test]
fn chat_response_serializes_known_shape() {
    let resp = ChatResponse {
        content: "hi".to_string(),
        model: "gpt-4o".to_string(),
        usage: Some(Usage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
            ..Default::default()
        }),
        finish_reason: Some(FinishReason::Stop),
        ..Default::default()
    };
    let v = serde_json::to_value(&resp).unwrap();
    assert_eq!(
        v,
        json!({
            "content": "hi",
            "model": "gpt-4o",
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 5,
                "total_tokens": 15
            },
            "finish_reason": "stop"
        })
    );
    // unset Option fields must be absent, not null
    assert!(v.get("tool_calls").is_none());
    assert!(v.get("logprobs").is_none());
}

#[test]
fn stream_chunk_serializes_known_shape() {
    let chunk = StreamChunk {
        delta: "hi".to_string(),
        done: false,
        finish_reason: Some(FinishReason::Stop),
        usage: Some(Usage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
            ..Default::default()
        }),
        thinking: Some("think".to_string()),
        ..Default::default()
    };
    let v = serde_json::to_value(&chunk).unwrap();
    assert_eq!(
        v,
        json!({
            "delta": "hi",
            "done": false,
            "finish_reason": "stop",
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 5,
                "total_tokens": 15
            },
            "thinking": "think"
        })
    );
    assert!(v.get("tool_calls").is_none());
    assert!(v.get("thinking_done").is_none());
}
