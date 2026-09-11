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

// ── GRD-003 ②：门自身必须会红 ──────────────────────────────────────────
//
// 本门用 `assert_eq!(actual, expected)` 钉 wire 形状。但**一个不透明的 assert_eq!
// 无法被负例驱动**——它只能证明"今天相等"，不能证明"形状变了会被发现"。
// 故把比较抽成纯函数 [`shape_diff`]（按路径逐项比），负例即可用**人为改过的形状**
// 直接驱动：改键名 / 删字段 / 加字段 → 必须报；相同 → 必须不报。

/// **纯函数**：`actual` 相对 `expected` 的形状差异（按 JSON 路径列出）。
///
/// 覆盖三类漂移：**键被改名**（等于"删一个 + 加一个"）、**字段被删**、**多出字段**；
/// 值变也报（wire 形状包含值）。
pub fn shape_diff(actual: &serde_json::Value, expected: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    diff_at("$", actual, expected, &mut out);
    out
}

fn diff_at(
    path: &str,
    actual: &serde_json::Value,
    expected: &serde_json::Value,
    out: &mut Vec<String>,
) {
    use serde_json::Value;
    match (actual, expected) {
        (Value::Object(a), Value::Object(e)) => {
            for (k, ev) in e {
                match a.get(k) {
                    None => out.push(format!("{path}.{k}: MISSING (expected {ev})")),
                    Some(av) => diff_at(&format!("{path}.{k}"), av, ev, out),
                }
            }
            for k in a.keys() {
                if !e.contains_key(k) {
                    out.push(format!("{path}.{k}: UNEXPECTED (new field on the wire)"));
                }
            }
        }
        (Value::Array(a), Value::Array(e)) => {
            if a.len() != e.len() {
                out.push(format!("{path}: array length {} != {}", a.len(), e.len()));
            }
            for (i, (av, ev)) in a.iter().zip(e.iter()).enumerate() {
                diff_at(&format!("{path}[{i}]"), av, ev, out);
            }
        }
        _ => {
            if actual != expected {
                out.push(format!("{path}: {actual} != {expected}"));
            }
        }
    }
}

/// 正向对照：真实响应与其冻结形状**不得**产生任何差异。
///
/// （保证抽取出的比较与线上钉住的形状是**同一口径**，不是另写一套。）
#[test]
fn real_response_matches_frozen_shape_under_shape_diff() {
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
    let actual = serde_json::to_value(&resp).unwrap();
    let expected = json!({
        "content": "hi",
        "model": "gpt-4o",
        "usage": { "prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15 },
        "finish_reason": "stop"
    });
    let diff = shape_diff(&actual, &expected);
    assert!(diff.is_empty(), "the real shape must match; got {diff:?}");
}

/// 负例一：**字段被删** → 必须报。
#[test]
fn negative_removed_field_is_detected() {
    let expected = json!({"content": "hi", "model": "gpt-4o"});
    let actual = json!({"content": "hi"});
    let diff = shape_diff(&actual, &expected);
    assert_eq!(diff.len(), 1, "got {diff:?}");
    assert!(diff[0].contains("MISSING"), "{diff:?}");
    assert!(diff[0].contains("model"), "{diff:?}");
}

/// 负例二：**多出字段**（新字段上了 wire）→ 必须报。
#[test]
fn negative_added_field_is_detected() {
    let expected = json!({"content": "hi"});
    let actual = json!({"content": "hi", "surprise": 1});
    let diff = shape_diff(&actual, &expected);
    assert_eq!(diff.len(), 1, "got {diff:?}");
    assert!(diff[0].contains("UNEXPECTED"), "{diff:?}");
}

/// 负例三：**键被改名**（wire key 变了）→ 必须报（两向各一条）。
#[test]
fn negative_renamed_key_is_detected() {
    let expected = json!({"total_tokens": 15});
    let actual = json!({"totalTokens": 15});
    let diff = shape_diff(&actual, &expected);
    assert_eq!(
        diff.len(),
        2,
        "rename = one missing + one unexpected; got {diff:?}"
    );
    assert!(diff.iter().any(|d| d.contains("MISSING")), "{diff:?}");
    assert!(diff.iter().any(|d| d.contains("UNEXPECTED")), "{diff:?}");
}

/// 负例四：**值变了**（如 `finish_reason` 的 wire 串被改）→ 必须报。
#[test]
fn negative_changed_value_is_detected() {
    let expected = json!({"finish_reason": "stop"});
    let actual = json!({"finish_reason": "Stop"});
    let diff = shape_diff(&actual, &expected);
    assert_eq!(diff.len(), 1, "got {diff:?}");
    assert!(diff[0].contains("finish_reason"), "{diff:?}");
}

/// 负例五：**嵌套字段**被删 → 必须按路径点名（不能只看顶层）。
#[test]
fn negative_nested_removal_is_detected_with_path() {
    let expected = json!({"usage": {"prompt_tokens": 10, "total_tokens": 15}});
    let actual = json!({"usage": {"prompt_tokens": 10}});
    let diff = shape_diff(&actual, &expected);
    assert_eq!(diff.len(), 1, "got {diff:?}");
    assert!(diff[0].contains("$.usage.total_tokens"), "{diff:?}");
}

/// 负例六：**数组长度变化** → 必须报。
#[test]
fn negative_array_length_change_is_detected() {
    let expected = json!({"tool_calls": [1, 2]});
    let actual = json!({"tool_calls": [1]});
    let diff = shape_diff(&actual, &expected);
    assert!(!diff.is_empty(), "array length change must be detected");
    assert!(diff.iter().any(|d| d.contains("array length")), "{diff:?}");
}

/// 反例（**假阳性**）：`None` 的 `Option` 字段不得被误报为"形状漂移"。
///
/// 实测口径（本测试当场取证，非推断）：`ChatResponse { usage: None, .. }` 序列化为
/// `{"content":…,"model":…,"usage":null}` —— **`usage` 以 `null` 出现**（该字段没有
/// `skip_serializing_if`），而未设的 `tool_calls` / `logprobs` **整个键缺席**。
/// 两者都是既有契约；本测试钉住它们，并证明 `shape_diff` 不会把二者误读成漂移。
#[test]
fn none_options_are_not_reported_as_drift() {
    let resp = ChatResponse {
        content: "hi".to_string(),
        model: "m".to_string(),
        ..Default::default()
    };
    let actual = serde_json::to_value(&resp).unwrap();

    // 取证式断言：usage 是 null 存在，tool_calls/logprobs 是键缺席。
    assert_eq!(actual.get("usage"), Some(&serde_json::Value::Null));
    assert!(
        actual.get("tool_calls").is_none(),
        "unset tool_calls must be absent"
    );
    assert!(
        actual.get("logprobs").is_none(),
        "unset logprobs must be absent"
    );

    // 与真实形状比较 → 不得有任何差异（否则就是假阳性）。
    let expected = json!({"content": "hi", "model": "m", "usage": null});
    assert!(
        shape_diff(&actual, &expected).is_empty(),
        "the true shape must produce no diff; got {:?}",
        shape_diff(&actual, &expected)
    );

    // 反向：把 usage 当成"键缺席"就必须报（证明上面的通过不是因为比较太松）。
    let without_usage = json!({"content": "hi", "model": "m"});
    let diff = shape_diff(&actual, &without_usage);
    assert_eq!(diff.len(), 1, "got {diff:?}");
    assert!(diff[0].contains("UNEXPECTED"), "{diff:?}");
}
