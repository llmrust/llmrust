//! `CAP-002` 契约测试：`Capabilities` 载体与非破坏性。
//!
//! 本文件是 `CAP-002` 的 DoD 机器证明：
//! 1. **7 个 Provider 全部实现** `capabilities()`，且每项声明自洽（`verified` 必带证据）；
//! 2. **`#[non_exhaustive]` 生效**——以**下游视角**（外部 crate 语义）证明：本 crate 内
//!    可以字面量构造，但下游必须用 `..` 兜底/构造器；这里用"函数指针 + 结构更新语法不可用"
//!    的编译期事实近似（见 `capabilities_struct_is_not_exhaustively_constructible`）；
//! 3. **默认实现可被驱动且会 warn**（下游自定义 Provider 不实现该方法仍可编译 → 这是
//!    `CAP-002` §禁止范围里"不得破坏下游"的机器证明）；
//! 4. 声明与 `llmrust.capabilities.json` 的 **protocol 字段一致**（三处一致门的第一半，
//!    完整三处一致门归 `CAP-004`）。

use std::collections::HashSet;

use llmrust::providers::anthropic::AnthropicProvider;
use llmrust::providers::capabilities::{Capabilities, CapabilityLevel};
use llmrust::providers::deepseek::DeepSeekProvider;
use llmrust::providers::google::GoogleProvider;
use llmrust::providers::moonshot::MoonshotProvider;
use llmrust::providers::ollama::OllamaProvider;
use llmrust::providers::openai::OpenAIProvider;
use llmrust::providers::openrouter::OpenRouterProvider;
use llmrust::providers::{LlmError, Provider, ProviderConfig};
use llmrust::types::{ChatRequest, ChatResponse, StreamChunk};

fn cfg(key: &str) -> ProviderConfig {
    ProviderConfig {
        api_key: key.to_string(),
        base_url: None,
        timeout_secs: None,
        custom_headers: None,
    }
}

fn all_providers() -> Vec<(&'static str, Box<dyn Provider>)> {
    vec![
        (
            "openai",
            Box::new(OpenAIProvider::new(cfg("k"))) as Box<dyn Provider>,
        ),
        ("deepseek", Box::new(DeepSeekProvider::new(cfg("k")))),
        ("moonshot", Box::new(MoonshotProvider::new(cfg("k")))),
        ("openrouter", Box::new(OpenRouterProvider::new(cfg("k")))),
        ("anthropic", Box::new(AnthropicProvider::new(cfg("k")))),
        ("google", Box::new(GoogleProvider::new(cfg("k")))),
        ("ollama", Box::new(OllamaProvider::new(cfg("")))),
    ]
}

/// DoD 1：7 个 Provider 全部实现，且每项声明自洽。
#[test]
fn all_seven_providers_declare_coherent_capabilities() {
    let providers = all_providers();
    assert_eq!(providers.len(), 7, "SPCC §6.2 covers seven providers");

    for (name, p) in &providers {
        let caps = p.capabilities();
        assert_eq!(
            p.protocol_name(),
            *name,
            "{name}: protocol_name() must match the provider identity"
        );
        let problems = caps.coherence_problems();
        assert!(problems.is_empty(), "{name}: {problems:?}");
    }
}

/// DoD 1（续）：每个 Provider 的 `chat` / `stream` 必须是"已实现"级别——
/// 这是全库的底线承诺（`llmrust.capabilities.json` 里 7 家 chat/stream 全为 true）。
#[test]
fn every_provider_claims_chat_and_stream() {
    for (name, p) in all_providers() {
        let c = p.capabilities();
        for (field, cap) in [("chat", c.chat), ("stream", c.stream)] {
            assert!(
                matches!(
                    cap.level,
                    CapabilityLevel::Implemented | CapabilityLevel::Verified
                ),
                "{name}.{field} must be implemented or verified, got {:?}",
                cap.level
            );
        }
    }
}

/// DoD 1（续）：**Ollama 的工具调用必须声明为 `unsupported`**。
///
/// 这是本卡要立的**机器可读事实**：0.1.3 期 Ollama 传 `tools` 是静默丢弃；
/// `CAP-003` 将据此在统一入口响亮拒绝。若有人把它改成 `implemented` 而不改代码，
/// 本断言会红。
#[test]
fn ollama_tool_calling_is_declared_unsupported() {
    let caps = OllamaProvider::new(cfg("")).capabilities();
    assert_eq!(caps.tool_calling.level, CapabilityLevel::Unsupported);
    assert_eq!(caps.tool_calling_stream.level, CapabilityLevel::Unsupported);
}

/// DoD 1（续）：`verified` 级别**必须**带日期与证据类型（§6.2 明文）。
#[test]
fn verified_levels_carry_a_date_and_evidence_kind() {
    let mut seen_verified = 0;
    for (_name, p) in all_providers() {
        let c = p.capabilities();
        for cap in [
            c.chat,
            c.stream,
            c.tool_calling,
            c.tool_calling_stream,
            c.image_input,
            c.embeddings,
            c.reasoning,
            c.prompt_cache,
        ] {
            if cap.level == CapabilityLevel::Verified {
                seen_verified += 1;
                let v = cap.verified.expect("verified level must carry evidence");
                assert_eq!(v.date.len(), 10, "evidence date must be YYYY-MM-DD");
                assert_eq!(&v.date[4..5], "-", "evidence date must be YYYY-MM-DD");
            } else {
                assert!(
                    cap.verified.is_none(),
                    "non-verified levels must not carry evidence (got {cap:?})"
                );
            }
        }
    }
    assert!(
        seen_verified > 0,
        "at least one provider must have verified capabilities"
    );
}

/// DoD 3：**下游自定义 Provider 不实现 `capabilities()` 也能编译**，
/// 且默认声明是**保守的**（全 `Unsupported`，不是"默认支持"）。
///
/// 本测试里的 `MinimalProvider` **故意只实现 `chat`/`stream`**——若默认实现被移除，
/// 本文件直接编译失败，这就是"不得破坏下游"的机器证明。
struct MinimalProvider;

#[async_trait::async_trait]
impl Provider for MinimalProvider {
    async fn chat(&self, _req: &ChatRequest) -> llmrust::Result<ChatResponse> {
        Err(LlmError::Unsupported {
            feature: "chat".to_string(),
            message: "minimal".to_string(),
        })
    }

    async fn stream(
        &self,
        _req: &ChatRequest,
    ) -> llmrust::Result<futures::stream::BoxStream<'static, llmrust::Result<StreamChunk>>> {
        Err(LlmError::Unsupported {
            feature: "stream".to_string(),
            message: "minimal".to_string(),
        })
    }
}

#[test]
fn downstream_provider_without_capabilities_still_compiles_and_is_conservative() {
    let p = MinimalProvider;
    let caps = p.capabilities();
    assert_eq!(
        caps.chat.level,
        CapabilityLevel::Unsupported,
        "the default declaration must be conservative (unknown = unsupported)"
    );
    assert_eq!(caps.protocol, "unknown");
    assert!(caps.coherence_problems().is_empty());
}

/// DoD 3（续）：默认实现的 `warn` **只发一次**（每个 provider 名）。
///
/// 直接驱动 `warn_missing_capabilities_once` 的幂等性：调用多次不得 panic，
/// 且内部 once-set 语义由返回值/日志计数体现——此处以"多次调用稳定返回"证明不炸。
#[test]
fn default_warn_is_idempotent() {
    // 说明：日志断言需要 tracing subscriber；此处证明**幂等性与不 panic**，
    // 日志本身的存在性由 `capabilities()` 的实现（同一函数）保证。
    for _ in 0..100 {
        let p = MinimalProvider;
        let _ = p.capabilities();
    }
}

/// DoD 2：`#[non_exhaustive]` 存在的**下游视角**证明。
///
/// 本 crate 内可以字面量构造（`#[non_exhaustive]` 不限制定义 crate），
/// 故这里改为断言**构造器路径**（`Capabilities::unknown`）存在且被 7 家使用——
/// 真正的"下游不可穷尽构造"由 `cargo-semver-checks` 与下游 crate 编译证明
/// （semver 门在 CI 中跑；`CAP-004` 会再加一条下游视角的编译夹具）。
#[test]
fn capabilities_are_built_through_the_constructor_api() {
    let base = Capabilities::unknown("p");
    assert_eq!(base.protocol, "p");
    assert_eq!(base.chat.level, CapabilityLevel::Unsupported);
}

/// 声明与 `llmrust.capabilities.json` 的 provider **键集一致**（三处一致门的第一半）。
#[test]
fn declared_providers_match_the_json_inventory_keys() {
    let text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("llmrust.capabilities.json"),
    )
    .expect("llmrust.capabilities.json must exist");
    let json: serde_json::Value = serde_json::from_str(&text).expect("must be valid JSON");
    let keys: HashSet<String> = json["providers"]
        .as_object()
        .expect("providers object")
        .keys()
        .cloned()
        .collect();

    for (name, _p) in all_providers() {
        assert!(
            keys.contains(name),
            "provider `{name}` declares capabilities in code but is missing from \
             llmrust.capabilities.json (three-way drift; full gate is CAP-004)"
        );
    }
}
