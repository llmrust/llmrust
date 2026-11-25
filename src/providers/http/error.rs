//! One sanitization boundary for reqwest errors.

use crate::providers::LlmError;

/// Remove the URL from both the reqwest display and its source chain.
pub(crate) fn without_url(error: reqwest::Error) -> reqwest::Error {
    error.without_url()
}

/// Construct the public HTTP error variant without leaking request locations.
pub(crate) fn http_error(error: reqwest::Error) -> LlmError {
    LlmError::Http(without_url(error))
}

/// Format a reqwest error for a parse/stream message without its URL.
pub(crate) fn error_message(error: reqwest::Error) -> String {
    without_url(error).to_string()
}
