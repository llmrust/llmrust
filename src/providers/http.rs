//! Shared HTTP client construction for all providers.
//!
//! Every hosted provider (the OpenAI-compatible family, Anthropic, and Google)
//! needs a `reqwest::Client` tuned with the same connection pooling and
//! keepalive settings; only the overall request timeout differs. Hosted APIs
//! cap each request at [`REQUEST_TIMEOUT`] so a stalled connection can never
//! block a caller indefinitely, while local backends such as Ollama opt out of
//! the overall timeout because generation can legitimately run for a long time
//! on large models. Centralizing the builder here keeps the tuning identical
//! across providers and gives us a single place to evolve it.

use reqwest::Client;
use std::collections::HashMap;
use std::time::Duration;

/// Overall per-request timeout applied to hosted provider APIs.
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
/// TCP connect timeout applied to every provider so a stalled handshake fails fast.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// How long idle sockets are kept warm for connection reuse.
const TCP_KEEPALIVE: Duration = Duration::from_secs(30);
/// Maximum idle connections kept per host in the pool.
const POOL_MAX_IDLE_PER_HOST: usize = 32;

/// Build a `reqwest::Client` with llmrust's standard connection pooling and
/// keepalive settings.
///
/// `request_timeout` controls the overall per-request timeout: hosted APIs pass
/// `Some(REQUEST_TIMEOUT)`, while local backends such as Ollama pass `None`
/// because generation can legitimately run for a long time on large models. A
/// connect timeout is always enforced so a stalled handshake fails fast.
///
/// `custom_headers` are injected as **default headers** on the client. Per-
/// request `.header()` calls (e.g. `Authorization`) take precedence over these.
///
/// Falls back to a default client if the builder somehow fails (it will not in
/// practice with these static options).
pub(crate) fn build_http_client(
    request_timeout: Option<Duration>,
    custom_headers: Option<&HashMap<String, String>>,
) -> Client {
    let mut builder = Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .pool_max_idle_per_host(POOL_MAX_IDLE_PER_HOST)
        .tcp_keepalive(TCP_KEEPALIVE);
    if let Some(timeout) = request_timeout {
        builder = builder.timeout(timeout);
    }
    let mut default_headers = reqwest::header::HeaderMap::new();
    if let Some(headers) = custom_headers {
        for (k, v) in headers {
            let name = match reqwest::header::HeaderName::from_bytes(k.as_bytes()) {
                Ok(n) => n,
                Err(_) => {
                    tracing::warn!(header = %k, "skipping invalid custom header name");
                    continue;
                }
            };
            let val = match reqwest::header::HeaderValue::from_str(v) {
                Ok(v) => v,
                Err(_) => {
                    tracing::warn!(header = %k, "skipping custom header with invalid value");
                    continue;
                }
            };
            default_headers.insert(name, val);
        }
    }
    builder = builder.default_headers(default_headers);
    builder.build().unwrap_or_else(|_| Client::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_hosted_client_with_overall_timeout() {
        // Hosted-provider configuration carries an overall request timeout.
        let _ = build_http_client(Some(REQUEST_TIMEOUT), None);
    }

    #[test]
    fn builds_local_client_without_overall_timeout() {
        // Local-backend configuration (e.g. Ollama) opts out of the overall timeout.
        let _ = build_http_client(None, None);
    }
}
