//! Bounded, strict stream framing shared by every provider.
//!
//! Two protocols are covered:
//!
//! * **SSE** (OpenAI-compatible, Anthropic, Gemini): standard event framing —
//!   `data:` with or without the optional space, multi-line `data`, `\n` /
//!   `\r\n` / `\r` line endings, a leading BOM, comments, `event:` fields and
//!   dispatch on a blank line. For 0.1.x compatibility, a directly following
//!   `data:` line also starts a new frame when the current data is already a
//!   complete JSON value; incomplete JSON remains multi-line data. EOF flushes
//!   pending data. `event: error` surfaces as [`LlmError::Stream`].
//! * **NDJSON / JSON** (Ollama): complete lines, bounded and strictly decoded.
//!
//! A stream is only accepted when its `Content-Type` matches the protocol it
//! claims to speak. An HTTP 200 with an unexpected type is an explicit
//! [`LlmError::Parse`], never an empty success.

mod limits;
mod parser;

pub(crate) use parser::{event_line_stream, strict_line_stream};

use crate::providers::{LlmError, Result};
use reqwest::header::{HeaderMap, CONTENT_TYPE};

/// Require `text/event-stream` (an optional `;charset=...` is allowed).
pub(crate) fn require_event_stream(headers: &HeaderMap) -> Result<()> {
    require_media_type(headers, &["text/event-stream"], "text/event-stream")
}

/// Require `application/x-ndjson` or `application/json` (charset allowed).
pub(crate) fn require_ndjson_or_json(headers: &HeaderMap) -> Result<()> {
    require_media_type(
        headers,
        &["application/x-ndjson", "application/json"],
        "application/x-ndjson or application/json",
    )
}

fn require_media_type(headers: &HeaderMap, accepted: &[&str], expected: &str) -> Result<()> {
    let Some(value) = headers.get(CONTENT_TYPE) else {
        return Err(LlmError::Parse(format!(
            "stream response is missing Content-Type; expected {expected}"
        )));
    };
    let raw = value.to_str().map_err(|_| {
        LlmError::Parse(format!(
            "stream response Content-Type is not valid ASCII; expected {expected}"
        ))
    })?;
    let essence = raw
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if accepted.iter().any(|candidate| *candidate == essence) {
        Ok(())
    } else {
        Err(LlmError::Parse(format!(
            "unexpected stream Content-Type `{essence}`; expected {expected}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    async fn collect_events(
        chunks: Vec<&'static [u8]>,
    ) -> Vec<std::result::Result<parser::SseEvent, LlmError>> {
        let items: Vec<std::result::Result<&'static [u8], LlmError>> =
            chunks.into_iter().map(Ok).collect();
        parser::event_stream(futures::stream::iter(items))
            .collect()
            .await
    }

    #[tokio::test]
    async fn assembles_standard_variants() {
        let events = collect_events(vec![
            b"\xEF\xBB\xBF: keepalive\r\n",
            b"data:{\"a\":1}\r\n",
            // An intervening non-data field keeps this a standard multi-line
            // event rather than a single-newline compatibility boundary.
            b"id: multiline\r\n",
            b"data: 2\r\n\r\n",
        ])
        .await;
        assert_eq!(events.len(), 1, "one event dispatched on the blank line");
        let event = events[0].as_ref().unwrap();
        assert_eq!(event.data, "{\"a\":1}\n2", "multi-line data is joined");
    }

    #[tokio::test]
    async fn single_newline_complete_json_frames_and_eof_flush() {
        let events = collect_events(vec![b"data:{\"a\":1}\ndata:{\"b\":2}"]).await;
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].as_ref().unwrap().data, "{\"a\":1}");
        assert_eq!(events[1].as_ref().unwrap().data, "{\"b\":2}");
    }

    #[tokio::test]
    async fn incomplete_json_continues_multiline_data() {
        let events =
            collect_events(vec![b"data: {\"a\":\n", b"data: 1}\n", b"data: {\"b\":2}"]).await;
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].as_ref().unwrap().data, "{\"a\":\n1}");
        assert_eq!(events[1].as_ref().unwrap().data, "{\"b\":2}");
    }

    #[tokio::test]
    async fn standard_blank_line_dispatches_before_json_compatibility() {
        let events = collect_events(vec![b"data: {\"a\":\n\n"]).await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].as_ref().unwrap().data, "{\"a\":");
    }

    #[tokio::test]
    async fn cr_only_framing_and_crlf_split_across_chunks() {
        let events = collect_events(vec![b"data: a\r", b"\rdata: b\r\r"]).await;
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].as_ref().unwrap().data, "a");
        assert_eq!(events[1].as_ref().unwrap().data, "b");
    }

    #[tokio::test]
    async fn invalid_utf8_is_an_error() {
        let events = collect_events(vec![b"data: \xFF\n\n"]).await;
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], Err(LlmError::Parse(_))));
    }

    #[tokio::test]
    async fn oversized_line_is_bounded() {
        let mut chunk = Vec::with_capacity(limits::MAX_LINE_BYTES + 8);
        chunk.extend_from_slice(b"data: ");
        chunk.resize(limits::MAX_LINE_BYTES + 8, b'x');
        let events = collect_events(vec![Box::leak(chunk.into_boxed_slice())]).await;
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], Err(LlmError::Parse(message)) if message.contains("MAX_LINE_BYTES"))
        );
    }

    #[test]
    fn content_type_policy() {
        let mut headers = HeaderMap::new();
        assert!(require_event_stream(&headers).is_err(), "missing type");
        headers.insert(
            CONTENT_TYPE,
            "text/event-stream; charset=utf-8".parse().unwrap(),
        );
        assert!(require_event_stream(&headers).is_ok());
        assert!(require_ndjson_or_json(&headers).is_err());
        headers.insert(CONTENT_TYPE, "application/x-ndjson".parse().unwrap());
        assert!(require_ndjson_or_json(&headers).is_ok());
        headers.insert(CONTENT_TYPE, "application/json".parse().unwrap());
        assert!(require_ndjson_or_json(&headers).is_ok());
        assert!(require_event_stream(&headers).is_err());
    }
}
