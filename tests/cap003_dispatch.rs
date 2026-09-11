//! `CAP-003` 契约测试：**统一入口能力裁决**。
//!
//! 本文件是 `CAP-003` DoD 的机器证明：
//! 1. **Ollama 传 `tools` → `LlmError::Unsupported`**（0.1.3 期是**静默丢弃**）；
//! 2. Ollama `n > 1` → **放行**（只告警，不拒绝）——0.1.3 语义**不变**；
//! 3. **六处手工调用点已上移**（用 grep 式断言钉住，防止有人把裁决再塞回 Provider）；
//! 4. 已支持路径**行为零变化**（不支持的能力才拒绝；工具调用已实现的 Provider 不在入口被拒）。
//!
//! 关键性质：**拒绝发生在任何网络调用之前**——故本文件的测试**不需要网络**，
//! 也不依赖上游可用性（这对 CI 稳定性是必要的）。

use llmrust::providers::capabilities::{
    adjudicate, Capabilities, CapabilityLevel, RequestDemands, Verdict,
};
use llmrust::providers::ollama::OllamaProvider;
use llmrust::providers::Provider;
use llmrust::types::{ChatRequest, Content, ContentPart, Message, Tool};
use llmrust::{LlmError, LmrsClient};

fn ollama_client() -> LmrsClient {
    // 构造即可；**不发起任何请求**（裁决在调用 Provider 之前完成）。
    LmrsClient::new()
}

fn req_with_tools() -> ChatRequest {
    let mut req = ChatRequest::new("m", "hi");
    req.tools = Some(vec![Tool {
        tool_type: "function".to_string(),
        function: llmrust::types::FunctionDef {
            name: "f".to_string(),
            description: Some("d".to_string()),
            parameters: serde_json::json!({"type": "object"}),
        },
    }]);
    req
}

fn ollama_provider() -> OllamaProvider {
    use llmrust::ProviderConfig;
    OllamaProvider::new(ProviderConfig {
        api_key: String::new(),
        base_url: Some("http://127.0.0.1:1".to_string()), // 不可达端口：证明拒绝**早于**网络
        timeout_secs: None,
        custom_headers: None,
    })
}

/// **DoD 招牌**：Ollama 声明 `tool_calling = unsupported`，故带 `tools` 的请求
/// 必须在入口被判 `Unsupported`，而**不是**静默丢弃。
#[tokio::test]
async fn ollama_tools_are_rejected_not_silently_dropped() {
    let client = ollama_client();
    client
        .set_ollama(Some("http://127.0.0.1:1".to_string()))
        .await;

    let err = client
        .chat_with("ollama/llama3", req_with_tools())
        .await
        .expect_err("tools on Ollama must be refused, not silently dropped");

    match err {
        LlmError::Unsupported { feature, message } => {
            assert_eq!(
                feature, "tool_calling",
                "feature must name the capability face"
            );
            assert!(
                message.contains("CAP-003"),
                "message must cite the capability ruling: {message}"
            );
        }
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

/// 流式路径查的是 `tool_calling_stream` 面（两个面分别判定）。
#[tokio::test]
async fn ollama_streaming_tools_are_rejected_on_the_stream_face() {
    let client = ollama_client();
    client
        .set_ollama(Some("http://127.0.0.1:1".to_string()))
        .await;

    // 注意：成功分支是 `BoxStream`（不实现 Debug），故不能用 `expect_err`，
    // 必须用 match —— 这本身也说明"拒绝返回的是 Err 而不是流"。
    match client.stream_with("ollama/llama3", req_with_tools()).await {
        Err(LlmError::Unsupported { feature, .. }) => {
            assert_eq!(feature, "tool_calling_stream")
        }
        Err(_) => panic!("expected Unsupported on the stream face"),
        Ok(_) => panic!("streaming tools on Ollama must be refused, got a stream"),
    }
}

/// **`n > 1` 语义不变**：只告警，**不拒绝**。
///
/// 本测试只能证明"裁决表里没有 Reject"；真实请求会因端口不可达而失败——
/// 那正是我们要的区分：**失败原因是网络，而不是 `Unsupported`**。
#[tokio::test]
async fn ollama_n_greater_than_one_is_not_rejected() {
    let client = ollama_client();
    client
        .set_ollama(Some("http://127.0.0.1:1".to_string()))
        .await;

    let mut req = ChatRequest::new("m", "hi");
    req.n = Some(3);
    let err = client
        .chat_with("ollama/llama3", req)
        .await
        .expect_err("unreachable endpoint must fail");

    assert!(
        !matches!(err, LlmError::Unsupported { .. }),
        "n > 1 must NOT be turned into Unsupported (0.1.3 semantics preserved); got {err:?}"
    );
}

/// **DoD 3**：六处手工调用点已上移 —— 用源码级断言钉住"裁决不在 Provider 内部"。
///
/// 若有人把 `warn_if_unsupported_n` 或等价的手工裁决塞回 Provider，本测试红。
#[test]
fn capability_adjudication_lives_at_the_entry_point_only() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let providers_dir = root.join("src").join("providers");

    // 只扫**Provider 实现文件**：`capabilities.rs` 是裁决函数与 `warn_once` 的**定义所在**
    // （它当然含 `warn_once(`），`mod.rs` 是模块根 —— 把二者算作违规是**假阳性**。
    // 铁律：扫描域必须恰好是"不该再出现裁决的地方"。
    const PROVIDER_IMPLS: &[&str] = &[
        "anthropic.rs",
        "compat.rs",
        "deepseek.rs",
        "google.rs",
        "moonshot.rs",
        "ollama.rs",
        "openai.rs",
        "openrouter.rs",
    ];

    let mut offenders = Vec::new();
    for entry in std::fs::read_dir(&providers_dir).expect("providers dir") {
        let path = entry.expect("entry").path();
        if path.extension().is_some_and(|e| e == "rs") {
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            if !PROVIDER_IMPLS.contains(&name.as_str()) {
                continue;
            }
            let src = std::fs::read_to_string(&path).unwrap_or_default();
            // 只允许在注释里提到名字；真正的**调用**必须已迁走。
            for (idx, line) in src.lines().enumerate() {
                let t = line.trim_start();
                if t.starts_with("//") || t.starts_with("///") || t.starts_with("//!") {
                    continue;
                }
                if t.contains("warn_if_unsupported_n(") || t.contains("warn_once(") {
                    offenders.push(format!("{name}:{}: {}", idx + 1, line.trim()));
                }
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "capability adjudication must live at the unified entry point (CAP-003), \
         but these provider-internal call sites remain:\n{}",
        offenders.join("\n")
    );
}

/// **DoD 4**：已支持路径**行为零变化** —— 纯裁决层面，工具调用已实现的 Provider
/// 带 `tools` 时不产生任何 Reject。
#[test]
fn tools_pass_cleanly_when_the_capability_is_implemented() {
    let mut caps = Capabilities::unknown("openai-compatible");
    caps.tool_calling = llmrust::providers::capabilities::Capability::implemented();
    caps.tool_calling_stream = llmrust::providers::capabilities::Capability::implemented();

    let verdicts = adjudicate(
        &caps,
        &RequestDemands {
            tools: true,
            streaming: false,
            n: Some(1),
            images: false,
        },
    );
    assert!(
        verdicts
            .iter()
            .all(|v| !matches!(v, Verdict::Reject { .. })),
        "implemented tool calling must not be rejected: {verdicts:?}"
    );
}

/// 图像诉求同样走声明：Ollama 声明 `image_input = unsupported` → 带图即拒。
#[tokio::test]
async fn ollama_images_are_rejected() {
    let client = ollama_client();
    client
        .set_ollama(Some("http://127.0.0.1:1".to_string()))
        .await;

    let mut req = ChatRequest::new("m", "hi");
    req.messages = vec![Message {
        role: llmrust::types::Role::User,
        content: Content::Parts(vec![ContentPart::image_url("data:image/png;base64,AAAA")]),
        tool_calls: None,
        tool_call_id: None,
        name: None,
    }];

    let err = client
        .chat_with("ollama/llava", req)
        .await
        .expect_err("images on Ollama must be refused");
    match err {
        LlmError::Unsupported { feature, .. } => assert_eq!(feature, "image_input"),
        _ => panic!("expected Unsupported for images"),
    }
}

/// **回归防线（本卡开发中真实踩到的坑）**：**未声明能力的 Provider 不得被拒绝**。
///
/// 背景：`CAP-003` 首版实现把"未声明"与"声明为 unsupported"一视同仁，
/// 于是 `proxy` 的三个既有测试立刻红了（400 而非 200）——代理内部用的是**自定义 mock
/// Provider**，它没有覆写 `capabilities()`。若不放行，`CAP-003` 就成了"给所有下游
/// 自定义 Provider 加锁"，违反卡内"已支持路径行为零变化"。
///
/// 本测试把这条**钉死**：未声明 → `Warn`（放行 + 留痕），**绝不** `Reject`。
#[test]
fn undeclared_provider_is_warned_not_rejected() {
    let caps = Capabilities::unknown("custom"); // declared == false
    assert!(
        !caps.declared,
        "the default declaration must be marked undeclared"
    );

    let verdicts = adjudicate(
        &caps,
        &RequestDemands {
            tools: true,
            streaming: false,
            n: None,
            images: true,
        },
    );
    assert!(
        verdicts
            .iter()
            .all(|v| !matches!(v, Verdict::Reject { .. })),
        "undeclared capabilities must never cause a rejection: {verdicts:?}"
    );
    assert_eq!(
        verdicts
            .iter()
            .filter(|v| matches!(v, Verdict::Warn { .. }))
            .count(),
        2,
        "both tools and images should warn once each: {verdicts:?}"
    );
}

/// 对照：**明确声明为 unsupported** 的 Provider 才会被拒（这正是 Ollama 的情形）。
#[test]
fn declared_unsupported_is_the_only_reject_path() {
    let mut caps = Capabilities::unknown("declared-provider");
    caps.declared = true;
    let verdicts = adjudicate(
        &caps,
        &RequestDemands {
            tools: true,
            streaming: false,
            n: None,
            images: false,
        },
    );
    assert_eq!(verdicts.len(), 1, "got {verdicts:?}");
    assert!(
        matches!(
            verdicts[0],
            Verdict::Reject {
                feature: "tool_calling",
                ..
            }
        ),
        "{verdicts:?}"
    );
}

/// 声明层面的对照：Ollama 的能力声明确实把这三项标为 `unsupported`
/// （保证上面的拒绝不是偶然，而是**有声明依据**）。
#[test]
fn ollama_declaration_is_the_basis_for_the_rejections() {
    let caps = ollama_provider().capabilities();
    assert_eq!(caps.tool_calling.level, CapabilityLevel::Unsupported);
    assert_eq!(caps.tool_calling_stream.level, CapabilityLevel::Unsupported);
    assert_eq!(caps.image_input.level, CapabilityLevel::Unsupported);
    // 反面对照：chat/stream 必须**不是** unsupported（否则上面的拒绝就成了"全面拒绝"）。
    assert_ne!(caps.chat.level, CapabilityLevel::Unsupported);
    assert_ne!(caps.stream.level, CapabilityLevel::Unsupported);
}
