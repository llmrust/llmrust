//! API-002 Track ②: `api_freeze` — machine-check the API-001 classification
//! boundaries recorded in `docs/api-inventory.json`.
//!
//! This test is **fail-closed**: it hardcodes the expected classification for
//! every frozen (`STABLE` / `STABLE-ADDITIVE`) symbol and for the `UNSTABLE`
//! proxy group. Editing `docs/api-inventory.json` to loosen a classification
//! (to let a breaking change slip through) makes this test fail, so the gate
//! cannot be defeated by hand-editing the classification. The actual wire
//! shapes are pinned separately by the integration tests in `tests/response_freeze.rs`
//! (they intentionally live outside `src/types.rs` to respect the CI-003 hotspot
//! baseline for that file).

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Symbol {
    name: String,
    #[allow(dead_code)]
    kind: String,
    non_exhaustive: bool,
    root_reexported: bool,
    classification: String,
    reason: String,
}

#[derive(Debug, Deserialize)]
struct Inventory {
    schema: String,
    adjudicated_at: Option<String>,
    symbols: Vec<Symbol>,
}

fn inventory_path() -> PathBuf {
    // tests/ is a sibling of docs/ at the crate root.
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("docs");
    p.push("api-inventory.json");
    p
}

fn load_inventory() -> Inventory {
    let text = fs::read_to_string(inventory_path())
        .expect("docs/api-inventory.json must exist (API-001 deliverable)");
    serde_json::from_str(&text).expect("docs/api-inventory.json must be valid JSON")
}

fn symbol<'a>(inv: &'a Inventory, name: &str) -> &'a Symbol {
    by_name(inv)
        .get(name)
        .unwrap_or_else(|| panic!("api-inventory.json is missing symbol {name}"))
}

fn by_name(inv: &Inventory) -> HashMap<&str, &Symbol> {
    inv.symbols.iter().map(|s| (s.name.as_str(), s)).collect()
}

#[test]
fn inventory_is_adjudicated_baseline() {
    let inv = load_inventory();
    assert_eq!(inv.schema, "llmrust-api-inventory/1.0");
    assert!(
        inv.adjudicated_at.as_deref().is_some_and(|s| !s.is_empty()),
        "api-inventory.json must carry adjudicated_at (API-001 baseline)"
    );
}

#[test]
fn finish_reason_variant_set_is_frozen() {
    // D1: FinishReason is STABLE with its variant set FROZEN for 0.1.x.
    let inv = load_inventory();
    let s = symbol(&inv, "FinishReason");
    assert_eq!(s.classification, "STABLE");
    assert!(
        !s.non_exhaustive,
        "D1: adding #[non_exhaustive] now would itself break exhaustively-matching downstream"
    );
    assert!(
        s.reason.contains("FROZEN") || s.reason.contains("D1"),
        "FinishReason reason must record the frozen D1 adjudication"
    );
}

#[test]
fn chat_response_shape_is_frozen() {
    // D2: ChatResponse is STABLE, no new field in 0.1.x.
    let inv = load_inventory();
    let s = symbol(&inv, "ChatResponse");
    assert_eq!(s.classification, "STABLE");
    assert!(!s.non_exhaustive);
    assert!(
        s.reason.contains("D2"),
        "ChatResponse reason must record D2"
    );
}

#[test]
fn thinking_config_is_stable_but_not_root_reexported() {
    // D3: ThinkingConfig is STABLE but (gap) not root-reexported in 0.1.x.
    let inv = load_inventory();
    let s = symbol(&inv, "ThinkingConfig");
    assert_eq!(s.classification, "STABLE");
    assert!(
        !s.root_reexported,
        "D3: root-reexport gap is deferred to 0.2 (reachable via llmrust::types)"
    );
    assert!(
        s.reason.contains("D3"),
        "ThinkingConfig reason must record D3"
    );
}

#[test]
fn stable_additive_symbols_require_non_exhaustive() {
    // STABLE-ADDITIVE is only meaningful with #[non_exhaustive]; without it the
    // "additive" promise is a lie (adding a field would be breaking).
    let inv = load_inventory();
    for name in ["ChatRequest", "EmbeddingRequest"] {
        let s = symbol(&inv, name);
        assert_eq!(
            s.classification, "STABLE-ADDITIVE",
            "{name} must be STABLE-ADDITIVE"
        );
        assert!(
            s.non_exhaustive,
            "{name} STABLE-ADDITIVE requires #[non_exhaustive]"
        );
    }
}

#[test]
fn proxy_module_is_unstable() {
    // D6: the proxy DTO group is UNSTABLE (wire-facing, may evolve in 0.1.x).
    // This is the classification half of the proxy exemption — Track ①
    // (cargo-semver-checks) does not even compile the proxy feature, so the
    // UNSTABLE claim here is what keeps proxy changes out of the semver gate
    // without silently weakening it for STABLE types.
    let inv = load_inventory();
    let s = symbol(&inv, "proxy module (group)");
    assert_eq!(s.classification, "UNSTABLE");
    assert!(s.reason.contains("D6"), "proxy reason must record D6");
}

#[test]
fn stable_symbols_are_not_misclassified() {
    // Catch any STABLE symbol that was quietly reclassified to a looser bucket.
    // We enumerate the frozen set expected by API-002 and assert each is STABLE.
    let inv = load_inventory();
    let frozen = [
        "Role",
        "Tool",
        "FunctionDef",
        "ToolChoice",
        "ToolChoiceFunction",
        "ToolCall",
        "FunctionCall",
        "ContentPart",
        "ImageUrl",
        "Content",
        "Message",
        "Usage",
        "LogProbs",
        "TokenLogProb",
        "TopLogProb",
        "FinishReason",
        "ChatResponse",
        "StreamChunk",
        "ResponseFormat",
        "ThinkingConfig",
        "Embedding",
        "EmbeddingUsage",
        "EmbeddingResponse",
        "LmrsClient",
        "Provider",
        "RetryProvider",
        "Router",
        "RoutingStrategy",
        "ModelPricing",
    ];
    for name in frozen {
        let s = symbol(&inv, name);
        assert_eq!(s.classification, "STABLE", "{name} must remain STABLE");
    }
}

// ── GRD-003 ②：门自身必须会红（本文件此前 11 条测试全是正向） ──────────
//
// 本门的**核心比较**是"`docs/api-inventory.json` 的分类 vs API-002 冻结的预期"。
// 把它抽成纯函数 [`expectation_problems`]，负例即可用**被人为放宽的合成清单**直接驱动，
// 无需改动真文件——这正是"手改分类想让破坏性变更溜过去时必须红"的机器证明。

/// API-002 冻结的 STABLE 名单（与 `stable_symbols_are_not_misclassified` 同源）。
fn frozen_stable_names() -> Vec<&'static str> {
    vec![
        "Role",
        "Tool",
        "FunctionDef",
        "ToolChoice",
        "ToolChoiceFunction",
        "ToolCall",
        "FunctionCall",
        "ContentPart",
        "ImageUrl",
        "Content",
        "Message",
        "Usage",
        "LogProbs",
        "TokenLogProb",
        "TopLogProb",
        "FinishReason",
        "ChatResponse",
        "StreamChunk",
        "ResponseFormat",
        "ThinkingConfig",
        "Embedding",
        "EmbeddingUsage",
        "EmbeddingResponse",
        "LmrsClient",
        "Provider",
        "RetryProvider",
        "Router",
        "RoutingStrategy",
        "ModelPricing",
    ]
}

/// **纯函数**：清单相对冻结预期的全部问题（空 = 合规）。
///
/// 规则：
/// 1. 冻结名单里的每个符号必须**存在**且分类为 `STABLE`；
/// 2. `ChatRequest` / `EmbeddingRequest` 必须是 `STABLE-ADDITIVE` **且** `non_exhaustive`
///    （没有 `non_exhaustive` 时"可加字段"的承诺是假的）；
/// 3. 冻结名单里的符号**不得**被标为 `non_exhaustive`（那本身就是破坏性变更）。
fn expectation_problems(inv: &Inventory) -> Vec<String> {
    let map = by_name(inv);
    let mut problems = Vec::new();

    for name in frozen_stable_names() {
        match map.get(name) {
            None => problems.push(format!("{name}: missing from the inventory")),
            Some(s) => {
                if s.classification != "STABLE" {
                    problems.push(format!(
                        "{name}: classification is `{}` but API-002 freezes it as STABLE",
                        s.classification
                    ));
                }
                if s.non_exhaustive {
                    problems.push(format!(
                        "{name}: marked #[non_exhaustive] but is STABLE (that itself breaks \
                         exhaustively-matching downstream callers)"
                    ));
                }
            }
        }
    }

    for name in ["ChatRequest", "EmbeddingRequest"] {
        match map.get(name) {
            None => problems.push(format!("{name}: missing from the inventory")),
            Some(s) => {
                if s.classification != "STABLE-ADDITIVE" {
                    problems.push(format!(
                        "{name}: classification is `{}` but must be STABLE-ADDITIVE",
                        s.classification
                    ));
                }
                if !s.non_exhaustive {
                    problems.push(format!(
                        "{name}: STABLE-ADDITIVE requires #[non_exhaustive] (otherwise the \
                         additive promise is a lie)"
                    ));
                }
            }
        }
    }

    problems
}

fn inv_from(symbols: Vec<(&str, &str, bool)>) -> Inventory {
    Inventory {
        schema: "llmrust-api-inventory/1.0".to_string(),
        adjudicated_at: Some("2026-01-01".to_string()),
        symbols: symbols
            .into_iter()
            .map(|(name, classification, non_exhaustive)| Symbol {
                name: name.to_string(),
                kind: "struct".to_string(),
                non_exhaustive,
                root_reexported: true,
                classification: classification.to_string(),
                reason: "fixture".to_string(),
            })
            .collect(),
    }
}

/// 负例一（**本门的招牌场景**）：把冻结符号的分类**放宽**成 `UNSTABLE` → 必须报问题。
///
/// 这就是文件头承诺要拦的那件事："Editing `docs/api-inventory.json` to loosen a
/// classification ... makes this test fail"。
#[test]
fn negative_loosened_classification_is_reported() {
    let mut symbols: Vec<(&str, &str, bool)> = frozen_stable_names()
        .into_iter()
        .map(|n| (n, "STABLE", false))
        .collect();
    symbols.push(("ChatRequest", "STABLE-ADDITIVE", true));
    symbols.push(("EmbeddingRequest", "STABLE-ADDITIVE", true));

    // 合规基线：不得报任何问题。
    let clean = inv_from(symbols.clone());
    assert!(
        expectation_problems(&clean).is_empty(),
        "the fixture baseline must be clean: {:?}",
        expectation_problems(&clean)
    );

    // 放宽一个（ChatResponse: STABLE → UNSTABLE）→ 必须被点名。
    let loosened: Vec<(&str, &str, bool)> = symbols
        .iter()
        .map(|(n, c, ne)| {
            if *n == "ChatResponse" {
                (*n, "UNSTABLE", *ne)
            } else {
                (*n, *c, *ne)
            }
        })
        .collect();
    let problems = expectation_problems(&inv_from(loosened));
    assert_eq!(problems.len(), 1, "got {problems:?}");
    assert!(problems[0].contains("ChatResponse"), "{problems:?}");
    assert!(problems[0].contains("freezes it as STABLE"), "{problems:?}");
}

/// 负例二：**删掉一个冻结符号**（从清单里"蒸发"）→ 必须报问题。
#[test]
fn negative_missing_frozen_symbol_is_reported() {
    let mut symbols: Vec<(&str, &str, bool)> = frozen_stable_names()
        .into_iter()
        .filter(|n| *n != "FinishReason")
        .map(|n| (n, "STABLE", false))
        .collect();
    symbols.push(("ChatRequest", "STABLE-ADDITIVE", true));
    symbols.push(("EmbeddingRequest", "STABLE-ADDITIVE", true));
    let problems = expectation_problems(&inv_from(symbols));
    assert_eq!(problems.len(), 1, "got {problems:?}");
    assert!(problems[0].contains("FinishReason"), "{problems:?}");
    assert!(problems[0].contains("missing"), "{problems:?}");
}

/// 负例三：`STABLE-ADDITIVE` 失去 `#[non_exhaustive]` → 必须报问题（承诺变谎话）。
#[test]
fn negative_stable_additive_without_non_exhaustive_is_reported() {
    let mut symbols: Vec<(&str, &str, bool)> = frozen_stable_names()
        .into_iter()
        .map(|n| (n, "STABLE", false))
        .collect();
    symbols.push(("ChatRequest", "STABLE-ADDITIVE", false)); // ← 缺 non_exhaustive
    symbols.push(("EmbeddingRequest", "STABLE-ADDITIVE", true));
    let problems = expectation_problems(&inv_from(symbols));
    assert_eq!(problems.len(), 1, "got {problems:?}");
    assert!(problems[0].contains("non_exhaustive"), "{problems:?}");
}

/// 负例四：给冻结符号加 `#[non_exhaustive]` → 必须报问题（那本身就是破坏性变更）。
#[test]
fn negative_non_exhaustive_on_frozen_symbol_is_reported() {
    let mut symbols: Vec<(&str, &str, bool)> = frozen_stable_names()
        .into_iter()
        .map(|n| {
            if n == "Usage" {
                (n, "STABLE", true) // ← 非法
            } else {
                (n, "STABLE", false)
            }
        })
        .collect();
    symbols.push(("ChatRequest", "STABLE-ADDITIVE", true));
    symbols.push(("EmbeddingRequest", "STABLE-ADDITIVE", true));
    let problems = expectation_problems(&inv_from(symbols));
    assert_eq!(problems.len(), 1, "got {problems:?}");
    assert!(problems[0].contains("Usage"), "{problems:?}");
}

/// 真清单必须通过本纯函数（保证抽取出的比较与线上判定**同一口径**，不是摆设）。
#[test]
fn real_inventory_passes_the_extracted_comparison() {
    let problems = expectation_problems(&load_inventory());
    assert!(
        problems.is_empty(),
        "docs/api-inventory.json violates the frozen expectations:\n{}",
        problems.join("\n")
    );
}
