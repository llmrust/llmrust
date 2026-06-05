//! Shared helpers for parsing chunked SSE / line-delimited byte streams.
//!
//! HTTP response bodies arrive as arbitrary byte chunks whose boundaries do
//! **not** align with SSE line boundaries. A naive
//! `for line in String::from_utf8_lossy(&chunk).lines()` therefore has two
//! bugs:
//!
//! 1. A `data:` line split across two network packets is parsed as two
//!    incomplete halves, both of which fail to deserialize and are silently
//!    dropped — losing tokens. The more fragmented the network, the more is
//!    lost.
//! 2. A multi-byte UTF-8 character (e.g. CJK text or emoji) split across a
//!    packet boundary is turned into the U+FFFD replacement character by
//!    `from_utf8_lossy`, corrupting the output.
//!
//! [`line_stream`] fixes both by accumulating raw bytes in a buffer and only
//! decoding a line once a complete `\n`-terminated line is available.

use futures::{Stream, StreamExt};

/// Wrap a byte-chunk stream and yield complete text lines.
///
/// - Lines split across chunks are reassembled before being emitted.
/// - Multi-byte UTF-8 split across chunks is reassembled before decoding, so
///   it is never corrupted into replacement characters.
/// - The trailing `\n` (and an optional preceding `\r`) is stripped.
/// - Any bytes still buffered when the underlying stream ends are flushed as a
///   final line.
/// - An error from the underlying stream is forwarded, after which the stream
///   terminates.
pub fn line_stream<S, B, E>(
    byte_stream: S,
) -> impl Stream<Item = std::result::Result<String, E>>
where
    S: Stream<Item = std::result::Result<B, E>> + Send + 'static,
    B: AsRef<[u8]> + Send + 'static,
    E: Send + 'static,
{
    futures::stream::unfold(
        (byte_stream.boxed(), Vec::<u8>::new(), false),
        |(mut stream, mut buf, mut ended)| async move {
            loop {
                // Emit a complete line if the buffer already contains one.
                if let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                    let mut line: Vec<u8> = buf.drain(..=pos).collect();
                    line.pop(); // drop the trailing '\n'
                    if line.last() == Some(&b'\r') {
                        line.pop(); // drop a trailing '\r' (CRLF endings)
                    }
                    let decoded = String::from_utf8_lossy(&line).into_owned();
                    return Some((Ok(decoded), (stream, buf, ended)));
                }

                if ended {
                    // Underlying stream finished: flush any trailing bytes as a
                    // final (newline-less) line, then stop.
                    if buf.is_empty() {
                        return None;
                    }
                    let decoded = String::from_utf8_lossy(&buf).into_owned();
                    buf.clear();
                    return Some((Ok(decoded), (stream, buf, true)));
                }

                match stream.next().await {
                    Some(Ok(chunk)) => buf.extend_from_slice(chunk.as_ref()),
                    Some(Err(e)) => {
                        // Forward the error, then terminate on the next poll.
                        buf.clear();
                        return Some((Err(e), (stream, buf, true)));
                    }
                    None => ended = true,
                }
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::line_stream;
    use futures::StreamExt;

    async fn collect_lines(chunks: Vec<&'static [u8]>) -> Vec<String> {
        let items: Vec<std::result::Result<&'static [u8], std::io::Error>> =
            chunks.into_iter().map(Ok).collect();
        line_stream(futures::stream::iter(items))
            .map(|r| r.unwrap())
            .collect::<Vec<_>>()
            .await
    }

    #[tokio::test]
    async fn reassembles_line_split_across_chunks() {
        // "data: hello\n" split in the middle of the word.
        let lines = collect_lines(vec![b"data: hel", b"lo\n"]).await;
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

        let lines = collect_lines(vec![head, tail]).await;
        assert_eq!(lines, vec!["你好".to_string()]);
        assert!(!lines[0].contains('\u{FFFD}'));
    }

    #[tokio::test]
    async fn splits_multiple_lines_and_flushes_tail() {
        let lines = collect_lines(vec![b"a\nb\nc"]).await;
        assert_eq!(
            lines,
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
    }

    #[tokio::test]
    async fn strips_crlf_line_endings() {
        let lines = collect_lines(vec![b"x\r\ny\r\n"]).await;
        assert_eq!(lines, vec!["x".to_string(), "y".to_string()]);
    }
}
