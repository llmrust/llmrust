# Security Policy

## Supported Versions

Security fixes are provided for the latest published version of `llmrust`.

## Reporting a Vulnerability

Please report security issues privately instead of opening a public issue.

Contact: tianxiahs@foxmail.com

Please include:

- affected version or commit
- minimal reproduction steps
- impact assessment
- whether credentials, prompts, responses, or proxy endpoints may be exposed

## Proxy Security

The `proxy` feature can expose OpenAI-compatible and Anthropic-compatible HTTP endpoints.

Do not expose an unauthenticated llmrust proxy to the public internet.

Use one of the following:

- `LLMRUST_PROXY_KEY`
- `router_with_auth`
- a reverse proxy with TLS, authentication, and rate limiting

The default development router is intended for local use. Production deployments should restrict CORS, require authentication, and avoid logging prompts, responses, request bodies, API keys, image data, tool arguments, or full URLs.

## Logging

llmrust tracing events are designed to avoid API keys, prompt content, response text, request bodies, tool arguments, image data, and full URLs. Logs should use counts and lengths where useful.

## Publishing security (REL-001)

The release path is a **protected, tag-only, dry-run-only** pipeline
(implemented in `.github/workflows/release.yml`, specified by Issue #97).

- **Tag-only trigger.** The pipeline runs solely on a `vX.Y.Z` tag push.
  There is no `workflow_dispatch` manual entry — no emergency bypass.
- **No upload, ever, in CI.** Only `cargo publish --dry-run` is performed
  (no `--token`, no `--allow-dirty`). The artifact is never uploaded from CI.
- **Manual environment approval.** The `dry-run` job is gated behind the
  `release` environment (required reviewers configured in repo settings).
- **Secret scanning, twice.** gitleaks scans the working tree and again the
  extracted `.crate` bytes (the exact payload that would be published).
- **Version consistency.** A pre-flight script (`.github/scripts/release-validate.sh`)
  refuses any release where the tag, `Cargo.toml`, `llmrust.capabilities.json`,
  and `CHANGELOG.md` versions disagree, or where the tree is dirty.
- **Publish freeze is not lifted by this pipeline.** Even after REL-001 is
  `DONE`, an actual `cargo publish` to crates.io requires the Owner's
  explicit approval (SPCC §3.3). The 0.1.2 accident (dirty, tag-less,
  version-drifted publish) is structurally prevented by these gates.
