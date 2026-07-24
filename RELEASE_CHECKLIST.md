# Release Checklist

## v0.1.1

Before publishing:

- [ ] `git checkout main && git pull origin main`
- [ ] `cargo fmt --check`
- [ ] `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] `cargo test`
- [ ] `cargo test --all-features`
- [ ] `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features`
- [ ] `cargo publish --dry-run`
- [ ] `cargo package --list`
- [ ] Confirm CI is green on `main`
- [ ] Confirm `Cargo.toml` version is `0.1.1`
- [ ] Confirm `rust-version` is `1.86`
- [ ] Confirm `CHANGELOG.md` has `## [0.1.1] - 2026-06-16`
- [ ] Create tag: `git tag -a v0.1.1 -m "Release v0.1.1"`
- [ ] Push tag: `git push origin v0.1.1`
- [ ] Publish crate: `cargo publish`
- [ ] Create GitHub Release from `v0.1.1`

## v0.1.0

Before publishing:

- [ ] `git checkout main && git pull origin main`
- [ ] `cargo fmt --check`
- [ ] `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] `cargo test`
- [ ] `cargo test --all-features`
- [ ] `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features`
- [ ] `cargo publish --dry-run`
- [ ] `cargo package --list`
- [ ] Confirm CI is green on `main`
- [ ] Confirm `Cargo.toml` version is `0.1.0`
- [ ] Confirm `rust-version` is `1.86`
- [ ] Confirm `CHANGELOG.md` has `## [0.1.0] - 2026-06-11`
- [ ] Create tag: `git tag -a v0.1.0 -m "Release v0.1.0"`
- [ ] Push tag: `git push origin v0.1.0`
- [ ] Publish crate: `cargo publish`
- [ ] Create GitHub Release from `v0.1.0`

---

## Protected tag-only release pipeline (REL-001, Issue #97)

This section documents the **gated** release path. As of REL-001, a real
publish is no longer a bare `cargo publish`; it is a protected, tag-only,
dry-run-only pipeline. Manual `cargo publish` (with `--token` /
`--allow-dirty`) is **forbidden** and would bypass every gate below. The
historical `cargo publish` steps in the v0.1.0 / v0.1.1 sections above are
superseded by this pipeline.

### Preconditions (all required)
- [ ] A semver tag `vX.Y.Z` exists and is pushed.
- [ ] `Cargo.toml` version, `llmrust.capabilities.json` `version`, and a
      `CHANGELOG.md` `## [X.Y.Z]` section all agree (enforced by
      `.github/scripts/release-validate.sh`).
- [ ] Working tree is clean (no uncommitted changes).
- [ ] Release freeze lifted by Owner (SPCC §3.3) — REL-001 does NOT lift it.

### Pipeline gates (run automatically on tag push)
- [ ] `validate` job: pre-flight script (tag / version / clean / default-feature
      no-proxy-dep) passes.
- [ ] `validate` job: CI-003 guards `architecture_guard` + `package_guard` pass.
- [ ] `validate` job: gitleaks primary scan (working tree) passes.
- [ ] `dry-run` job (behind `release` environment approval): `cargo publish --dry-run`
      succeeds — **zero network upload**.
- [ ] `dry-run` job: secondary gitleaks scan on the extracted `.crate` passes;
      sha256 + provenance sample emitted.
- [ ] `guards-fmt-clippy` job: `cargo fmt --check` + `cargo clippy -D warnings`
      pass (re-run after any auto-fix — CI-003 MUST-1 lesson).

### Actual publish (manual, Owner-approved only)
- [ ] Owner approves and runs the real `cargo publish` from a clean, tagged,
      verified local checkout — outside CI, with explicit sign-off.
- [ ] Confirm `https://crates.io/crates/llmrust` shows the new version.

> The four negative cases (non-tag trigger, version mismatch, dirty tree,
> forbidden-file-in-package) are proven to fail via local injection drills;
> see the REL-001 implementation PR for the FAILED-then-reverted evidence.
