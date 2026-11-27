//! Shared helpers for parsing chunked SSE / line-delimited byte streams.
//!
//! Production providers use the bounded, strict framing in
//! [`crate::providers::sse`]. The public [`line_stream`] function is the
//! deprecated 0.1.6 compatibility API: it deliberately retains the original
//! lossy decoding and unbounded buffering behaviour so existing 0.1.x
//! downstream code keeps compiling and behaving unchanged. It must not be used
//! for new production stream parsing.

pub mod legacy;

use futures::Stream;

/// Wrap a byte-chunk stream and yield complete text lines.
///
/// This is the original 0.1.6 API, retained for compatibility. New code must
/// use the strict provider stream APIs instead. Its historical behaviour is:
///
/// - Lines split across chunks are reassembled before being emitted.
/// - Only `\n` terminates a line; an optional preceding `\r` is stripped.
/// - Bytes are decoded with `String::from_utf8_lossy`.
/// - Buffering is unbounded, as in 0.1.6.
/// - Any bytes still buffered when the underlying stream ends are flushed as a
///   final line.
/// - An error from the underlying stream is forwarded, after which the stream
///   terminates.
#[deprecated(
    note = "use providers::sse strict stream APIs; legacy API retained for 0.1.x compatibility"
)]
pub fn line_stream<S, B, E>(byte_stream: S) -> impl Stream<Item = std::result::Result<String, E>>
where
    S: Stream<Item = std::result::Result<B, E>> + Send + 'static,
    B: AsRef<[u8]> + Send + 'static,
    E: Send + 'static,
{
    legacy::line_stream(byte_stream)
}

#[cfg(test)]
mod tests {
    use crate::providers::sse::strict_line_stream;
    use crate::providers::LlmError;
    use futures::StreamExt;

    async fn collect_lines(
        chunks: Vec<&'static [u8]>,
    ) -> Vec<std::result::Result<String, LlmError>> {
        let items: Vec<std::result::Result<&'static [u8], LlmError>> =
            chunks.into_iter().map(Ok).collect();
        strict_line_stream(futures::stream::iter(items))
            .collect()
            .await
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
