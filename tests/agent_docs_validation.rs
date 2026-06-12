//! Validate agent-facing docs and capability metadata.
//!
//! Replaces `scripts/validate_agent_docs.py` with a Rust integration test
//! so the llmrust repository stays Rust-native. Uses only `std` and
//! `serde_json` — no extra dependencies.
//!
//! Covered checks:
//! 1. llmrust.capabilities.json structure, providers, auth config, n_policy
//! 2. examples/README.md lists only real, registered examples, proxy_server
//!    includes --features proxy
//! 3. AGENTS.md / docs/CONTRACTS.md free of known incorrect phrases
//! 4. Capability disclaimer present in docs

use std::collections::HashSet;
use std::fs;
use std::path::Path;

/// Return the repository root (parent of the `tests/` directory).
fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

// ── helpers for extracting example names from markdown / TOML ──────

/// Extract example names from a markdown table.
/// Table rows look like: `| \`demo\` | ... |`
/// We only take the **first** backtick-enclosed word per row,
/// which is the example name column.
fn extract_example_names_from_md(text: &str) -> HashSet<String> {
    let mut names = HashSet::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('|') {
            continue;
        }
        // Find first `\`...\`` pattern — this is the example name column
        if let Some(start) = trimmed.find('`') {
            let after_first = &trimmed[start + 1..];
            if let Some(end) = after_first.find('`') {
                let name = &after_first[..end];
                if name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    && !name.is_empty()
                    && name.to_lowercase() != "example"
                {
                    names.insert(name.to_string());
                }
            }
        }
    }
    names
}

/// Extract example names from Cargo.toml by finding `[[example]]` blocks
/// and reading the `name` field. No `toml` crate needed.
fn extract_example_names_from_cargo_toml(text: &str) -> HashSet<String> {
    let mut names = HashSet::new();
    let mut in_example_block = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == "[[example]]" {
            in_example_block = true;
            continue;
        }
        if in_example_block && trimmed.starts_with('[') {
            in_example_block = false;
            continue;
        }
        if in_example_block {
            if let Some(rest) = trimmed.strip_prefix("name") {
                if let Some(val) = rest.split_once('"').and_then(|(_, v)| v.split_once('"')) {
                    names.insert(val.0.to_string());
                }
            }
        }
    }
    names
}

/// List all `.rs` files in the examples/ directory (stem only).
fn example_rs_files() -> HashSet<String> {
    let dir = repo_root().join("examples");
    let mut files = HashSet::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "rs") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    files.insert(stem.to_string());
                }
            }
        }
    }
    files
}

// ── test 1: capabilities.json structure ───────────────────────────

#[test]
fn capabilities_json_exists_and_is_valid() {
    let path = repo_root().join("llmrust.capabilities.json");
    let text = fs::read_to_string(&path).expect("llmrust.capabilities.json not found");
    let _cap: serde_json::Value =
        serde_json::from_str(&text).expect("llmrust.capabilities.json is not valid JSON");
}

#[test]
fn capabilities_json_has_name_and_version() {
    let text = fs::read_to_string(repo_root().join("llmrust.capabilities.json")).unwrap();
    let cap: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(cap["name"], "llmrust", "name must be 'llmrust'");
    assert!(
        !cap["version"].as_str().unwrap_or("").is_empty(),
        "version is missing"
    );
}

#[test]
fn capabilities_json_providers_complete() {
    let text = fs::read_to_string(repo_root().join("llmrust.capabilities.json")).unwrap();
    let cap: serde_json::Value = serde_json::from_str(&text).unwrap();
    let providers = cap["providers"]
        .as_object()
        .expect("providers must be a JSON object");

    let required: HashSet<&str> = [
        "openai",
        "deepseek",
        "moonshot",
        "openrouter",
        "anthropic",
        "google",
        "ollama",
    ]
    .into_iter()
    .collect();

    let present: HashSet<&str> = providers.keys().map(|k| k.as_str()).collect();

    let missing: Vec<_> = required.difference(&present).copied().collect();
    assert!(missing.is_empty(), "missing provider entries: {missing:?}");

    let extra: Vec<_> = present.difference(&required).copied().collect();
    assert!(extra.is_empty(), "unexpected provider entries: {extra:?}");
}

#[test]
fn capabilities_json_proxy_auth_config() {
    let text = fs::read_to_string(repo_root().join("llmrust.capabilities.json")).unwrap();
    let cap: serde_json::Value = serde_json::from_str(&text).unwrap();
    let auth = &cap["proxy"]["auth"];
    assert_eq!(
        auth["config"].as_str().unwrap_or(""),
        "LLMRUST_PROXY_KEY",
        "proxy.auth.config must be 'LLMRUST_PROXY_KEY'"
    );
}

#[test]
fn capabilities_json_n_policy() {
    let text = fs::read_to_string(repo_root().join("llmrust.capabilities.json")).unwrap();
    let cap: serde_json::Value = serde_json::from_str(&text).unwrap();
    let n_policy = cap["proxy"]["n_policy"]
        .as_str()
        .expect("proxy.n_policy must be a string")
        .to_lowercase();

    let has_accepts = n_policy.contains("accepts") || n_policy.contains("accept");
    let has_n1 = n_policy.contains("n = 1") || n_policy.contains("n=1");
    let has_reject = n_policy.contains("n = 0")
        || n_policy.contains("n=0")
        || n_policy.contains("n > 1")
        || n_policy.contains("n>1");

    assert!(
        has_accepts,
        "proxy.n_policy must mention accepting missing n or n=1, got: {n_policy:?}"
    );
    assert!(has_n1, "proxy.n_policy must mention n=1, got: {n_policy:?}");
    assert!(
        has_reject,
        "proxy.n_policy must mention rejecting n=0 or n>1, got: {n_policy:?}"
    );
}

#[test]
fn capabilities_json_description_has_disclaimer() {
    let text = fs::read_to_string(repo_root().join("llmrust.capabilities.json")).unwrap();
    let cap: serde_json::Value = serde_json::from_str(&text).unwrap();
    let desc = cap["description"].as_str().unwrap_or("").to_lowercase();
    assert!(
        desc.contains("actual upstream model support may vary"),
        "capabilities.json description missing disclaimer"
    );
}

// ── test 2: examples/README.md ────────────────────────────────────

#[test]
fn examples_readme_lists_only_registered_examples() {
    let readme_text = fs::read_to_string(repo_root().join("examples").join("README.md"))
        .expect("examples/README.md not found");
    let cargo_toml =
        fs::read_to_string(repo_root().join("Cargo.toml")).expect("Cargo.toml not found");

    let listed = extract_example_names_from_md(&readme_text);
    let cargo_examples = extract_example_names_from_cargo_toml(&cargo_toml);
    let rs_files = example_rs_files();

    assert!(
        !listed.is_empty(),
        "no examples found in examples/README.md table"
    );

    for ex in &listed {
        assert!(
            cargo_examples.contains(ex),
            "'{ex}' listed in examples/README.md but not in Cargo.toml [[example]]"
        );
        assert!(
            rs_files.contains(ex),
            "'{ex}' listed in examples/README.md but examples/{ex}.rs does not exist"
        );
    }
}

#[test]
fn proxy_server_command_includes_features_proxy() {
    let readme_text = fs::read_to_string(repo_root().join("examples").join("README.md"))
        .expect("examples/README.md not found");

    // Check if the README has proxy_server near --features proxy
    let has_proxy_cmd = readme_text.contains("proxy_server");
    if has_proxy_cmd {
        let features_near =
            readme_text.contains("--features proxy") && readme_text.contains("proxy_server");
        assert!(
            features_near,
            "proxy_server in examples/README.md must include --features proxy in its command"
        );
    }
}

// ── test 3: dangerous phrase scan ─────────────────────────────────

const DANGEROUS_PHRASES: &[(&str, &str)] = &[
    (
        "subtle crate",
        "refers to nonexistent subtle crate dependency",
    ),
    (
        "Reject n != 1 (or absent)",
        "incorrect n policy (missing n is accepted)",
    ),
    (
        "reject n != 1 (or absent)",
        "incorrect n policy (missing n is accepted)",
    ),
    (
        "all 8 provider implementations",
        "hardcodes provider count (should not hardcode)",
    ),
];

const AGENT_FILES: &[&str] = &[
    "AGENTS.md",
    "docs/CONTRACTS.md",
    "docs/CAPABILITIES.md",
    "examples/README.md",
    "llmrust.capabilities.json",
];

/// Per-file check: no dangerous phrase appears.
#[test]
fn agent_docs_free_of_dangerous_phrases() {
    for fname in AGENT_FILES {
        let path = repo_root().join(fname);
        if !path.exists() {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap_or_default();
        let lower = text.to_lowercase();

        for (phrase, explanation) in DANGEROUS_PHRASES {
            assert!(
                !lower.contains(&phrase.to_lowercase()),
                "{fname}: contains '{phrase}' — {explanation}"
            );
        }

        // Context-sensitive: proxy_server without --features proxy
        if lower.contains("cargo run --example proxy_server") {
            // The file mentions this specific command; check --features proxy nearby
            if !lower.contains("--features proxy") {
                // May still be OK if the command template is generic (cargo run --example <name>)
                // Only fail if the literal string appears without --features proxy
                let cmd_idx = lower.find("cargo run --example proxy_server").unwrap();
                let window = &lower[cmd_idx..];
                if !window.contains("--features proxy") {
                    panic!("{fname}: proxy_server command without --features proxy");
                }
            }
        }
    }
}

// ── test 4: capability disclaimer ─────────────────────────────────

#[test]
fn capabilities_md_has_disclaimer() {
    let text = fs::read_to_string(repo_root().join("docs").join("CAPABILITIES.md"))
        .expect("docs/CAPABILITIES.md not found");

    assert!(
        text.contains("Actual upstream model support may vary"),
        "docs/CAPABILITIES.md missing disclaimer: 'Actual upstream model support may vary'"
    );
}
