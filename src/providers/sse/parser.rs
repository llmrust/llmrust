//! Bounded, strict-UTF-8 line and SSE-event framing.
//!
//! HTTP response bodies arrive as arbitrary byte chunks whose boundaries do
//! **not** align with line or event boundaries. This module reassembles them
//! before decoding:
//!
//! * every complete line and the final EOF tail are decoded with
//!   [`String::from_utf8`] — invalid bytes become an explicit
//!   [`LlmError::Parse`], never U+FFFD replacement characters;
//! * lines are bounded by [`MAX_LINE_BYTES`] *before* the buffer grows;
//! * `\n`, `\r\n` and bare `\r` terminators are supported, including a
//!   `\r\n` pair split across two network chunks;
//! * SSE events are dispatched only on a blank line, so multi-line `data:`
//!   fields are joined (not parsed as separate JSON payloads);
//! * `data:` with and without the optional single space is accepted;
//! * a leading UTF-8 BOM is stripped from the first line.

use super::limits::{MAX_EVENT_BYTES, MAX_FIELD_BYTES, MAX_LINE_BYTES};
use crate::providers::{LlmError, Result};
use futures::stream::{BoxStream, Stream, StreamExt};
use std::collections::VecDeque;
use std::pin::Pin;
use std::task::{Context, Poll};

/// One dispatched SSE event.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SseEvent {
    pub(crate) event: Option<String>,
    pub(crate) data: String,
}

/// Incremental splitter from byte chunks to complete, strictly decoded lines.
struct LineSplitter {
    buf: Vec<u8>,
    /// A line ended with `\r` at the very end of the previous chunk. If the
    /// next chunk starts with `\n`, that byte is the second half of CRLF.
    skip_lf: bool,
    saw_first_line: bool,
}

impl LineSplitter {
    fn new() -> Self {
        Self {
            buf: Vec::new(),
            skip_lf: false,
            saw_first_line: false,
        }
    }

    /// Feed bytes and append every completed line to `out`.
    fn push(&mut self, bytes: &[u8], out: &mut Vec<String>) -> Result<()> {
        let mut rest = bytes;
        if self.skip_lf {
            if rest.first() == Some(&b'\n') {
                rest = &rest[1..];
            }
            self.skip_lf = false;
        }
        while !rest.is_empty() {
            let Some(pos) = rest.iter().position(|&b| b == b'\n' || b == b'\r') else {
                self.append(rest)?;
                break;
            };
            let terminator = rest[pos];
            self.append(&rest[..pos])?;
            rest = &rest[pos + 1..];
            if terminator == b'\r' {
                if rest.first() == Some(&b'\n') {
                    rest = &rest[1..];
                } else if rest.is_empty() {
                    // CRLF may be split across chunks; remember to drop LF.
                    self.skip_lf = true;
                }
            }
            self.emit_line(out)?;
        }
        Ok(())
    }

    /// Flush the final (newline-less) line, if any.
    fn finish(&mut self, out: &mut Vec<String>) -> Result<()> {
        if !self.buf.is_empty() {
            self.emit_line(out)?;
        }
        Ok(())
    }

    fn append(&mut self, bytes: &[u8]) -> Result<()> {
        let next = self.buf.len().saturating_add(bytes.len());
        if next > MAX_LINE_BYTES {
            return Err(LlmError::Parse(format!(
                "stream line exceeds MAX_LINE_BYTES ({MAX_LINE_BYTES} bytes); \
                 refusing to buffer more (KI-STREAMUTIL-LINE-CAP)"
            )));
        }
        self.buf.extend_from_slice(bytes);
        Ok(())
    }

    fn emit_line(&mut self, out: &mut Vec<String>) -> Result<()> {
        let bytes = std::mem::take(&mut self.buf);
        let mut line = String::from_utf8(bytes).map_err(|_| {
            LlmError::Parse(
                "stream is not valid UTF-8; refusing to replace invalid bytes".to_string(),
            )
        })?;
        if !self.saw_first_line {
            self.saw_first_line = true;
            if let Some(stripped) = line.strip_prefix('\u{feff}') {
                line = stripped.to_string();
            }
        }
        out.push(line);
        Ok(())
    }
}

/// Assembles SSE events from complete lines and dispatches on blank lines.
#[derive(Default)]
struct SseAssembler {
    event: Option<String>,
    data: String,
    has_data: bool,
}

impl SseAssembler {
    fn push_line(&mut self, line: &str, out: &mut Vec<SseEvent>) -> Result<()> {
        if line.is_empty() {
            if !self.data.is_empty() {
                out.push(SseEvent {
                    event: self.event.take(),
                    data: std::mem::take(&mut self.data),
                });
            } else {
                self.event = None;
                self.data.clear();
            }
            self.has_data = false;
            return Ok(());
        }
        if line.starts_with(':') {
            // Comment / keepalive.
            return Ok(());
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match field {
            "data" => {
                let needed = self
                    .data
                    .len()
                    .saturating_add(value.len())
                    .saturating_add(usize::from(self.has_data));
                if needed > MAX_EVENT_BYTES {
                    return Err(LlmError::Parse(format!(
                        "SSE event exceeds MAX_EVENT_BYTES ({MAX_EVENT_BYTES} bytes)"
                    )));
                }
                if self.has_data {
                    self.data.push('\n');
                }
                self.data.push_str(value);
                self.has_data = true;
            }
            "event" | "id" => {
                if value.len() > MAX_FIELD_BYTES {
                    return Err(LlmError::Parse(format!(
                        "SSE field `{field}` exceeds MAX_FIELD_BYTES ({MAX_FIELD_BYTES} bytes)"
                    )));
                }
                if field == "event" {
                    self.event = Some(value.to_string());
                }
            }
            // `retry` and unknown fields carry no stream semantics we consume.
            _ => {}
        }
        Ok(())
    }

    fn finish(&mut self, out: &mut Vec<SseEvent>) -> Result<()> {
        if !self.data.is_empty() {
            out.push(SseEvent {
                event: self.event.take(),
                data: std::mem::take(&mut self.data),
            });
        }
        self.has_data = false;
        Ok(())
    }
}

// ── Strict line stream ───────────────────────────────────────

struct LineStream<B, E> {
    inner: BoxStream<'static, std::result::Result<B, E>>,
    splitter: LineSplitter,
    queue: VecDeque<String>,
    /// A decode/bound/transport error discovered while a batch of already
    /// completed lines was still queued. The queued lines are delivered first;
    /// the error follows so earlier content is not lost to a later bad line.
    pending_error: Option<E>,
    ended: bool,
    failed: bool,
}

/// Wrap a byte-chunk stream and yield complete, strictly decoded text lines.
///
/// The error type `E` must be constructible from [`LlmError`] so decode and
/// bound violations can surface as explicit stream errors.
pub(crate) fn strict_line_stream<S, B, E>(
    byte_stream: S,
) -> impl Stream<Item = std::result::Result<String, E>>
where
    S: Stream<Item = std::result::Result<B, E>> + Send + 'static,
    B: AsRef<[u8]> + Send + 'static,
    E: From<LlmError> + Send + Unpin + 'static,
{
    LineStream {
        inner: byte_stream.boxed(),
        splitter: LineSplitter::new(),
        queue: VecDeque::new(),
        pending_error: None,
        ended: false,
        failed: false,
    }
}

impl<B, E> Stream for LineStream<B, E>
where
    B: AsRef<[u8]> + Send + 'static,
    E: From<LlmError> + Send + Unpin + 'static,
{
    type Item = std::result::Result<String, E>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            if let Some(line) = this.queue.pop_front() {
                return Poll::Ready(Some(Ok(line)));
            }
            if let Some(error) = this.pending_error.take() {
                this.failed = true;
                return Poll::Ready(Some(Err(error)));
            }
            if this.failed || this.ended {
                return Poll::Ready(None);
            }
            match this.inner.as_mut().poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(Ok(chunk))) => {
                    let mut lines = Vec::new();
                    let decode = this.splitter.push(chunk.as_ref(), &mut lines);
                    this.queue.extend(lines);
                    if let Err(error) = decode {
                        this.pending_error = Some(E::from(error));
                    }
                }
                Poll::Ready(Some(Err(error))) => {
                    this.pending_error = Some(error);
                }
                Poll::Ready(None) => {
                    this.ended = true;
                    let mut lines = Vec::new();
                    let decode = this.splitter.finish(&mut lines);
                    this.queue.extend(lines);
                    if let Err(error) = decode {
                        this.pending_error = Some(E::from(error));
                    }
                }
            }
        }
    }
}

// ── SSE event stream ─────────────────────────────────────────

struct EventStream {
    lines: BoxStream<'static, Result<String>>,
    assembler: SseAssembler,
    queue: VecDeque<SseEvent>,
    ended: bool,
    failed: bool,
}

/// Wrap a byte-chunk stream and yield dispatched SSE events.
pub(crate) fn event_stream<S, B>(byte_stream: S) -> impl Stream<Item = Result<SseEvent>>
where
    S: Stream<Item = Result<B>> + Send + 'static,
    B: AsRef<[u8]> + Send + 'static,
{
    EventStream {
        lines: strict_line_stream::<S, B, LlmError>(byte_stream).boxed(),
        assembler: SseAssembler::default(),
        queue: VecDeque::new(),
        ended: false,
        failed: false,
    }
}

impl Stream for EventStream {
    type Item = Result<SseEvent>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            if this.failed {
                return Poll::Ready(None);
            }
            if let Some(event) = this.queue.pop_front() {
                return Poll::Ready(Some(Ok(event)));
            }
            if this.ended {
                return Poll::Ready(None);
            }
            match this.lines.as_mut().poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(Ok(line))) => {
                    let mut events = Vec::new();
                    if let Err(error) = this.assembler.push_line(&line, &mut events) {
                        this.failed = true;
                        return Poll::Ready(Some(Err(error)));
                    }
                    this.queue.extend(events);
                }
                Poll::Ready(Some(Err(error))) => {
                    this.failed = true;
                    return Poll::Ready(Some(Err(error)));
                }
                Poll::Ready(None) => {
                    this.ended = true;
                    let mut events = Vec::new();
                    if let Err(error) = this.assembler.finish(&mut events) {
                        this.failed = true;
                        return Poll::Ready(Some(Err(error)));
                    }
                    this.queue.extend(events);
                }
            }
        }
    }
}

/// Adapt the event stream back to one complete `data:` line per event.
///
/// The providers' wire parsers consume `data:` lines; assembling events first
/// is what makes `data:`-without-space, multi-line data, CR-only framing and
/// BOM handling correct in one shared place. An `event: error` frame becomes
/// an explicit [`LlmError::Stream`] instead of a silently ignored line.
pub(crate) fn event_line_stream<S, B>(byte_stream: S) -> impl Stream<Item = Result<String>>
where
    S: Stream<Item = Result<B>> + Send + 'static,
    B: AsRef<[u8]> + Send + 'static,
{
    event_stream(byte_stream).map(|event| match event {
        Ok(event) => {
            if event.event.as_deref() == Some("error") {
                Err(LlmError::Stream(sse_error_message(&event.data)))
            } else {
                Ok(format!("data: {}", event.data))
            }
        }
        Err(error) => Err(error),
    })
}

/// Best-effort message for an `event: error` frame. Bounded and never echoes
/// the whole event payload; only the upstream-declared message is surfaced.
fn sse_error_message(data: &str) -> String {
    let declared = serde_json::from_str::<serde_json::Value>(data)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(|message| message.as_str())
                .or_else(|| value.get("message").and_then(|message| message.as_str()))
                .map(str::to_string)
        });
    let mut message = declared.unwrap_or_else(|| "upstream sent an SSE error event".to_string());
    if message.is_empty() {
        message = "upstream sent an SSE error event".to_string();
    }
    const MAX_ERROR_CHARS: usize = 512;
    if message.chars().count() > MAX_ERROR_CHARS {
        let cut = message
            .char_indices()
            .nth(MAX_ERROR_CHARS)
            .map(|(index, _)| index)
            .unwrap_or(message.len());
        message.truncate(cut);
        message.push_str("...");
    }
    message
}
