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
