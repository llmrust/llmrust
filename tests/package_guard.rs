//! Package allowlist guard for CI-003 (SPCC §9.2).
//!
//! Parses `cargo package --list` and fails if the candidate release artifact
//! would contain any forbidden file (secrets, logs, local/vcs debris). This is
//! the machine counterpart to the release allowlist; a `publish.log` sneaked
//! into the crate must make CI fail.
//!
//! Run locally with: `cargo test --test package_guard`

use std::process::Command;

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
    let mut violations = Vec::new();

    for line in listing.lines() {
        let name = line.trim();
        if name.is_empty() {
            continue;
        }
        let forbidden = name.ends_with(".log")
            || name.starts_with(".env")
            || name.contains("/.env")
            || name.ends_with(".env")
            || name == ".git"
            || name.starts_with(".git/")
            || name.contains("target/")
            || name.ends_with(".secret");
        if forbidden {
            violations.push(name.to_string());
        }
    }

    assert!(
        violations.is_empty(),
        "Forbidden files present in `cargo package` artifact (SPCC §9.2):\n{}",
        violations.join("\n")
    );
}
