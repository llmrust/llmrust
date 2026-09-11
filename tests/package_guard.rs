//! Package allowlist guard for CI-003 (SPCC §9.2).
//!
//! Parses `cargo package --list` and fails if the candidate release artifact
//! would contain any forbidden file (secrets, logs, local/vcs debris). This is
//! the machine counterpart to the release allowlist; a `publish.log` sneaked
//! into the crate must make CI fail.
//!
//! The forbidden-entry predicate is a **pure function** ([`forbidden_entry`]) so a
//! negative test can drive it directly without shelling out (`GRD-003` item ②:
//! every guard needs a negative test that fails when the guard is weakened).
//!
//! Run locally with: `cargo test --test package_guard`

use std::process::Command;

/// Is this entry from `cargo package --list` forbidden in the artifact?
///
/// Pure, so it can be exercised by synthetic listings in tests.
pub fn forbidden_entry(name: &str) -> bool {
    name.ends_with(".log")
        || name.starts_with(".env")
        || name.contains("/.env")
        || name.ends_with(".env")
        || name == ".git"
        || name.starts_with(".git/")
        || name.contains("target/")
        || name.ends_with(".secret")
}

/// All forbidden entries in a `cargo package --list` output (pure).
pub fn violations_in(listing: &str) -> Vec<String> {
    listing
        .lines()
        .map(str::trim)
        .filter(|n| !n.is_empty() && forbidden_entry(n))
        .map(str::to_string)
        .collect()
}

#[test]
fn package_list_contains_only_allowed_files() {
    let output = Command::new("cargo")
        .args(["package", "--list", "--allow-dirty"])
        .output()
        .expect("failed to run `cargo package --list`");

    // N-4 (architecture audit 2026-08-04): the guard must be fail-closed — if
    // `cargo package --list` itself fails, the empty stdout must not look like
    // "zero forbidden files". Assert the exit status and surface stderr before
    // trusting the listing.
    assert!(
        output.status.success(),
        "`cargo package --list` failed (exit {:?}); stderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let listing = String::from_utf8_lossy(&output.stdout);
    let violations = violations_in(&listing);

    assert!(
        violations.is_empty(),
        "Forbidden files present in `cargo package` artifact (SPCC §9.2):\n{}",
        violations.join("\n")
    );
}

// ── 负例（GRD-003 ②）：门被削弱时必须能红 ───────────────────────────────

/// 负例：`publish.log` 混进产物 → **必须**被判违规。
///
/// 这正是本门文件头注释里承诺要拦的那件事；没有这条，门就是装饰。
#[test]
fn negative_forbidden_entries_are_reported() {
    let listing = "\
Cargo.toml
src/lib.rs
publish.log
.env
.env.local
.git/config
target/debug/foo
credentials.secret
";
    let violations = violations_in(listing);
    assert!(
        violations.contains(&"publish.log".to_string()),
        "publish.log must be flagged; got {violations:?}"
    );
    assert!(
        violations.contains(&".env".to_string()),
        "got {violations:?}"
    );
    assert!(
        violations.contains(&".env.local".to_string()),
        "got {violations:?}"
    );
    assert!(
        violations.contains(&".git/config".to_string()),
        "got {violations:?}"
    );
    assert!(
        violations.contains(&"target/debug/foo".to_string()),
        "got {violations:?}"
    );
    assert!(
        violations.contains(&"credentials.secret".to_string()),
        "got {violations:?}"
    );
    assert_eq!(
        violations.len(),
        6,
        "exactly the six forbidden entries; got {violations:?}"
    );
}

/// 正向对照：干净清单**不得**产生违规（否则是"无人能过"的假门）。
#[test]
fn clean_listing_yields_no_violations() {
    let listing = "\
Cargo.toml
Cargo.lock
README.md
src/lib.rs
src/providers/anthropic.rs
tests/package_guard.rs
";
    assert!(violations_in(listing).is_empty(), "clean listing must pass");
}

/// 边界：允许清单里合法的相似名不得误伤（`logging.md` ≠ `*.log`；`.envoy` 不算 `.env`）。
#[test]
fn lookalike_entries_are_not_flagged() {
    assert!(!forbidden_entry("docs/logging.md"));
    assert!(!forbidden_entry("src/env.rs"));
    assert!(!forbidden_entry("docs/git.md"));
    assert!(!forbidden_entry("Cargo.toml"));
}
