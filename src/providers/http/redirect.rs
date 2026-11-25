//! Provider endpoint construction and base-URL validation.
//!
//! Constructors retain their infallible public signatures, so validation is
//! performed immediately before every network request. Rejected URLs are never
//! included in the error text: configuration values may contain credentials.

use reqwest::Url;

use crate::providers::{LlmError, Result};

/// Validate a provider base URL and remove only trailing path slashes.
///
/// Userinfo, query strings, and fragments are rejected because appending a
/// provider endpoint to them is ambiguous and can leak credentials or alter the
/// request target. Interior path segments are left untouched.
pub(crate) fn validate_base_url(raw: &str) -> Result<String> {
    let url = Url::parse(raw)
        .map_err(|_| LlmError::Parse("provider base URL is not a valid absolute URL".into()))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(LlmError::Parse(
            "provider base URL must be an absolute http(s) URL with a host".into(),
        ));
    }
    let raw_authority = raw
        .trim()
        .split_once("://")
        .and_then(|(_, rest)| rest.split(['/', '?', '#']).next())
        .unwrap_or_default();
    if !url.username().is_empty() || url.password().is_some() || raw_authority.contains('@') {
        return Err(LlmError::Parse(
            "provider base URL must not contain userinfo".into(),
        ));
    }
    if url.query().is_some() {
        return Err(LlmError::Parse(
            "provider base URL must not contain a query string".into(),
        ));
    }
    if url.fragment().is_some() {
        return Err(LlmError::Parse(
            "provider base URL must not contain a fragment".into(),
        ));
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

/// Append a fixed provider endpoint to a validated base URL.
pub(crate) fn endpoint_url(base_url: &str, endpoint: &str) -> Result<String> {
    Ok(format!("{}/{}", validate_base_url(base_url)?, endpoint))
}

/// Build a Gemini endpoint with the model encoded as one URL path segment.
pub(crate) fn gemini_endpoint_url(base_url: &str, model: &str, streaming: bool) -> Result<Url> {
    let base_url = validate_base_url(base_url)?;
    let mut url = Url::parse(&base_url)
        .map_err(|_| LlmError::Parse("provider base URL is not a valid absolute URL".into()))?;
    let operation = if streaming {
        "streamGenerateContent"
    } else {
        "generateContent"
    };
    url.path_segments_mut()
        .map_err(|_| LlmError::Parse("provider base URL cannot carry path segments".into()))?
        .push("models")
        .push(&format!("{model}:{operation}"));
    if streaming {
        url.query_pairs_mut().append_pair("alt", "sse");
    }
    Ok(url)
}
