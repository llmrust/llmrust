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
