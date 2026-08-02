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

Authentication uses a reviewed constant-time comparison (`subtle::ConstantTimeEq`) — never a hand-rolled XOR loop. An empty or whitespace-only `LLMRUST_PROXY_KEY` refuses to start (SPCC §7.1): `router_with_auth` panics at construction and `serve()` returns an error; an unset key keeps the unauthenticated loopback-only mode. `GET /health` requires no authentication and returns only `{"status":"ok"}` (SPCC §7.2); it never reaches upstream providers.

The proxy sends **no CORS allow-origin header by default** (SPCC §7.1): browser cross-origin access is only enabled by explicitly wrapping the `Router` with a restrictive `CorsLayer`. `Access-Control-Allow-Origin: *` is only permitted on the authenticated router with explicit Owner risk acceptance — never the default. Without `LLMRUST_PROXY_KEY` set, the proxy binds only to loopback addresses (`127.0.0.1` / `::1` / `localhost`); serving on a non-loopback address requires authentication.

## Proxy Deployment

Production deployments should use a reverse proxy with TLS (e.g. Caddy / nginx / a cloud load balancer) in front of llmrust — llmrust does **not** terminate TLS itself (SPCC §7.1). Recommended settings:

- **Bind address**: keep the default loopback bind (`127.0.0.1:3000`) behind the reverse proxy; never expose an unauthenticated proxy on a public address. A non-loopback bind requires `LLMRUST_PROXY_KEY`.
- **Authentication**: set `LLMRUST_PROXY_KEY` (a strong random secret). Every request must then send `Authorization: Bearer <key>`; comparison is constant-time via `subtle`.
- **CORS**: enable browser access only via an explicit `CorsLayer` allowlist; `Access-Control-Allow-Origin: *` requires the authenticated router and explicit risk acceptance.
- **Request body limit** (PRX-005): the proxy rejects bodies over 2 MiB by default with a protocol-shaped 413. To raise it (e.g. for vision / large base64 image payloads), set `LLMRUST_PROXY_MAX_BODY_BYTES` to the desired byte count — and keep the reverse proxy's client-max-body-size in sync. **Vision/base64 image loads routinely exceed 2 MiB; raise the limit explicitly in such deployments.**
- **Rate limiting**: the proxy does not implement rate limiting; terminate one at the reverse proxy / load balancer.

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
