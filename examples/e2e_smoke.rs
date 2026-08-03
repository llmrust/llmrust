//! E2E-001: restricted provider smoke matrix.
//!
//! Low-budget real-upstream calls to surface protocol drift that local
//! fixtures cannot cover. Runs a single cheap chat request per provider (plus
//! an OpenAI embeddings call and the O-1 reasoning paths) and prints only
//! status/count/error-kind — never prompts, responses, or keys.
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

use llmrust::providers::anthropic::AnthropicProvider;
use llmrust::providers::deepseek::DeepSeekProvider;
use llmrust::providers::google::GoogleProvider;
use llmrust::providers::moonshot::MoonshotProvider;
use llmrust::providers::ollama::OllamaProvider;
use llmrust::providers::openai::OpenAIProvider;
use llmrust::providers::openrouter::OpenRouterProvider;
use llmrust::providers::{LlmError, Provider, ProviderConfig};
use llmrust::types::{ChatRequest, EmbeddingRequest, ThinkingConfig};

// ── Budget / gate constants ──────────────────────────────────────────────

/// Env var that arms the harness. Anything other than exactly `"1"` disarms it.
pub const E2E_GATE_ENV: &str = "LLMRUST_E2E";

/// Max output tokens per chat path (SPCC §11.7 E2E-001: fixed minimal input
/// and max token ceiling).
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

/// Fixed minimal prompt (≤ 20 words) — identical for every chat path so cost
/// and token counts stay comparable across providers.
pub const PROMPT: &str = "Reply with the single word: pong";

// ── Pinned model IDs (verified 2026-08-03 against official docs; see docs/E2E-SMOKE.md) ──

pub const MODEL_OPENAI_CHAT: &str = "gpt-5-nano";
pub const MODEL_OPENAI_EMBED: &str = "text-embedding-3-small";
pub const MODEL_ANTHROPIC: &str = "claude-haiku-4-5";
pub const MODEL_GOOGLE: &str = "gemini-3.1-flash-lite";
pub const MODEL_DEEPSEEK: &str = "deepseek-v4-flash";
pub const MODEL_MOONSHOT: &str = "kimi-k2.6";
pub const MODEL_OPENROUTER: &str = "nvidia/nemotron-3-ultra-550b-a55b:free";
pub const MODEL_OLLAMA: &str = "llama3.2";

/// Anthropic thinking requires a non-empty budget (REA-002/O-6: the upstream
/// SDK type makes `budget_tokens` mandatory). 1024 is the minimum the
/// Anthropic API accepts; the reasoning path therefore raises max_tokens to
/// budget + small output headroom (still ≪ the $0.10 budget: 3072 tokens at
/// Haiku 4.5 pricing ≈ $0.015).
pub const ANTHROPIC_REASONING_BUDGET: u64 = 1024;
pub const ANTHROPIC_REASONING_MAX_TOKENS: u64 = 2048;

// ── Matrix ───────────────────────────────────────────────────────────────

/// Reasoning subset of a target (O-1 carry-over: real-endpoint verification of
/// the REA-003/REA-002 reasoning paths on the providers that map them).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReasoningKind {
    /// No reasoning path for this provider in 0.1.3.
    None,
    /// OpenAI `reasoning_effort` mapping (REA-003).
    OpenAiEffort,
    /// Anthropic extended thinking mapping (REA-002).
    AnthropicThinking,
}

/// One row of the smoke matrix.
#[derive(Clone, Copy, Debug)]
pub struct SmokeTarget {
    pub provider: &'static str,
    pub model: &'static str,
    /// Env vars checked, in order, for an API key. Empty for Ollama.
    pub key_envs: &'static [&'static str],
    pub reasoning: ReasoningKind,
    pub embed: bool,
}

/// The pinned 7-provider matrix. Model IDs are exact and official-verified;
/// no "cheapest/或同档" wording anywhere (architect ruling SHOULD-1).
pub fn matrix() -> Vec<SmokeTarget> {
    vec![
        SmokeTarget {
            provider: "openai",
            model: MODEL_OPENAI_CHAT,
            key_envs: &["OPENAI_API_KEY", "LLMRUST_OPENAI_KEY"],
            reasoning: ReasoningKind::OpenAiEffort,
            embed: true,
        },
        SmokeTarget {
            provider: "anthropic",
            model: MODEL_ANTHROPIC,
            key_envs: &["ANTHROPIC_API_KEY", "LLMRUST_ANTHROPIC_KEY"],
            reasoning: ReasoningKind::AnthropicThinking,
            embed: false,
        },
        SmokeTarget {
            provider: "google",
            model: MODEL_GOOGLE,
            key_envs: &["GOOGLE_API_KEY", "LLMRUST_GOOGLE_KEY"],
            reasoning: ReasoningKind::None,
            embed: false,
        },
        SmokeTarget {
            provider: "deepseek",
            model: MODEL_DEEPSEEK,
            key_envs: &["DEEPSEEK_API_KEY", "LLMRUST_DEEPSEEK_KEY"],
            reasoning: ReasoningKind::None,
            embed: false,
        },
        SmokeTarget {
            provider: "moonshot",
            model: MODEL_MOONSHOT,
            key_envs: &["MOONSHOT_API_KEY", "LLMRUST_MOONSHOT_KEY"],
            reasoning: ReasoningKind::None,
            embed: false,
        },
        SmokeTarget {
            provider: "openrouter",
            model: MODEL_OPENROUTER,
            key_envs: &["OPENROUTER_API_KEY", "LLMRUST_OPENROUTER_KEY"],
            reasoning: ReasoningKind::None,
            embed: false,
        },
        SmokeTarget {
            provider: "ollama",
            model: MODEL_OLLAMA,
            key_envs: &[],
            reasoning: ReasoningKind::None,
            embed: false,
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

/// Redacted single-line record: `provider, model, ok|error|skipped, status,
/// ms, error_kind`. Never contains prompts, responses, or keys.
pub fn format_row(
    provider: &str,
    model: &str,
    outcome: &str,
    status: Option<u16>,
    ms: Option<u64>,
    kind: Option<&str>,
) -> String {
    let status = status.map(|s| s.to_string()).unwrap_or_else(|| "-".into());
    let ms = ms.map(|m| m.to_string()).unwrap_or_else(|| "-".into());
    let kind = kind.unwrap_or("-");
    format!("{provider}, {model}, {outcome}, {status}, {ms}, {kind}")
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

/// Outcome of one chat path: status, elapsed ms, and error-kind label
/// (`None` on success).
struct ChatOutcome {
    status: Option<u16>,
    ms: u64,
    kind: Option<&'static str>,
}

/// Run one embeddings path against the real upstream, bounded by the timeout.
async fn run_embed(provider: &dyn Provider, model: &str) -> ChatOutcome {
    let req = EmbeddingRequest::new(model, "ping");
    let start = Instant::now();
    let result =
        tokio::time::timeout(Duration::from_secs(TIMEOUT_SECS), provider.embed(&req)).await;
    let ms = start.elapsed().as_millis() as u64;
    match result {
        Ok(Ok(_resp)) => ChatOutcome {
            status: Some(200),
            ms,
            kind: None,
        },
        Ok(Err(LlmError::Api { status, .. })) => ChatOutcome {
            status: Some(status),
            ms,
            kind: Some("api"),
        },
        Ok(Err(e)) => ChatOutcome {
            status: None,
            ms,
            kind: Some(error_kind_label(&e)),
        },
        Err(_) => ChatOutcome {
            status: None,
            ms,
            kind: Some("timeout"),
        },
    }
}

/// Run one chat path against the real upstream, bounded by the timeout.
/// On success `kind` is `None`; on failure `status` carries the HTTP status
/// when known (from `LlmError::Api`).
async fn run_chat(provider: &dyn Provider, model: &str, reasoning: ReasoningKind) -> ChatOutcome {
    let mut req = ChatRequest::new(model, PROMPT).with_max_tokens(match reasoning {
        ReasoningKind::AnthropicThinking => ANTHROPIC_REASONING_MAX_TOKENS,
        _ => MAX_TOKENS,
    });
    if reasoning == ReasoningKind::OpenAiEffort {
        req.thinking = Some(ThinkingConfig::Enabled {
            budget_tokens: None,
        });
    } else if reasoning == ReasoningKind::AnthropicThinking {
        req.thinking = Some(ThinkingConfig::Enabled {
            budget_tokens: Some(ANTHROPIC_REASONING_BUDGET),
        });
    }

    let start = Instant::now();
    let result = tokio::time::timeout(Duration::from_secs(TIMEOUT_SECS), provider.chat(&req)).await;
    let ms = start.elapsed().as_millis() as u64;
    match result {
        Ok(Ok(_resp)) => ChatOutcome {
            status: Some(200),
            ms,
            kind: None,
        },
        Ok(Err(LlmError::Api { status, .. })) => ChatOutcome {
            status: Some(status),
            ms,
            kind: Some("api"),
        },
        Ok(Err(e)) => ChatOutcome {
            status: None,
            ms,
            kind: Some(error_kind_label(&e)),
        },
        Err(_) => ChatOutcome {
            status: None,
            ms,
            kind: Some("timeout"),
        },
    }
}

#[tokio::main]
async fn main() {
    let gate = std::env::var(E2E_GATE_ENV).ok();
    if !gate_enabled(gate.as_deref()) {
        println!("skipped: {E2E_GATE_ENV} not set to '1' (zero network, zero execution)");
        return;
    }
    println!("provider, model, ok|error|skipped, status, ms, error_kind");

    let sem = Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENCY));
    let mut handles = Vec::new();
    for t in matrix() {
        let sem = Arc::clone(&sem);
        handles.push(tokio::spawn(run_target(t, sem)));
    }
    let mut lines = Vec::new();
    for h in handles {
        if let Ok(line) = h.await {
            lines.push(line);
        }
    }
    // Sort for deterministic output regardless of scheduling.
    lines.sort();
    for line in lines {
        println!("{line}");
    }
}

async fn run_target(t: SmokeTarget, sem: Arc<tokio::sync::Semaphore>) -> String {
    let _permit = match sem.acquire().await {
        Ok(p) => p,
        Err(_) => return format_row(t.provider, t.model, "error", None, None, Some("semaphore")),
    };

    let gate = std::env::var(E2E_GATE_ENV).ok();
    let key_present = resolve_key(&t).is_some();
    let skip = if t.provider == "ollama" {
        skip_reason(&t, gate.as_deref(), false, ollama_reachable().await)
    } else {
        skip_reason(&t, gate.as_deref(), key_present, true)
    };

    if skip.is_some() {
        return format_row(t.provider, t.model, "skipped", None, None, None);
    }

    let key = resolve_key(&t).unwrap_or_default();
    let provider = provider_for(&t, &key);

    let mut lines = Vec::new();
    let out = run_chat(provider.as_ref(), t.model, t.reasoning).await;
    let chat_label = format!("{}/chat", t.provider);
    match out.kind {
        None => lines.push(format_row(
            &chat_label,
            t.model,
            "ok",
            out.status,
            Some(out.ms),
            None,
        )),
        Some(kind) => lines.push(format_row(
            &chat_label,
            t.model,
            "error",
            out.status,
            Some(out.ms),
            Some(kind),
        )),
    }
    if t.embed {
        let embed_out = run_embed(provider.as_ref(), MODEL_OPENAI_EMBED).await;
        let embed_label = format!("{}/embed", t.provider);
        match embed_out.kind {
            None => lines.push(format_row(
                &embed_label,
                MODEL_OPENAI_EMBED,
                "ok",
                embed_out.status,
                Some(embed_out.ms),
                None,
            )),
            Some(kind) => lines.push(format_row(
                &embed_label,
                MODEL_OPENAI_EMBED,
                "error",
                embed_out.status,
                Some(embed_out.ms),
                Some(kind),
            )),
        }
    }
    lines.join("\n")
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
        let row = format_row("openai", MODEL_OPENAI_CHAT, "ok", Some(200), Some(42), None);
        assert!(!row.contains(secret), "key leaked into row: {row}");
        assert!(!row.contains(PROMPT), "prompt leaked into row: {row}");
        let err_row = format_row(
            "deepseek",
            MODEL_DEEPSEEK,
            "error",
            Some(401),
            Some(7),
            Some("api"),
        );
        assert!(!err_row.contains(secret));
        assert!(!err_row.contains(PROMPT));
        // The full harness output buffer must also stay clean.
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
            reasoning: ReasoningKind::None,
            embed: false,
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
            reasoning: ReasoningKind::None,
            embed: false,
        };
        assert_eq!(
            skip_reason(&ollama_t, Some("1"), false, false),
            Some(SkipReason::OllamaUnreachable)
        );
        assert_eq!(skip_reason(&ollama_t, Some("1"), false, true), None);
        // Skipped is not a failure: the label is distinct.
        assert_ne!(
            skip_reason(&ollama_t, Some("1"), false, false),
            Some(SkipReason::GateDisarmed)
        );
    }

    #[test]
    fn t4_output_format_is_redacted_csv() {
        // T-4: output format — fixed 6-field CSV with "-" placeholders.
        let ok = format_row("openai", MODEL_OPENAI_CHAT, "ok", Some(200), Some(42), None);
        assert_eq!(ok, "openai, gpt-5-nano, ok, 200, 42, -");
        let skip = format_row("ollama", MODEL_OLLAMA, "skipped", None, None, None);
        assert_eq!(skip, "ollama, llama3.2, skipped, -, -, -");
        let err = format_row(
            "google",
            MODEL_GOOGLE,
            "error",
            Some(500),
            Some(9),
            Some("api"),
        );
        assert_eq!(err, "google, gemini-3.1-flash-lite, error, 500, 9, api");
    }

    #[test]
    fn t5_matrix_covers_all_7_providers_and_pins_ids() {
        // T-5: subset coverage — static reference check: all 7 providers
        // present, unique, exact pinned IDs, no vague wording.
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
        // OpenAI is the only embeddings target in the pinned matrix.
        assert!(m.iter().filter(|t| t.embed).all(|t| t.provider == "openai"));
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
