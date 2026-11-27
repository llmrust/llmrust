//! Shared helpers for parsing chunked SSE / line-delimited byte streams.
//!
//! HTTP response bodies arrive as arbitrary byte chunks whose boundaries do
//! **not** align with SSE line boundaries. A naive per-chunk `.lines()` split
//! therefore has two bugs:
//!
//! 1. A `data:` line split across two network packets is parsed as two
//!    incomplete halves, both of which fail to deserialize and are silently
//!    dropped — losing tokens. The more fragmented the network, the more is
//!    lost.
//! 2. A multi-byte UTF-8 character (e.g. CJK text or emoji) split across a
//!    packet boundary is turned into the U+FFFD replacement character by lossy
//!    decoding, corrupting the output.
//!
//! [`line_stream`] fixes both by accumulating raw bytes in a buffer and only
//! decoding a line once a complete terminator is available. Decoding is
//! **strict**: invalid UTF-8 is an explicit error instead of replacement
//! characters, and a line is rejected before its buffer can grow past the
//! shared hard bound.
//!
//! Most providers now consume the richer [`crate::providers::sse`] event
//! parser; this function remains as the public line-level API. Its generic
//! error type gained a `From<LlmError>` bound so decode/bound violations can
//! be surfaced through the caller's error type rather than swallowed.

use crate::providers::LlmError;
use futures::Stream;

/// Wrap a byte-chunk stream and yield complete text lines.
///
/// - Lines split across chunks are reassembled before being emitted.
/// - Multi-byte UTF-8 split across chunks is reassembled before decoding.
/// - `\n`, `\r\n` and bare `\r` terminators are accepted (including a `\r\n`
///   pair split across two chunks).
/// - A leading UTF-8 BOM is stripped from the first line.
/// - The trailing terminator is stripped.
/// - Any bytes still buffered when the underlying stream ends are flushed as a
///   final line.
/// - An error from the underlying stream is forwarded, after which the stream
///   terminates.
/// - Invalid UTF-8 or an oversized line becomes an `Err` carrying
///   [`LlmError::Parse`]; it is never replaced with U+FFFD.
pub fn line_stream<S, B, E>(byte_stream: S) -> impl Stream<Item = std::result::Result<String, E>>
where
    S: Stream<Item = std::result::Result<B, E>> + Send + 'static,
    B: AsRef<[u8]> + Send + 'static,
    E: From<LlmError> + Send + Unpin + 'static,
{
    crate::providers::sse::strict_line_stream(byte_stream)
}

#[cfg(test)]
mod tests {
    use super::line_stream;
    use crate::providers::LlmError;
    use futures::StreamExt;

    async fn collect_lines(
        chunks: Vec<&'static [u8]>,
    ) -> Vec<std::result::Result<String, LlmError>> {
        let items: Vec<std::result::Result<&'static [u8], LlmError>> =
            chunks.into_iter().map(Ok).collect();
        line_stream(futures::stream::iter(items)).collect().await
    }

    async fn lines(chunks: Vec<&'static [u8]>) -> Vec<String> {
        collect_lines(chunks)
            .await
            .into_iter()
            .map(|line| line.unwrap())
            .collect()
    }

    #[tokio::test]
    async fn reassembles_line_split_across_chunks() {
        // "data: hello\n" split in the middle of the word.
        let lines = lines(vec![b"data: hel", b"lo\n"]).await;
        assert_eq!(lines, vec!["data: hello".to_string()]);
    }

    #[tokio::test]
    async fn reassembles_multibyte_utf8_split_across_chunks() {
        // "你好" is 6 bytes (3 per char). Split inside the first character so
        // that neither chunk is independently valid UTF-8.
        let bytes = "你好".as_bytes();
        let (head, tail) = bytes.split_at(2);
        let head: &'static [u8] = Box::leak(head.to_vec().into_boxed_slice());
        let mut tail_buf = tail.to_vec();
        tail_buf.push(b'\n');
        let tail: &'static [u8] = Box::leak(tail_buf.into_boxed_slice());

        let lines = lines(vec![head, tail]).await;
        assert_eq!(lines, vec!["你好".to_string()]);
        assert!(!lines[0].contains('\u{FFFD}'));
    }

    #[tokio::test]
    async fn splits_multiple_lines_and_flushes_tail() {
        let lines = lines(vec![b"a\nb\nc"]).await;
        assert_eq!(
            lines,
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
    }

    #[tokio::test]
    async fn strips_crlf_line_endings() {
        let lines = lines(vec![b"x\r\ny\r\n"]).await;
        assert_eq!(lines, vec!["x".to_string(), "y".to_string()]);
    }

    #[tokio::test]
    async fn invalid_utf8_is_an_error_not_replacement_characters() {
        let out = collect_lines(vec![b"ok\n", b"bad \xFF\n"]).await;
        assert!(matches!(&out[0], Ok(line) if line == "ok"));
        assert!(
            matches!(&out[1], Err(LlmError::Parse(message)) if message.contains("UTF-8")),
            "expected a Parse error, got {:?}",
            out[1]
        );
    }
}
