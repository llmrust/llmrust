//! Hard upper bounds for incremental stream parsing.
//!
//! These are deliberately finite: an upstream (or a broken intermediary) must
//! never be able to make the library buffer an unbounded amount of memory
//! before a line/event terminator arrives. Every bound is checked *before* the
//! backing buffer grows, so the process fails with an explicit parse error
//! instead of allocating without limit.

/// Maximum bytes one physical line (SSE or newline-delimited JSON) may occupy,
/// excluding its line terminator.
pub(crate) const MAX_LINE_BYTES: usize = 1024 * 1024;

/// Maximum bytes one dispatched SSE event's `data` payload may occupy after
/// multi-line `data:` fields are joined with `\n`.
pub(crate) const MAX_EVENT_BYTES: usize = 4 * 1024 * 1024;

/// Maximum bytes one non-data SSE field (`event:`, `id:`, ...) may occupy.
pub(crate) const MAX_FIELD_BYTES: usize = 8 * 1024;
