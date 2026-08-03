//! E2E-001: restricted provider smoke matrix.
//!
//! Low-budget real-upstream calls to surface protocol drift that local
//! fixtures cannot cover. For each provider the harness runs the subset pinned
//! in the approved design sample (chat / stream / tools / reasoning / embed,
//! per capability matrix) and prints only status/count/error-kind — never
//! prompts, responses, or keys.
//!
//! # Gate
//!
//! Everything is env-gated by `LLMRUST_E2E=1`. Without it the harness prints
//! `skipped` and exits 0 — zero network, zero execution. This keeps ordinary
//! `cargo test` / CI runs untouched; the manual and weekly GitHub Actions runs
//! set the gate explicitly.
//!
//! ```bash
//! LLMRUST_E2E=1 cargo run --example e2e_smoke
//! ```
//!
//! A provider without its API key (or, for Ollama, with its local server
//! unreachable) is reported `skipped`, never `failed`. See
//! `docs/E2E-SMOKE.md` for the pinned model IDs, prices, verification dates,
//! and secret setup.

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use llmrust::providers::anthropic::AnthropicProvider;
use llmrust::providers::deepseek::DeepSeekProvider;
use llmrust::providers::google::GoogleProvider;
use llmrust::providers::moonshot::MoonshotProvider;
use llmrust::providers::ollama::OllamaProvider;
use llmrust::providers::openai::OpenAIProvider;
use llmrust::providers::openrouter::OpenRouterProvider;
use llmrust::providers::{LlmError, Provider, ProviderConfig};
use llmrust::types::{ChatRequest, EmbeddingRequest, ThinkingConfig, Tool};

// ── Budget / gate constants ──────────────────────────────────────────────

/// Env var that arms the harness. Anything other than exactly `"1"` disarms it.
pub const E2E_GATE_ENV: &str = "LLMRUST_E2E";

/// Max output tokens per path (SPCC §11.7 E2E-001: fixed minimal input and
/// max token ceiling). Reasoning paths may raise this where the upstream
/// contract requires it (see `ANTHROPIC_REASONING_*`).
pub const MAX_TOKENS: u64 = 128;

/// Per-request timeout in seconds (SPCC §11.7: timeout ceiling).
pub const TIMEOUT_SECS: u64 = 30;

/// Max in-flight upstream calls (SPCC §11.7: concurrency ceiling).
pub const MAX_CONCURRENCY: usize = 2;

/// Budget ceiling in US cents per run. Every path below is ≪ this; the
/// constant pins the ceiling so a future model swap cannot silently raise it.
pub const BUDGET_USD_CENTS: u64 = 10;

/// Ollama has no key; its skip trigger is "server unreachable", probed with a
/// short connect timeout (architect ruling SHOULD-3, comment 5160760281).
pub const OLLAMA_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
pub const OLLAMA_DEFAULT_ADDR: &str = "127.0.0.1:11434";

/// Fixed minimal prompt (≤ 20 words) — identical for every path so cost and
/// token counts stay comparable across providers.
pub const PROMPT: &str = "Reply with the single word: pong";

/// Tool schema used on the `tools` path — single string field, one round-trip.
const TOOL_SCHEMA: &str =
    r#"{"type":"object","properties":{"answer":{"type":"string"}},"required":["answer"]}"#;

// ── Pinned model IDs (verified 2026-08-03 against official docs; see docs/E2E-SMOKE.md) ──

pub const MODEL_OPENAI_CHAT: &str = "gpt-5-nano";
pub const MODEL_OPENAI_EMBED: &str = "text-embedding-3-small";
pub const MODEL_ANTHROPIC: &str = "claude-haiku-4-5";
pub const MODEL_GOOGLE: &str = "gemini-3.1-flash-lite";
pub const MODEL_DEEPSEEK: &str = "deepseek-v4-flash";
pub const MODEL_MOONSHOT: &str = "kimi-k2.6";
pub const MODEL_OPENROUTER: &str = "nvidia/nemotron-3-ultra-550b-a55b:free";
pub const MODEL_OLLAMA: &str = "llama3.2";
pub const MODEL_OLLAMA_EMBED: &str = "nomic-embed-text";

/// Anthropic thinking requires a non-empty budget (REA-002/O-6: the upstream
/// SDK type makes `budget_tokens` mandatory). 1024 is the minimum the
/// Anthropic API accepts; the reasoning path therefore raises max_tokens to
/// budget + small output headroom (still ≪ the $0.10 budget: 3072 tokens at
/// Haiku 4.5 pricing ≈ $0.015).
pub const ANTHROPIC_REASONING_BUDGET: u64 = 1024;
pub const ANTHROPIC_REASONING_MAX_TOKENS: u64 = 2048;

// ── Matrix ───────────────────────────────────────────────────────────────

/// A path exercised against a provider (design-sample subset).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Path {
    /// Non-streaming chat completion.
    Chat,
    /// Streaming chat completion (SSE/NDJSON surface — high drift surface).
    Stream,
    /// Single-function tool call wire round-trip.
    Tools,
    /// Reasoning path (O-1 carry-over; stream-based where upstream maps it).
    Reasoning,
    /// Embeddings call.
    Embed,
}

/// Reasoning mapping of a provider (per REASONING-CONTRACT).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReasoningKind {
    /// No reasoning path for this provider in 0.1.3.
    None,
    /// OpenAI `reasoning_effort` mapping (REA-003).
    OpenAiEffort,
    /// Anthropic extended thinking mapping (REA-002).
    AnthropicThinking,
    /// Google `thinkingConfig` mapping (REA-004G).
    GoogleThinking,
}

/// One row of the smoke matrix.
#[derive(Clone, Copy, Debug)]
pub struct SmokeTarget {
    pub provider: &'static str,
    pub model: &'static str,
    /// Env vars checked, in order, for an API key. Empty for Ollama.
    pub key_envs: &'static [&'static str],
    /// Subset exercised for this provider (design-sample pinned).
    pub paths: &'static [Path],
    pub reasoning: ReasoningKind,
    pub embed_model: Option<&'static str>,
}

/// The pinned 7-provider matrix. Model IDs are exact and official-verified;
/// no "cheapest/或同档" wording anywhere (architect ruling SHOULD-1).
pub fn matrix() -> Vec<SmokeTarget> {
    vec![
        SmokeTarget {
            provider: "openai",
            model: MODEL_OPENAI_CHAT,
            key_envs: &["OPENAI_API_KEY", "LLMRUST_OPENAI_KEY"],
            paths: &[
                Path::Chat,
                Path::Stream,
                Path::Tools,
                Path::Reasoning,
                Path::Embed,
            ],
            reasoning: ReasoningKind::OpenAiEffort,
            embed_model: Some(MODEL_OPENAI_EMBED),
        },
        SmokeTarget {
            provider: "anthropic",
            model: MODEL_ANTHROPIC,
            key_envs: &["ANTHROPIC_API_KEY", "LLMRUST_ANTHROPIC_KEY"],
            paths: &[Path::Chat, Path::Stream, Path::Tools, Path::Reasoning],
            reasoning: ReasoningKind::AnthropicThinking,
            embed_model: None,
        },
        SmokeTarget {
            provider: "google",
            model: MODEL_GOOGLE,
            key_envs: &["GOOGLE_API_KEY", "LLMRUST_GOOGLE_KEY"],
            paths: &[Path::Chat, Path::Stream, Path::Tools, Path::Reasoning],
            reasoning: ReasoningKind::GoogleThinking,
            embed_model: None,
        },
        SmokeTarget {
            provider: "deepseek",
            model: MODEL_DEEPSEEK,
            key_envs: &["DEEPSEEK_API_KEY", "LLMRUST_DEEPSEEK_KEY"],
            paths: &[Path::Chat, Path::Stream, Path::Tools],
            reasoning: ReasoningKind::None,
            embed_model: None,
        },
        SmokeTarget {
            provider: "moonshot",
            model: MODEL_MOONSHOT,
            key_envs: &["MOONSHOT_API_KEY", "LLMRUST_MOONSHOT_KEY"],
            paths: &[Path::Chat, Path::Stream, Path::Tools],
            reasoning: ReasoningKind::None,
            embed_model: None,
        },
        SmokeTarget {
            provider: "openrouter",
            model: MODEL_OPENROUTER,
            key_envs: &["OPENROUTER_API_KEY", "LLMRUST_OPENROUTER_KEY"],
            paths: &[Path::Chat, Path::Stream, Path::Tools],
            reasoning: ReasoningKind::None,
            embed_model: None,
        },
        SmokeTarget {
            provider: "ollama",
            model: MODEL_OLLAMA,
            key_envs: &[],
            paths: &[Path::Chat, Path::Stream, Path::Embed],
            reasoning: ReasoningKind::None,
            embed_model: Some(MODEL_OLLAMA_EMBED),
        },
    ]
}

/// Why a target is skipped, if at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SkipReason {
    /// Gate is disarmed (`LLMRUST_E2E` != "1") — nothing runs.
    GateDisarmed,
    /// No API key configured for this provider.
    NoKey,
    /// Ollama local server unreachable (probe timed out).
    OllamaUnreachable,
}

// ── Pure helpers (unit-testable) ─────────────────────────────────────────

/// `true` only when the gate env var is exactly `"1"`.
pub fn gate_enabled(gate_value: Option<&str>) -> bool {
    gate_value == Some("1")
}

/// Classify whether a target should be skipped. Returns `None` when the target
/// should run.
pub fn skip_reason(
    t: &SmokeTarget,
    gate_value: Option<&str>,
    key_present: bool,
    ollama_reachable: bool,
) -> Option<SkipReason> {
    if !gate_enabled(gate_value) {
        return Some(SkipReason::GateDisarmed);
    }
    if t.provider == "ollama" {
        return if ollama_reachable {
            None
        } else {
            Some(SkipReason::OllamaUnreachable)
        };
    }
    if !key_present {
        Some(SkipReason::NoKey)
    } else {
        None
    }
}

/// Short error-kind label for the redacted summary line.
pub fn error_kind_label(e: &LlmError) -> &'static str {
    match e {
        LlmError::Http(_) => "http",
        LlmError::Api { .. } => "api",
        LlmError::Stream(_) => "stream",
        LlmError::Parse(_) => "parse",
        LlmError::UnknownProvider(_) => "unknown_provider",
        LlmError::Unsupported { .. } => "unsupported",
    }
}

/// Redacted single-line record:
/// `provider/path, model, ok|error|skipped, status, ms, error_kind`.
/// Never contains prompts, responses, or keys.
pub fn format_row(
    provider_path: &str,
    model: &str,
    outcome: &str,
    status: Option<u16>,
    ms: Option<u64>,
    kind: Option<&str>,
) -> String {
    let status = status.map(|s| s.to_string()).unwrap_or_else(|| "-".into());
    let ms = ms.map(|m| m.to_string()).unwrap_or_else(|| "-".into());
    let kind = kind.unwrap_or("-");
    format!("{provider_path}, {model}, {outcome}, {status}, {ms}, {kind}")
}

// ── Runtime ──────────────────────────────────────────────────────────────

fn provider_for(t: &SmokeTarget, key: &str) -> Arc<dyn Provider> {
    let mut config = ProviderConfig::new(key);
    config.timeout_secs = Some(TIMEOUT_SECS);
    match t.provider {
        "openai" => Arc::new(OpenAIProvider::new(config)),
        "anthropic" => Arc::new(AnthropicProvider::new(config)),
        "google" => Arc::new(GoogleProvider::new(config)),
        "deepseek" => Arc::new(DeepSeekProvider::new(config)),
        "moonshot" => Arc::new(MoonshotProvider::new(config)),
        "openrouter" => Arc::new(OpenRouterProvider::new(config)),
        "ollama" => Arc::new(OllamaProvider::new(config)),
        other => panic!("unknown provider in matrix: {other}"),
    }
}

fn resolve_key(t: &SmokeTarget) -> Option<String> {
    t.key_envs
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
}

async fn ollama_reachable() -> bool {
    tokio::time::timeout(
        OLLAMA_PROBE_TIMEOUT,
        tokio::net::TcpStream::connect(OLLAMA_DEFAULT_ADDR),
    )
    .await
    .map(|r| r.is_ok())
    .unwrap_or(false)
}

/// Outcome of one path: status, elapsed ms, and error-kind label
/// (`None` on success).
struct PathOutcome {
    status: Option<u16>,
    ms: u64,
    kind: Option<&'static str>,
}

impl PathOutcome {
    fn ok(ms: u64) -> Self {
        Self {
            status: Some(200),
            ms,
            kind: None,
        }
    }
    fn err(ms: u64, e: &LlmError) -> Self {
        let status = match e {
            LlmError::Api { status, .. } => Some(*status),
            _ => None,
        };
        Self {
            status,
            ms,
            kind: Some(error_kind_label(e)),
        }
    }
    fn timeout(ms: u64) -> Self {
        Self {
            status: None,
            ms,
            kind: Some("timeout"),
        }
    }
}

/// Run one chat path (non-streaming) against the real upstream.
async fn run_chat(provider: &dyn Provider, model: &str) -> PathOutcome {
    let req = ChatRequest::new(model, PROMPT).with_max_tokens(MAX_TOKENS);
    let start = Instant::now();
    let result = tokio::time::timeout(Duration::from_secs(TIMEOUT_SECS), provider.chat(&req)).await;
    let ms = start.elapsed().as_millis() as u64;
    match result {
        Ok(Ok(_)) => PathOutcome::ok(ms),
        Ok(Err(e)) => PathOutcome::err(ms, &e),
        Err(_) => PathOutcome::timeout(ms),
    }
}

/// Consume a stream up to the first chunk: parsed chunk, error, clean end, or
/// timeout. Returns `(chunks, first_error)`.
async fn collect_first_chunk(
    stream: &mut (impl futures::Stream<Item = Result<llmrust::types::StreamChunk, LlmError>> + Unpin),
) -> (u32, Option<LlmError>) {
    let next = tokio::time::timeout(Duration::from_secs(TIMEOUT_SECS), stream.next()).await;
    match next {
        Ok(Some(Ok(_))) => (1, None),
        Ok(Some(Err(e))) => (0, Some(e)),
        Ok(None) => (0, Some(LlmError::Stream("empty stream".into()))),
        Err(_) => (0, Some(LlmError::Stream("timeout".into()))),
    }
}

/// Run one streaming path — collect at least one chunk to prove the SSE/NDJSON
/// surface parses end to end (drift's highest-frequency surface).
async fn run_stream(provider: &dyn Provider, model: &str) -> PathOutcome {
    let req = ChatRequest::new(model, PROMPT)
        .with_stream()
        .with_max_tokens(MAX_TOKENS);
    let start = Instant::now();
    let result =
        tokio::time::timeout(Duration::from_secs(TIMEOUT_SECS), provider.stream(&req)).await;
    let ms = start.elapsed().as_millis() as u64;
    match result {
        Ok(Ok(mut stream)) => {
            let (chunks, first_err) = collect_first_chunk(&mut stream).await;
            match first_err {
                Some(e) if chunks == 0 => PathOutcome::err(ms, &e),
                _ => PathOutcome::ok(ms),
            }
        }
        Ok(Err(e)) => PathOutcome::err(ms, &e),
        Err(_) => PathOutcome::timeout(ms),
    }
}

/// Run one tool-call path — minimal single-function round-trip.
async fn run_tools(provider: &dyn Provider, model: &str) -> PathOutcome {
    let tool = Tool::function(
        "get_answer",
        Some("Return the single-word answer".into()),
        serde_json::from_str(TOOL_SCHEMA).expect("static tool schema"),
    );
    let req = ChatRequest::new(model, PROMPT)
        .with_max_tokens(MAX_TOKENS)
        .with_tools(vec![tool]);
    let start = Instant::now();
    let result = tokio::time::timeout(Duration::from_secs(TIMEOUT_SECS), provider.chat(&req)).await;
    let ms = start.elapsed().as_millis() as u64;
    match result {
        Ok(Ok(_)) => PathOutcome::ok(ms),
        Ok(Err(e)) => PathOutcome::err(ms, &e),
        Err(_) => PathOutcome::timeout(ms),
    }
}

/// Run the reasoning path (O-1 carry-over). Stream-based where the upstream
/// maps reasoning on the streaming surface; contract-conformant
/// `LlmError::Unsupported` is a valid E2E result, not a failure.
async fn run_reasoning(provider: &dyn Provider, model: &str, kind: ReasoningKind) -> PathOutcome {
    let mut req = ChatRequest::new(model, PROMPT)
        .with_stream()
        .with_max_tokens(match kind {
            ReasoningKind::AnthropicThinking => ANTHROPIC_REASONING_MAX_TOKENS,
            _ => MAX_TOKENS,
        });
    match kind {
        ReasoningKind::OpenAiEffort => {
            req.thinking = Some(ThinkingConfig::Enabled {
                budget_tokens: None,
            });
        }
        ReasoningKind::AnthropicThinking => {
            req.thinking = Some(ThinkingConfig::Enabled {
                budget_tokens: Some(ANTHROPIC_REASONING_BUDGET),
            });
        }
        ReasoningKind::GoogleThinking => {
            req.thinking = Some(ThinkingConfig::Enabled {
                budget_tokens: None,
            });
        }
        ReasoningKind::None => unreachable!("reasoning path requires a reasoning kind"),
    }

    let start = Instant::now();
    let result =
        tokio::time::timeout(Duration::from_secs(TIMEOUT_SECS), provider.stream(&req)).await;
    let ms = start.elapsed().as_millis() as u64;
    match result {
        Ok(Ok(mut stream)) => {
            let (chunks, first_err) = collect_first_chunk(&mut stream).await;
            match first_err {
                Some(e) if chunks == 0 => PathOutcome::err(ms, &e),
                _ => PathOutcome::ok(ms),
            }
        }
        Ok(Err(e)) => PathOutcome::err(ms, &e),
        Err(_) => PathOutcome::timeout(ms),
    }
}

/// Run one embeddings path against the real upstream.
async fn run_embed(provider: &dyn Provider, model: &str) -> PathOutcome {
    let req = EmbeddingRequest::new(model, "ping");
    let start = Instant::now();
    let result =
        tokio::time::timeout(Duration::from_secs(TIMEOUT_SECS), provider.embed(&req)).await;
    let ms = start.elapsed().as_millis() as u64;
    match result {
        Ok(Ok(_)) => PathOutcome::ok(ms),
        Ok(Err(e)) => PathOutcome::err(ms, &e),
        Err(_) => PathOutcome::timeout(ms),
    }
}

#[tokio::main]
async fn main() {
    let gate = std::env::var(E2E_GATE_ENV).ok();
    if !gate_enabled(gate.as_deref()) {
        println!("skipped: {E2E_GATE_ENV} not set to '1' (zero network, zero execution)");
        return;
    }
    println!("provider/path, model, ok|error|skipped, status, ms, error_kind");

    let sem = Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENCY));
    let mut handles = Vec::new();
    for t in matrix() {
        let sem = Arc::clone(&sem);
        handles.push(tokio::spawn(run_target(t, sem)));
    }
    let mut lines = Vec::new();
    for h in handles {
        if let Ok(mut target_lines) = h.await {
            lines.append(&mut target_lines);
        }
    }
    // Sort for deterministic output regardless of scheduling.
    lines.sort();
    for line in lines {
        println!("{line}");
    }
}

async fn run_target(t: SmokeTarget, sem: Arc<tokio::sync::Semaphore>) -> Vec<String> {
    let _permit = match sem.acquire().await {
        Ok(p) => p,
        Err(_) => {
            return vec![format_row(
                &format!("{}/semaphore", t.provider),
                t.model,
                "error",
                None,
                None,
                Some("semaphore"),
            )]
        }
    };

    let gate = std::env::var(E2E_GATE_ENV).ok();
    let key_present = resolve_key(&t).is_some();
    let skip = if t.provider == "ollama" {
        skip_reason(&t, gate.as_deref(), false, ollama_reachable().await)
    } else {
        skip_reason(&t, gate.as_deref(), key_present, true)
    };

    if skip.is_some() {
        return vec![format_row(t.provider, t.model, "skipped", None, None, None)];
    }

    let key = resolve_key(&t).unwrap_or_default();
    let provider = provider_for(&t, &key);

    let mut lines = Vec::new();
    for path in t.paths {
        let label = format!("{}/{:?}", t.provider, path).to_lowercase();
        let outcome = match path {
            Path::Chat => run_chat(provider.as_ref(), t.model).await,
            Path::Stream => run_stream(provider.as_ref(), t.model).await,
            Path::Tools => run_tools(provider.as_ref(), t.model).await,
            Path::Reasoning => run_reasoning(provider.as_ref(), t.model, t.reasoning).await,
            Path::Embed => run_embed(provider.as_ref(), t.embed_model.unwrap_or(t.model)).await,
        };
        match outcome.kind {
            None => lines.push(format_row(
                &label,
                t.model,
                "ok",
                outcome.status,
                Some(outcome.ms),
                None,
            )),
            Some(kind) => lines.push(format_row(
                &label,
                t.model,
                "error",
                outcome.status,
                Some(outcome.ms),
                Some(kind),
            )),
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t1_gate_disarmed_skips_everything() {
        // T-1: env gate zero-execution semantics.
        assert!(!gate_enabled(None));
        assert!(!gate_enabled(Some("")));
        assert!(!gate_enabled(Some("true")));
        assert!(gate_enabled(Some("1")));
        let t = matrix()[0];
        assert_eq!(
            skip_reason(&t, None, true, true),
            Some(SkipReason::GateDisarmed)
        );
    }

    #[test]
    fn t2_no_sensitive_data_in_output() {
        // T-2: redaction — a forged key value and the prompt must never appear
        // in any row emitted by format_row.
        let secret = "sk-e2e-test-secret-0123456789abcdef";
        let row = format_row(
            "openai/chat",
            MODEL_OPENAI_CHAT,
            "ok",
            Some(200),
            Some(42),
            None,
        );
        assert!(!row.contains(secret), "key leaked into row: {row}");
        assert!(!row.contains(PROMPT), "prompt leaked into row: {row}");
        let err_row = format_row(
            "deepseek/stream",
            MODEL_DEEPSEEK,
            "error",
            Some(401),
            Some(7),
            Some("api"),
        );
        assert!(!err_row.contains(secret));
        assert!(!err_row.contains(PROMPT));
        let all = format!("{}\n{}\n", row, err_row);
        assert!(!all.contains(secret));
        assert!(!all.contains(PROMPT));
    }

    #[test]
    fn t3_skip_is_not_fail() {
        // T-3: skip semantics — no key => skipped; ollama unreachable => skipped.
        let t = SmokeTarget {
            provider: "deepseek",
            model: MODEL_DEEPSEEK,
            key_envs: &["DEEPSEEK_API_KEY"],
            paths: &[Path::Chat],
            reasoning: ReasoningKind::None,
            embed_model: None,
        };
        std::env::remove_var("DEEPSEEK_API_KEY");
        assert_eq!(
            skip_reason(&t, Some("1"), false, true),
            Some(SkipReason::NoKey)
        );
        assert_eq!(skip_reason(&t, Some("1"), true, true), None);

        let ollama_t = SmokeTarget {
            provider: "ollama",
            model: MODEL_OLLAMA,
            key_envs: &[],
            paths: &[Path::Chat],
            reasoning: ReasoningKind::None,
            embed_model: Some(MODEL_OLLAMA_EMBED),
        };
        assert_eq!(
            skip_reason(&ollama_t, Some("1"), false, false),
            Some(SkipReason::OllamaUnreachable)
        );
        assert_eq!(skip_reason(&ollama_t, Some("1"), false, true), None);
        assert_ne!(
            skip_reason(&ollama_t, Some("1"), false, false),
            Some(SkipReason::GateDisarmed)
        );
    }

    #[test]
    fn t4_output_format_is_redacted_csv() {
        // T-4: output format — fixed 6-field CSV with "-" placeholders and
        // provider/path labels.
        let ok = format_row(
            "openai/chat",
            MODEL_OPENAI_CHAT,
            "ok",
            Some(200),
            Some(42),
            None,
        );
        assert_eq!(ok, "openai/chat, gpt-5-nano, ok, 200, 42, -");
        let skip = format_row("ollama", MODEL_OLLAMA, "skipped", None, None, None);
        assert_eq!(skip, "ollama, llama3.2, skipped, -, -, -");
        let err = format_row(
            "google/stream",
            MODEL_GOOGLE,
            "error",
            Some(500),
            Some(9),
            Some("api"),
        );
        assert_eq!(
            err,
            "google/stream, gemini-3.1-flash-lite, error, 500, 9, api"
        );
    }

    #[test]
    fn t5_matrix_covers_all_7_providers_and_pins_ids() {
        // T-5: subset coverage — static reference check: all 7 providers
        // present, unique, exact pinned IDs, no vague wording, and the per
        // provider path subset matches the design sample.
        let m = matrix();
        let names: Vec<&str> = m.iter().map(|t| t.provider).collect();
        for expected in [
            "openai",
            "anthropic",
            "google",
            "deepseek",
            "moonshot",
            "openrouter",
            "ollama",
        ] {
            assert!(names.contains(&expected), "missing provider {expected}");
        }
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "duplicate provider in matrix");

        for t in &m {
            assert!(!t.model.is_empty());
            assert!(
                !t.model.contains("最廉") && !t.model.contains("或同档"),
                "vague wording leaked: {}",
                t.model
            );
        }

        // Design-sample subset pinning:
        // openai: chat + stream + tools + reasoning + embed
        // anthropic/google: chat + stream + tools + reasoning
        // deepseek/moonshot/openrouter: chat + stream + tools
        // ollama: chat + stream + embed
        let by_name = |n: &str| m.iter().find(|t| t.provider == n).unwrap();
        let paths_of = |n: &str| by_name(n).paths.to_vec();
        assert_eq!(
            paths_of("openai"),
            vec![
                Path::Chat,
                Path::Stream,
                Path::Tools,
                Path::Reasoning,
                Path::Embed
            ]
        );
        assert_eq!(
            paths_of("anthropic"),
            vec![Path::Chat, Path::Stream, Path::Tools, Path::Reasoning]
        );
        assert_eq!(
            paths_of("google"),
            vec![Path::Chat, Path::Stream, Path::Tools, Path::Reasoning]
        );
        for n in ["deepseek", "moonshot", "openrouter"] {
            assert_eq!(paths_of(n), vec![Path::Chat, Path::Stream, Path::Tools]);
        }
        assert_eq!(
            paths_of("ollama"),
            vec![Path::Chat, Path::Stream, Path::Embed]
        );
        // Reasoning kinds pinned per provider.
        assert_eq!(by_name("openai").reasoning, ReasoningKind::OpenAiEffort);
        assert_eq!(
            by_name("anthropic").reasoning,
            ReasoningKind::AnthropicThinking
        );
        assert_eq!(by_name("google").reasoning, ReasoningKind::GoogleThinking);
        // Embed targets: openai (hosted) and ollama (local).
        assert_eq!(by_name("openai").embed_model, Some(MODEL_OPENAI_EMBED));
        assert_eq!(by_name("ollama").embed_model, Some(MODEL_OLLAMA_EMBED));
    }

    #[test]
    fn t6_budget_constants_are_within_ceiling() {
        // T-6: budget ceiling assertions.
        assert!(MAX_TOKENS <= 128, "max_tokens ceiling violated");
        assert!(MAX_CONCURRENCY <= 2, "concurrency ceiling violated");
        assert!(BUDGET_USD_CENTS <= 10, "budget ceiling violated");
        assert!(TIMEOUT_SECS <= 30, "timeout ceiling violated");
        // Anthropic reasoning path stays well under the run budget at pinned
        // pricing (1024 + 2048 tokens ≈ 3072 × $5/M ≈ $0.015).
        assert!(ANTHROPIC_REASONING_MAX_TOKENS <= 4096);
    }

    #[test]
    fn error_kind_labels_are_stable() {
        // Labels used in the redacted summary must stay stable for greppability.
        let http_err: reqwest::Error = reqwest::Proxy::all("::not-a-url::").unwrap_err();
        assert_eq!(error_kind_label(&LlmError::Http(http_err)), "http");
        assert_eq!(
            error_kind_label(&LlmError::Api {
                status: 429,
                message: "x".into()
            }),
            "api"
        );
        assert_eq!(error_kind_label(&LlmError::Stream("x".into())), "stream");
        assert_eq!(error_kind_label(&LlmError::Parse("x".into())), "parse");
        assert_eq!(
            error_kind_label(&LlmError::UnknownProvider("x".into())),
            "unknown_provider"
        );
        assert_eq!(
            error_kind_label(&LlmError::Unsupported {
                feature: "f".into(),
                message: "x".into()
            }),
            "unsupported"
        );
    }
}
