# Contributing to llmrust

Thank you for contributing — whether you're human, an AI coding agent, or a human-agent team.

## Quick start

```bash
git clone https://github.com/llmrust/llmrust.git
cd llmrust

# Build everything
cargo build --all-targets --all-features

# Run all tests
cargo test
cargo test --all-features

# Lint
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check

# Docs
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

## For AI coding agents

If you are an AI agent operating on behalf of a human user, please read `AGENTS.md` first. It contains the core rules and common pitfalls that will save you (and the maintainer) time.

When submitting a PR, check the "AI Agent Contribution" box in the PR template and fill in your agent/model name and human operator.

## Pull request process

1. Fork the repository and create a feature branch.
2. Make your changes. Keep commits focused — one logical change per commit.
3. Run the full CI suite locally (see commands above).
4. Open a PR against `main`. Fill in the PR template completely.
5. A maintainer will review. Expect feedback focused on correctness, clarity, and test coverage.

## What makes a good contribution

- **Tests**: Every behavior change needs a test. Bug fixes need a regression test that fails before the fix.
- **Small diffs**: Prefer small, focused PRs over large ones. They're reviewed faster.
- **Clear intent**: The PR description should explain *why*, not just *what*.
- **No surprises**: If you're adding a dependency, explain why it's necessary.

## Code conventions

- Follow `cargo fmt` (the project uses standard Rust formatting).
- Respect `cargo clippy` with `-D warnings` — no warnings allowed in CI.
- Use `thiserror` for error types (the `LlmError` enum).
- Use `tracing` for logging (`debug!`, `warn!`, `error!`). Never `println!` in library code.
- Provider implementations go in `src/providers/<name>.rs`.
- Proxy handlers go in `src/proxy/`.
- Shared HTTP client config is in `src/providers/http.rs`.

## Commit conventions

No strict format required, but we prefer descriptive, imperative-style messages:

```
fix: surface stream parse errors instead of silently dropping them
feat: add native tool calling for Anthropic provider
docs: clarify proxy model routing in README
test: add multi-turn conversation test for Gemini
refactor: extract common HTTP client builder
```

## License

By contributing, you agree that your contributions will be licensed under both MIT and Apache-2.0 (the same dual license as the project).

## Questions?

Open an issue on GitHub or start a discussion. We're happy to help.
