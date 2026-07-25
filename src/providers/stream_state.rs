//! Shared streaming terminal-state machine (STR-001).
//!
//! Collapses a provider's already-parsed [`StreamChunk`] sequence into the
//! single-terminal shape required by SPCC §6.5 / §6.6:
//!
//! ```text
//! START -> CONTENT/REASONING/TOOL (0..N) -> TERMINAL (exactly 1) -> CLOSED
//! ```
//!
//! This module consumes *event semantics* only — it never touches wire
//! bytes. Per the STR-001 design (Issue #116), it is the shared collapse
//! layer that individual providers no longer need to re-implement.
//!
//! Invariant (DoD): on success the public `stream()` output carries exactly
//! one `done = true` chunk, that terminal carries the final
//! `finish_reason` / `usage` / `tool_calls` / `thinking_done`, and an `Err`
//! is never followed by a success terminal (§6.6: error priority).

use crate::types::{FinishReason, StreamChunk, ToolCall, Usage};
use crate::Result;
use futures::stream::Stream;
use std::pin::Pin;
use std::task::{Context, Poll};

/// Internal phase of the collapse machine.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// No terminal signal seen yet.
    Started,
    /// A `done = true` chunk has been seen; emission is deferred (lazy
    /// terminal) so late-arriving metadata (e.g. a usage-only chunk after
    /// the finish chunk) is still captured before the single terminal is
    /// emitted at end-of-stream.
    TerminalSeen,
    /// An `Err` was seen; the stream closes and must never emit a success
    /// terminal.
    Errored,
}

/// Stateful stream adapter produced by [`unify_terminal`].
pub(crate) struct UnifyTerminal<S> {
    inner: S,
    phase: Phase,
    /// Buffered textual delta from the first `done = true` chunk. Terminal
    /// chunks after the first are dropped (their content is not forwarded),
    /// so only the first terminal's delta is preserved.
    final_delta: String,
    finish_reason: Option<FinishReason>,
    usage: Option<Usage>,
    tool_calls: Option<Vec<ToolCall>>,
    thinking_done: Option<bool>,
    /// Whether the single trailing terminal has already been emitted.
    terminal_emitted: bool,
}

impl<S> UnifyTerminal<S>
where
    S: Stream<Item = Result<StreamChunk>> + Unpin,
{
    /// Harvest metadata from a chunk without touching wire bytes. Terminal
    /// *content* (delta) is handled separately so the machine can drop
    /// post-terminal content while still keeping the final metadata.
    fn harvest(&mut self, chunk: &StreamChunk) {
        if chunk.finish_reason.is_some() {
            self.finish_reason = chunk.finish_reason.clone();
        }
        if chunk.usage.is_some() {
            self.usage = chunk.usage.clone();
        }
        if chunk.tool_calls.is_some() {
            self.tool_calls = chunk.tool_calls.clone();
        }
        if chunk.thinking_done.is_some() {
            self.thinking_done = chunk.thinking_done;
        }
    }

    /// Build and mark-emitted the single trailing terminal.
    fn emit_terminal(&mut self) -> StreamChunk {
        self.terminal_emitted = true;
        StreamChunk {
            delta: std::mem::take(&mut self.final_delta),
            done: true,
            finish_reason: self.finish_reason.clone(),
            usage: self.usage.clone(),
            tool_calls: self.tool_calls.clone(),
            thinking: None,
            thinking_done: self.thinking_done,
        }
    }
}

impl<S> Stream for UnifyTerminal<S>
where
    S: Stream<Item = Result<StreamChunk>> + Unpin,
{
    type Item = Result<StreamChunk>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            if this.terminal_emitted {
                return Poll::Ready(None);
            }
            match Pin::new(&mut this.inner).poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => {
                    // Stream ended. In the Errored phase we never emit a
                    // success terminal. Otherwise emit exactly one terminal,
                    // covering both a real terminal already seen
                    // (TerminalSeen) and the safety net for providers that
                    // omit `done` (Started → synthesize).
                    if this.phase == Phase::Errored {
                        return Poll::Ready(None);
                    }
                    return Poll::Ready(Some(Ok(this.emit_terminal())));
                }
                Poll::Ready(Some(Err(e))) => {
                    this.phase = Phase::Errored;
                    // Forward the error and close; never a success terminal.
                    return Poll::Ready(Some(Err(e)));
                }
                Poll::Ready(Some(Ok(chunk))) => {
                    if this.phase == Phase::Errored {
                        // Defensive: drop anything that somehow follows an error.
                        return Poll::Ready(None);
                    }
                    if chunk.done {
                        if this.phase == Phase::Started {
                            // First terminal signal: defer emission, harvest
                            // metadata and buffer this chunk's delta.
                            this.phase = Phase::TerminalSeen;
                            this.harvest(&chunk);
                            this.final_delta.push_str(&chunk.delta);
                        } else {
                            // Repeat terminal ([DONE] / usage-only after
                            // finish): drop content, keep metadata only.
                            this.harvest(&chunk);
                        }
                        // Do not forward; continue draining so late metadata
                        // is captured before the trailing emit.
                        continue;
                    }
                    // Non-terminal chunk.
                    if this.phase == Phase::Started {
                        // Forward as-is (still `done = false`), harvest any
                        // metadata it carries (e.g. a usage-only chunk).
                        this.harvest(&chunk);
                        return Poll::Ready(Some(Ok(chunk)));
                    }
                    // TerminalSeen + non-terminal chunk: post-terminal content.
                    // Drop it (§6.5) but keep harvesting metadata.
                    this.harvest(&chunk);
                    continue;
                }
            }
        }
    }
}

/// Collapse a provider's parsed [`StreamChunk`] stream into the single-terminal
/// shape required by SPCC §6.5 / §6.6.
///
/// The returned stream:
/// * forwards every non-terminal (`done = false`) chunk unchanged;
/// * defers the single terminal: a `done = true` chunk is *not* forwarded
///   immediately — its metadata is harvested and its delta buffered;
/// * emits exactly one `done = true` terminal at end-of-stream carrying the
///   final metadata (this captures usage that arrives after the finish chunk);
/// * synthesizes a terminal at end-of-stream if none was ever seen (safety net
///   for providers that omit `done`);
/// * forwards any `Err` and then closes without emitting a success terminal.
///
/// This function never inspects wire bytes; it operates purely on the parsed
/// event stream.
pub(crate) fn unify_terminal<S>(stream: S) -> UnifyTerminal<S>
where
    S: Stream<Item = Result<StreamChunk>> + Unpin + Send + 'static,
{
    UnifyTerminal {
        inner: stream,
        phase: Phase::Started,
        final_delta: String::new(),
        finish_reason: None,
        usage: None,
        tool_calls: None,
        thinking_done: None,
        terminal_emitted: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::LlmError;
    use futures::stream::{self, StreamExt};

    /// Drain the machine into a `Vec` for assertions.
    async fn drain(chunks: Vec<Result<StreamChunk>>) -> Vec<Result<StreamChunk>> {
        unify_terminal(stream::iter(chunks)).collect().await
    }

    /// Count `done = true` chunks among the successfully parsed items.
    fn count_done(out: &[Result<StreamChunk>]) -> usize {
        out.iter()
            .filter_map(|r| r.as_ref().ok())
            .filter(|c| c.done)
            .count()
    }

    /// Return the (unique) terminal chunk.
    fn terminal_of(out: &[Result<StreamChunk>]) -> &StreamChunk {
        out.iter()
            .filter_map(|r| r.as_ref().ok())
            .find(|c| c.done)
            .expect("exactly one terminal")
    }

    fn usage(t: u64, p: u64, c: u64) -> Usage {
        Usage {
            prompt_tokens: p,
            completion_tokens: c,
            total_tokens: t,
            ..Default::default()
        }
    }

    // T-1: finish(done=T) → usage(done=T) → [DONE](done=T), with leading text.
    #[tokio::test]
    async fn t1_finish_then_usage_then_done() {
        let out = drain(vec![
            Ok(StreamChunk {
                delta: "hi".into(),
                ..Default::default()
            }),
            Ok(StreamChunk {
                delta: String::new(),
                done: true,
                finish_reason: Some(FinishReason::Stop),
                ..Default::default()
            }),
            Ok(StreamChunk {
                delta: String::new(),
                done: true,
                usage: Some(usage(8, 5, 3)),
                ..Default::default()
            }),
            Ok(StreamChunk {
                delta: String::new(),
                done: true,
                ..Default::default()
            }),
        ])
        .await;
        assert_eq!(count_done(&out), 1, "exactly one done");
        let term = terminal_of(&out);
        assert_eq!(term.finish_reason, Some(FinishReason::Stop));
        assert_eq!(
            term.usage.as_ref().unwrap().total_tokens,
            8,
            "late usage captured"
        );
    }

    // T-2: usage-only(done=F) → finish(done=T).
    #[tokio::test]
    async fn t2_usage_then_finish() {
        let out = drain(vec![
            Ok(StreamChunk {
                delta: String::new(),
                usage: Some(usage(2, 1, 1)),
                ..Default::default()
            }),
            Ok(StreamChunk {
                delta: "hi".into(),
                done: true,
                finish_reason: Some(FinishReason::Stop),
                ..Default::default()
            }),
        ])
        .await;
        assert_eq!(count_done(&out), 1);
        let term = terminal_of(&out);
        assert_eq!(term.usage.as_ref().unwrap().total_tokens, 2);
        assert_eq!(term.finish_reason, Some(FinishReason::Stop));
    }

    // T-3: text(done=F) → Err. No success terminal must appear.
    #[tokio::test]
    async fn t3_error_after_content() {
        let out = drain(vec![
            Ok(StreamChunk {
                delta: "partial".into(),
                ..Default::default()
            }),
            Err(LlmError::Stream("boom".into())),
        ])
        .await;
        assert_eq!(count_done(&out), 0, "no success terminal after error");
        assert!(out.iter().any(|r| r.is_err()), "error forwarded");
    }

    // T-4: tool delta → finish(done=T). Terminal must carry tool_calls.
    #[tokio::test]
    async fn t4_tool_fragments() {
        let tool_calls = Some(vec![ToolCall {
            id: "call_1".into(),
            call_type: "function".into(),
            function: crate::types::FunctionCall {
                name: "greet".into(),
                arguments: "{}".into(),
            },
        }]);
        let out = drain(vec![
            Ok(StreamChunk {
                tool_calls: tool_calls.clone(),
                ..Default::default()
            }),
            Ok(StreamChunk {
                delta: String::new(),
                done: true,
                finish_reason: Some(FinishReason::Stop),
                ..Default::default()
            }),
        ])
        .await;
        assert_eq!(count_done(&out), 1);
        let term = terminal_of(&out);
        assert_eq!(term.tool_calls.as_ref().unwrap().len(), 1);
        assert_eq!(term.finish_reason, Some(FinishReason::Stop));
    }

    // T-5: text → done=T → done=T. Second terminal must be dropped.
    #[tokio::test]
    async fn t5_duplicate_done() {
        let out = drain(vec![
            Ok(StreamChunk {
                delta: "x".into(),
                ..Default::default()
            }),
            Ok(StreamChunk {
                delta: String::new(),
                done: true,
                ..Default::default()
            }),
            Ok(StreamChunk {
                delta: String::new(),
                done: true,
                ..Default::default()
            }),
        ])
        .await;
        assert_eq!(count_done(&out), 1);
    }

    // T-6: terminal(done=T) → content(done=F). Post-terminal content dropped.
    #[tokio::test]
    async fn t6_post_terminal_content() {
        let out = drain(vec![
            Ok(StreamChunk {
                delta: String::new(),
                done: true,
                finish_reason: Some(FinishReason::Stop),
                ..Default::default()
            }),
            Ok(StreamChunk {
                delta: "leak".into(),
                ..Default::default()
            }),
        ])
        .await;
        assert_eq!(count_done(&out), 1);
        let forwarded_non_terminal = out.iter().filter_map(|r| r.as_ref().ok()).any(|c| !c.done);
        assert!(
            !forwarded_non_terminal,
            "post-terminal content must be dropped, not forwarded"
        );
    }

    // T-7: empty start terminal (done=T, no content).
    #[tokio::test]
    async fn t7_empty_start_terminal() {
        let out = drain(vec![Ok(StreamChunk {
            delta: String::new(),
            done: true,
            ..Default::default()
        })])
        .await;
        assert_eq!(count_done(&out), 1);
        let term = terminal_of(&out);
        assert_eq!(term.delta, "", "empty terminal content is allowed");
    }

    // T-8: text → end with no `done`. Synthesized terminal must appear once,
    // and must not duplicate the already-forwarded text.
    #[tokio::test]
    async fn t8_no_done_on_end() {
        let out = drain(vec![Ok(StreamChunk {
            delta: "text".into(),
            ..Default::default()
        })])
        .await;
        assert_eq!(count_done(&out), 1);
        let term = terminal_of(&out);
        assert_eq!(
            term.delta, "",
            "synthesized terminal carries no duplicate text"
        );
    }

    // T-9: thinking → thinking_done → finish(done=T). thinking_done reaches
    // the terminal (E-003 not regressed).
    #[tokio::test]
    async fn t9_reasoning_preserved() {
        let out = drain(vec![
            Ok(StreamChunk {
                thinking: Some("let me think".into()),
                ..Default::default()
            }),
            Ok(StreamChunk {
                thinking_done: Some(true),
                ..Default::default()
            }),
            Ok(StreamChunk {
                delta: String::new(),
                done: true,
                finish_reason: Some(FinishReason::Stop),
                ..Default::default()
            }),
        ])
        .await;
        assert_eq!(count_done(&out), 1);
        let term = terminal_of(&out);
        assert_eq!(
            term.thinking_done,
            Some(true),
            "thinking_done reaches terminal"
        );
    }
}
