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
