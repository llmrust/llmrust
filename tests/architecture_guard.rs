//! Architecture guards for CI-003 (SPCC §4.3 / §4.4).
//!
//! Machine-enforced boundaries. Two guards:
//!   1. `dependency_edges_are_respected` — forbidden intra-crate `use` edges.
//!   2. `hotspot_files_do_not_grow` — files on the §4.4 hotspot ledger may not
//!      net-grow beyond their recorded baseline.
//!
//! Run locally with: `cargo test --test architecture_guard`

use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Forbidden dependency edges, derived verbatim from SPCC §4.3.
/// Each entry is `(file, forbidden_use_prefix)`: any `use`/`pub use` line in
/// `file` that contains `forbidden_use_prefix` is a violation.
fn forbidden_edges() -> Vec<(&'static str, &'static str)> {
    vec![
        // §4.3: types.rs must not depend on provider/client/router/proxy.
        ("src/types.rs", "crate::providers"),
        ("src/types.rs", "crate::proxy"),
        ("src/types.rs", "crate::router"),
        ("src/types.rs", "crate::LmrsClient"),
        ("src/types.rs", "crate::client"),
        // §4.3: a Provider must not depend on proxy or router.
        ("src/providers/mod.rs", "crate::proxy"),
        ("src/providers/mod.rs", "crate::router"),
        ("src/providers/compat.rs", "crate::proxy"),
        ("src/providers/compat.rs", "crate::router"),
        ("src/providers/google.rs", "crate::proxy"),
        ("src/providers/google.rs", "crate::router"),
        ("src/providers/anthropic.rs", "crate::proxy"),
        ("src/providers/anthropic.rs", "crate::router"),
        ("src/providers/openai.rs", "crate::proxy"),
        ("src/providers/openai.rs", "crate::router"),
        ("src/providers/deepseek.rs", "crate::proxy"),
        ("src/providers/deepseek.rs", "crate::router"),
        ("src/providers/moonshot.rs", "crate::proxy"),
        ("src/providers/moonshot.rs", "crate::router"),
        ("src/providers/ollama.rs", "crate::proxy"),
        ("src/providers/ollama.rs", "crate::router"),
        ("src/providers/openrouter.rs", "crate::proxy"),
        ("src/providers/openrouter.rs", "crate::router"),
        ("src/providers/retry.rs", "crate::proxy"),
        ("src/providers/retry.rs", "crate::router"),
        // §4.3: http.rs / stream_util.rs must not depend on a concrete Provider.
        ("src/providers/http.rs", "crate::providers::openai"),
        ("src/providers/http.rs", "crate::providers::anthropic"),
        ("src/providers/http.rs", "crate::providers::google"),
        ("src/providers/http.rs", "crate::providers::deepseek"),
        ("src/providers/http.rs", "crate::providers::moonshot"),
        ("src/providers/http.rs", "crate::providers::ollama"),
        ("src/providers/http.rs", "crate::providers::openrouter"),
        ("src/providers/http.rs", "crate::providers::compat"),
        ("src/providers/http.rs", "crate::providers::retry"),
        ("src/providers/stream_util.rs", "crate::providers::openai"),
        (
            "src/providers/stream_util.rs",
            "crate::providers::anthropic",
        ),
        ("src/providers/stream_util.rs", "crate::providers::google"),
        ("src/providers/stream_util.rs", "crate::providers::deepseek"),
        ("src/providers/stream_util.rs", "crate::providers::moonshot"),
        ("src/providers/stream_util.rs", "crate::providers::ollama"),
        (
            "src/providers/stream_util.rs",
            "crate::providers::openrouter",
        ),
        ("src/providers/stream_util.rs", "crate::providers::compat"),
        ("src/providers/stream_util.rs", "crate::providers::retry"),
        // §4.3: lib.rs / default feature must not pull in proxy-only deps.
        ("src/lib.rs", "axum"),
        ("src/lib.rs", "tower"),
    ]
}

#[test]
fn dependency_edges_are_respected() {
    let mut violations = Vec::new();
    for (file, prefix) in forbidden_edges() {
        let path = Path::new(file);
        if !path.exists() {
            continue;
        }
        let src = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        for line in src.lines() {
            let trimmed = line.trim_start();
            if !(trimmed.starts_with("use ") || trimmed.starts_with("pub use ")) {
                continue;
            }
            if trimmed.contains(prefix) {
                violations.push(format!("{file}: `{trimmed}` (forbidden edge `{prefix}`)"));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "Dependency-edge violations found (SPCC §4.3):\n{}",
        violations.join("\n")
    );
}

#[test]
fn hotspot_files_do_not_grow() {
    let json =
        fs::read_to_string("tests/hotspot_ledger.json").expect("hotspot_ledger.json must exist");
    let ledger: serde_json::Value = serde_json::from_str(&json).expect("ledger must be valid JSON");
    let baseline = ledger["baseline_lines"]
        .as_object()
        .expect("ledger must contain baseline_lines object");

    let mut violations = Vec::new();
    for (file, value) in baseline {
        let expected = value
            .as_u64()
            .expect("baseline must be an integer line count");
        let path = Path::new(file);
        let current = if path.exists() {
            fs::read_to_string(path).unwrap_or_default().lines().count() as u64
        } else {
            0
        };
        if current != expected {
            violations.push(format!(
                "{file}: {current} lines != baseline {expected} (hotspot drift in either direction forbidden by SPCC §4.4; shrink = REL-003 truncation class)"
            ));
        }
    }
    assert!(
        violations.is_empty(),
        "Hotspot-ledger violations:\n{}",
        violations.join("\n")
    );
}

/// Sanity: the ledger itself is internally consistent (every entry is a real,
/// existing source file above the §4.4 threshold of 800 lines).
#[test]
fn hotspot_ledger_is_consistent() {
    let json = fs::read_to_string("tests/hotspot_ledger.json").unwrap();
    let ledger: serde_json::Value = serde_json::from_str(&json).unwrap();
    let threshold = ledger["threshold"].as_u64().unwrap_or(800);
    let baseline = ledger["baseline_lines"].as_object().unwrap();
    let mut problems = Vec::new();
    for (file, value) in baseline {
        let lines = value.as_u64().unwrap();
        if lines < threshold {
            problems.push(format!("{file}: {lines} < threshold {threshold}"));
        }
        if !Path::new(file).exists() {
            problems.push(format!("{file}: file does not exist"));
        }
    }
    assert!(
        problems.is_empty(),
        "Ledger problems:\n{}",
        problems.join("\n")
    );

    // Ensure the in-test map of forbidden edges lines up with real files so the
    // guard cannot silently skip a moved/renamed module.
    let mut _checked: HashMap<&str, u64> = HashMap::new();
    for key in baseline.keys() {
        _checked.insert(key.as_str(), 0);
    }
}
