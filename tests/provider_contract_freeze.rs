//! Provider / error / Retry contract freeze tests for 0.1.3 (API-003).
//!
//! These pin the *behavior* shipped in 0.1.3 so that any later shape change
//! (Provider trait, `LlmError`, `RetryProvider` policy, embed default, client
//! delegation) breaks CI instead of silently drifting. They are additive and
//! do not alter any runtime behavior.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use futures::stream::{self, BoxStream};

use llmrust::providers::{LlmError, Provider, Result};
use llmrust::types::{ChatRequest, ChatResponse, EmbeddingRequest, EmbeddingResponse, StreamChunk};

// ── Mock providers ─────────────────────────────────────────────────────────

struct FlakyProvider {
    attempts: Arc<AtomicU32>,
    max_fails: u32,
    status: u16,
}

impl FlakyProvider {
    fn new(max_fails: u32, status: u16) -> Arc<Self> {
        Arc::new(Self {
            attempts: Arc::new(AtomicU32::new(0)),
            max_fails,
            status,
        })
    }
    fn call_count(&self) -> u32 {
        self.attempts.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Provider for FlakyProvider {
    async fn chat(&self, _req: &ChatRequest) -> Result<ChatResponse> {
        let n = self.attempts.fetch_add(1, Ordering::SeqCst);
        if n < self.max_fails {
            return Err(LlmError::Api {
                status: self.status,
                message: "transient".into(),
            });
        }
        Ok(ChatResponse {
            content: "ok".into(),
            model: "test".into(),
            ..Default::default()
        })
    }
    async fn stream(&self, _req: &ChatRequest) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        let n = self.attempts.fetch_add(1, Ordering::SeqCst);
        if n < self.max_fails {
            return Err(LlmError::Api {
                status: self.status,
                message: "transient".into(),
            });
        }
        Ok(Box::pin(stream::once(async {
            Ok(StreamChunk {
                delta: "ok".into(),
                done: true,
                ..Default::default()
            })
        })))
    }
    // embed() uses the trait default (Unsupported) — exercised by IT-1.
}

struct EmbedOkProvider;
#[async_trait]
impl Provider for EmbedOkProvider {
    async fn chat(&self, _: &ChatRequest) -> Result<ChatResponse> {
        Ok(ChatResponse::default())
    }
    async fn stream(&self, _: &ChatRequest) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        Ok(Box::pin(stream::empty()))
    }
    async fn embed(&self, req: &EmbeddingRequest) -> Result<EmbeddingResponse> {
        Ok(EmbeddingResponse {
            model: req.model.clone(),
            data: vec![llmrust::Embedding {
                index: 0,
                embedding: vec![0.1],
            }],
            usage: None,
        })
    }
}

struct EmbedUnsupportedProvider;
#[async_trait]
impl Provider for EmbedUnsupportedProvider {
    async fn chat(&self, _: &ChatRequest) -> Result<ChatResponse> {
        Ok(ChatResponse::default())
    }
    async fn stream(&self, _: &ChatRequest) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        Ok(Box::pin(stream::empty()))
    }
    // embed() default Unsupported — exercised by IT-4b.
}

fn req() -> ChatRequest {
    ChatRequest::new("test", "hello")
}

// ── IT-1: embed default is Unsupported ─────────────────────────────────────

#[tokio::test]
async fn it1_embed_default_unsupported() {
    let p = FlakyProvider::new(0, 400);
    let err = p
        .embed(&EmbeddingRequest::new("test", "hello"))
        .await
        .unwrap_err();
    match err {
        LlmError::Unsupported { feature, .. } => assert_eq!(feature, "embeddings"),
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

// ── IT-2: LlmError variant surface ─────────────────────────────────────────

#[tokio::test]
async fn it2_llm_error_variants() {
    // `Api` exposes `status` (used by retry policy / proxy mapping).
    let LlmError::Api { status, .. } = (LlmError::Api {
        status: 429,
        message: "rl".into(),
    }) else {
        unreachable!()
    };
    assert_eq!(status, 429);

    // The other constructible variants exist with their documented shapes.
    let _s = LlmError::Stream("x".into());
    let _p = LlmError::Parse("bad".into());
    let _u = LlmError::UnknownProvider("p".into());
    let LlmError::Unsupported { feature, .. } = (LlmError::Unsupported {
        feature: "f".into(),
        message: "m".into(),
    }) else {
        unreachable!()
    };
    assert_eq!(feature, "f");

    // `Http` is `#[from] reqwest::Error` with no public constructor; it is
    // covered by real-network tests elsewhere.
}

// ── IT-3: RetryProvider retry policy ───────────────────────────────────────

#[tokio::test]
async fn it3a_retries_5xx() {
    let inner = FlakyProvider::new(2, 502);
    let retry = llmrust::RetryProvider::new(inner, 3);
    let resp = retry.chat(&req()).await.unwrap();
    assert_eq!(resp.content, "ok");
}

#[tokio::test]
async fn it3b_no_retry_4xx() {
    let inner = FlakyProvider::new(10, 400);
    let retry = llmrust::RetryProvider::new(inner.clone(), 3);
    let err = retry.chat(&req()).await.unwrap_err();
    assert!(matches!(err, LlmError::Api { status: 400, .. }));
    assert_eq!(inner.call_count(), 1, "4xx must not be retried");
}

#[tokio::test]
async fn it3c_no_retry_429() {
    let inner = FlakyProvider::new(10, 429);
    let retry = llmrust::RetryProvider::new(inner.clone(), 3);
    let err = retry.chat(&req()).await.unwrap_err();
    assert!(matches!(err, LlmError::Api { status: 429, .. }));
    assert_eq!(
        inner.call_count(),
        1,
        "429 must not be retried by RetryProvider"
    );
}

#[tokio::test]
async fn it3d_respects_max_retries() {
    let inner = FlakyProvider::new(10, 502);
    let retry = llmrust::RetryProvider::new(inner.clone(), 2); // 2 retries => 3 attempts
    let err = retry.chat(&req()).await.unwrap_err();
    assert!(matches!(err, LlmError::Api { status: 502, .. }));
    assert_eq!(inner.call_count(), 3, "must stop after max_retries");
}

// ── IT-4: embed delegation through RetryProvider ───────────────────────────

#[tokio::test]
async fn it4a_embed_delegates() {
    let inner = Arc::new(EmbedOkProvider);
    let retry = llmrust::RetryProvider::new(inner, 0);
    let resp = retry
        .embed(&EmbeddingRequest::new("test", "hello"))
        .await
        .unwrap();
    assert_eq!(resp.data.len(), 1);
}

#[tokio::test]
async fn it4b_embed_unsupported_passthrough() {
    let inner = Arc::new(EmbedUnsupportedProvider);
    let retry = llmrust::RetryProvider::new(inner, 0);
    let err = retry
        .embed(&EmbeddingRequest::new("test", "hello"))
        .await
        .unwrap_err();
    assert!(matches!(err, LlmError::Unsupported { .. }));
}

// ── IT-5: LmrsClient delegation API surface (compile-time freeze) ──────────

#[tokio::test]
async fn it5_client_delegation_api_present() {
    // Runtime freeze: every LmrsClient delegation entry point used by 0.1.3
    // must exist and be callable. These only store provider config — no network.
    let client = llmrust::LmrsClient::new();
    client.set_openai("sk-test").await;
    client
        .set_openai_compatible("sk-test", "https://api.example.com/v1")
        .await;
    client.set_anthropic("sk-test").await;
    client.set_deepseek("sk-test").await;
    client.set_google("sk-test").await;
    client
        .set_ollama(Some("http://localhost:11434".into()))
        .await;
    client.set_moonshot("sk-test").await;
    client.set_openrouter("sk-test").await;
    let custom: std::sync::Arc<dyn llmrust::providers::Provider> =
        std::sync::Arc::new(EmbedUnsupportedProvider);
    client.set_custom("custom", custom).await;
    // from_env() reads env and returns a client (no panic when vars absent).
    let _from_env = llmrust::LmrsClient::from_env().await;
}

// ── GRD-003 ②：门自身必须会红 ──────────────────────────────────────────
//
// 本文件此前 9 条测试全是"跑一遍、断言结果"的正向测试：它们能证明**今天**策略正确，
// 但**不能证明"策略被改坏会被发现"**。下面把被钉住的 retry 策略抽成纯函数
// [`retry_policy_problems`]，再用**真实观测**与**人为违约样本**双向驱动。

/// 一次观测到的重试行为：`(http status, 实际尝试次数, 配置的最大重试次数)`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryObservation {
    /// 上游返回的 HTTP 状态码。
    pub status: u16,
    /// 内层 provider 被调用的次数（1 = 未重试）。
    pub attempts: u32,
    /// `RetryProvider::new(inner, max_retries)` 的配置值。
    pub max_retries: u32,
}

/// **纯函数**：观测相对 0.1.3 钉住的 RetryProvider 策略的违规（空 = 合规）。
///
/// 策略（与 `it3a/it3b/it3c` 同源）：
/// - **5xx**：可重试，最多 `max_retries` 次重试 ⇒ `attempts <= max_retries + 1`；
/// - **4xx / 429**：**不重试** ⇒ `attempts == 1`（这是 ACL/计费安全的关键：
///   4xx 重试会重复计费且掩盖客户端错误）。
pub fn retry_policy_problems(obs: &[RetryObservation]) -> Vec<String> {
    let mut out = Vec::new();
    for o in obs {
        if (400..500).contains(&o.status) {
            if o.attempts != 1 {
                out.push(format!(
                    "status {}: attempts = {} but 4xx/429 must NOT be retried (expected 1)",
                    o.status, o.attempts
                ));
            }
        } else if ((500..600).contains(&o.status) || o.status == 0)
            && o.attempts > o.max_retries + 1
        {
            out.push(format!(
                "status {}: attempts = {} exceeds max_retries + 1 = {}",
                o.status,
                o.attempts,
                o.max_retries + 1
            ));
        }
        if o.attempts == 0 {
            out.push(format!(
                "status {}: attempts = 0 (provider was never called)",
                o.status
            ));
        }
    }
    out
}

/// 负例一：**4xx 被重试**（策略违约，且会重复计费）→ 必须报。
#[test]
fn negative_retried_4xx_is_reported() {
    let problems = retry_policy_problems(&[RetryObservation {
        status: 400,
        attempts: 3,
        max_retries: 3,
    }]);
    assert_eq!(problems.len(), 1, "got {problems:?}");
    assert!(problems[0].contains("must NOT be retried"), "{problems:?}");
}

/// 负例二：**429 被重试** → 必须报（429 与 4xx 同策）。
#[test]
fn negative_retried_429_is_reported() {
    let problems = retry_policy_problems(&[RetryObservation {
        status: 429,
        attempts: 2,
        max_retries: 3,
    }]);
    assert_eq!(problems.len(), 1, "got {problems:?}");
}

/// 负例三：**5xx 重试次数超上限** → 必须报。
#[test]
fn negative_5xx_over_retry_budget_is_reported() {
    let problems = retry_policy_problems(&[RetryObservation {
        status: 502,
        attempts: 9,
        max_retries: 3,
    }]);
    assert_eq!(problems.len(), 1, "got {problems:?}");
    assert!(problems[0].contains("exceeds max_retries"), "{problems:?}");
}

/// 负例四：`attempts == 0`（provider 根本没被调用）→ 必须报（否则"没调用"会被当成合规）。
#[test]
fn negative_zero_attempts_is_reported() {
    let problems = retry_policy_problems(&[RetryObservation {
        status: 200,
        attempts: 0,
        max_retries: 3,
    }]);
    assert_eq!(problems.len(), 1, "got {problems:?}");
    assert!(problems[0].contains("never called"), "{problems:?}");
}

/// **正向对照 + 与真实行为的对账**：真跑一遍 `RetryProvider`，把**实测观测值**
/// 喂给同一个纯函数 —— 必须零违规（证明线上的确遵守被钉住的策略，且抽出的判据
/// 与线上行为是同一口径，不是自说自话）。
#[tokio::test]
async fn real_retry_behaviour_satisfies_the_extracted_policy() {
    // 4xx/429：不得重试
    for status in [400u16, 401, 403, 429] {
        let inner = FlakyProvider::new(10, status);
        let retry = llmrust::RetryProvider::new(inner.clone(), 3);
        let _ = retry.chat(&req()).await;
        let obs = RetryObservation {
            status,
            attempts: inner.call_count(),
            max_retries: 3,
        };
        let problems = retry_policy_problems(&[obs]);
        assert!(
            problems.is_empty(),
            "real provider violated the pinned policy: {problems:?} (obs = {obs:?})"
        );
    }

    // 5xx：允许重试，但不得超预算（这里 3 次失败 + 1 次成功 = 4 次尝试，预算 3+1）
    let inner = FlakyProvider::new(3, 502);
    let retry = llmrust::RetryProvider::new(inner.clone(), 3);
    let resp = retry.chat(&req()).await.expect("must eventually succeed");
    assert_eq!(resp.content, "ok");
    let obs = RetryObservation {
        status: 502,
        attempts: inner.call_count(),
        max_retries: 3,
    };
    let problems = retry_policy_problems(&[obs]);
    assert!(
        problems.is_empty(),
        "5xx retry budget violated: {problems:?} (obs = {obs:?})"
    );
}

// ── B-018-HTTP: local transport / request-scope audit ───────────────────────

mod audit_2026_http_tests {
    use super::*;
    use futures::StreamExt;
    use llmrust::providers::anthropic::AnthropicProvider;
    use llmrust::providers::compat::OpenAiCompatibleProvider;
    use llmrust::providers::google::GoogleProvider;
    use llmrust::providers::ollama::OllamaProvider;
    use llmrust::providers::openai::OpenAIProvider;
    use llmrust::providers::retry::parse_retry_after;
    use llmrust::{CapabilityLevel, ProviderConfig};
    use std::io::{ErrorKind, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::AtomicUsize;
    use std::sync::{Barrier, Mutex};
    use std::time::{Duration, Instant};

    #[derive(Debug, Clone)]
    pub(super) struct CapturedRequest {
        pub(super) target: String,
        pub(super) headers: String,
        pub(super) body: String,
    }

    pub(super) type ResponseSpec = (u16, Vec<(String, String)>, String);

    pub(super) fn read_request(stream: &mut TcpStream) -> std::io::Result<CapturedRequest> {
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        let mut bytes = Vec::new();
        let header_end = loop {
            if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                break index + 4;
            }
            let mut buffer = [0_u8; 4096];
            let count = stream.read(&mut buffer)?;
            if count == 0 {
                return Err(std::io::Error::new(
                    ErrorKind::UnexpectedEof,
                    "request headers ended early",
                ));
            }
            bytes.extend_from_slice(&buffer[..count]);
        };
        let headers = String::from_utf8_lossy(&bytes[..header_end]).to_string();
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().unwrap_or(0))
            })
            .unwrap_or(0);
        while bytes.len() < header_end + content_length {
            let mut buffer = [0_u8; 4096];
            let count = stream.read(&mut buffer)?;
            if count == 0 {
                return Err(std::io::Error::new(
                    ErrorKind::UnexpectedEof,
                    "request body ended early",
                ));
            }
            bytes.extend_from_slice(&buffer[..count]);
        }
        let request_line = headers.lines().next().unwrap_or_default();
        let target = request_line
            .split_whitespace()
            .nth(1)
            .unwrap_or_default()
            .to_string();
        Ok(CapturedRequest {
            target,
            headers,
            body: String::from_utf8_lossy(&bytes[header_end..header_end + content_length])
                .to_string(),
        })
    }

    pub(super) fn write_response(
        stream: &mut TcpStream,
        status: u16,
        headers: &[(String, String)],
        body: &str,
    ) {
        let reason = match status {
            200 => "OK",
            307 => "Temporary Redirect",
            503 => "Service Unavailable",
            _ => "Test",
        };
        let mut response = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        );
        for (name, value) in headers {
            response.push_str(name);
            response.push_str(": ");
            response.push_str(value);
            response.push_str("\r\n");
        }
        response.push_str("\r\n");
        response.push_str(body);
        stream.write_all(response.as_bytes()).ok();
        stream.flush().ok();
    }

    pub(super) fn serve_n<H>(
        listener: TcpListener,
        count: usize,
        mut handler: H,
    ) -> std::thread::JoinHandle<Vec<CapturedRequest>>
    where
        H: FnMut(CapturedRequest) -> Option<ResponseSpec> + Send + 'static,
    {
        std::thread::spawn(move || {
            let mut captured = Vec::with_capacity(count);
            for _ in 0..count {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let Ok(request) = read_request(&mut stream) else {
                    continue;
                };
                captured.push(request.clone());
                if let Some((status, headers, body)) = handler(request) {
                    write_response(&mut stream, status, &headers, &body);
                }
            }
            captured
        })
    }

    fn serve_concurrent_n<H>(
        listener: TcpListener,
        count: usize,
        handler: H,
    ) -> std::thread::JoinHandle<Vec<CapturedRequest>>
    where
        H: Fn(CapturedRequest) -> Option<ResponseSpec> + Send + Sync + 'static,
    {
        std::thread::spawn(move || {
            let handler = Arc::new(handler);
            let captured = Arc::new(Mutex::new(Vec::with_capacity(count)));
            let mut workers = Vec::with_capacity(count);
            for _ in 0..count {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let handler = Arc::clone(&handler);
                let captured = Arc::clone(&captured);
                workers.push(std::thread::spawn(move || {
                    let Ok(request) = read_request(&mut stream) else {
                        return;
                    };
                    captured.lock().unwrap().push(request.clone());
                    if let Some((status, headers, body)) = handler(request) {
                        write_response(&mut stream, status, &headers, &body);
                    }
                }));
            }
            for worker in workers {
                worker.join().unwrap();
            }
            let captured = captured.lock().unwrap().clone();
            captured
        })
    }

    fn openai_chat_response() -> String {
        r#"{"model":"test","choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}"#.into()
    }

    #[tokio::test]
    async fn audit_2026_http_error_redacts_url_and_userinfo() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let url = format!(
            "http://alice:topsecret@{}/private?api_key=hidden#fragment",
            address
        );
        let error = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(&url)
            .send()
            .await
            .unwrap_err();
        assert!(
            error.url().is_some(),
            "fixture must start with a located reqwest error"
        );
        let converted: LlmError = error.into();
        let LlmError::Http(error) = &converted else {
            panic!("expected an HTTP error");
        };
        assert!(
            error.url().is_none(),
            "without_url must clear the located URL"
        );
        let shown = format!("{converted:?}");
        for secret in [&url, "alice", "topsecret", "private", "hidden", "fragment"] {
            assert!(!shown.contains(secret), "error leaked {secret}: {shown}");
        }

        let provider = OpenAiCompatibleProvider::new(
            ProviderConfig::new("k")
                .with_base_url(format!("http://{address}/v1"))
                .with_timeout_secs(1),
            [],
        );
        let error = provider.chat(&req()).await.unwrap_err();
        assert!(matches!(error, LlmError::Http(_)));
        let shown = error.to_string();
        assert!(
            !shown.contains(&address.to_string()),
            "provider error leaked its URL: {shown}"
        );
    }

    #[tokio::test]
    async fn audit_2026_http_cross_origin_redirect_replays_nothing() {
        let target_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let target_address = target_listener.local_addr().unwrap();
        target_listener.set_nonblocking(true).unwrap();
        let target = std::thread::spawn(move || {
            let mut captured = Vec::new();
            let deadline = Instant::now() + Duration::from_millis(600);
            while Instant::now() < deadline {
                match target_listener.accept() {
                    Ok((mut stream, _)) => {
                        if let Ok(request) = read_request(&mut stream) {
                            captured.push(request);
                            write_response(&mut stream, 200, &[], &openai_chat_response());
                        }
                        break;
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
            captured
        });

        let origin_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin_address = origin_listener.local_addr().unwrap();
        let origin = serve_n(origin_listener, 1, move |_request| {
            Some((
                307,
                vec![(
                    "Location".into(),
                    format!("http://{target_address}/replayed"),
                )],
                String::new(),
            ))
        });
        let provider = OpenAIProvider::new(
            ProviderConfig::new("credential")
                .with_base_url(format!("http://{origin_address}/v1"))
                .with_header("X-Audit-Secret", "header-sentinel")
                .with_timeout_secs(2),
        );
        let result = provider
            .chat(&ChatRequest::new("test", "body-sentinel"))
            .await;
        let origin_requests = origin.join().unwrap();
        let target_requests = target.join().unwrap();

        assert!(
            target_requests.is_empty(),
            "redirect target received requests: {target_requests:?}"
        );
        assert_eq!(origin_requests.len(), 1);
        assert!(origin_requests[0].body.contains("body-sentinel"));
        assert!(origin_requests[0]
            .headers
            .to_ascii_lowercase()
            .contains("header-sentinel"));
        match result {
            Err(LlmError::Api { status, .. }) => assert_eq!(status, 307),
            other => panic!("a cross-origin 307 must remain an error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn audit_2026_http_concurrent_retry_hints_are_request_scoped() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let barrier = Arc::new(Barrier::new(3));
        let sequence = Arc::new(AtomicUsize::new(0));
        let server_barrier = Arc::clone(&barrier);
        let server_sequence = Arc::clone(&sequence);
        let server = serve_concurrent_n(listener, 4, move |request| {
            let index = server_sequence.fetch_add(1, Ordering::SeqCst);
            if index < 3 {
                server_barrier.wait();
            }
            let model = serde_json::from_str::<serde_json::Value>(&request.body)
                .ok()
                .and_then(|body| body["model"].as_str().map(str::to_string))
                .unwrap_or_default();
            match model.as_str() {
                "hinted" => Some((
                    503,
                    vec![("Retry-After".into(), "60".into())],
                    r#"{"error":{"message":"busy"}}"#.into(),
                )),
                "no-hint" => Some((503, Vec::new(), r#"{"error":{"message":"busy"}}"#.into())),
                "network-error" => {
                    // The connection was already read by the fixture. Closing it
                    // without a response produces a transport error for this call.
                    None
                }
                "success" => Some((200, Vec::new(), openai_chat_response())),
                other => panic!("unexpected model {other}"),
            }
        });
        // serve_n owns the stream, so a None response must really close it.
        // Replace the helper's behavior by using a dedicated shutdown in the
        // handler is not possible through ResponseSpec; the loop drops it on None.
        let provider = OpenAiCompatibleProvider::new(
            ProviderConfig::new("k")
                .with_base_url(format!("http://{address}/v1"))
                .with_timeout_secs(2),
            [],
        );
        let hinted_request = ChatRequest::new("hinted", "one");
        let no_hint_request = ChatRequest::new("no-hint", "two");
        let network_request = ChatRequest::new("network-error", "three");
        let (hinted, no_hint, network_error) = tokio::join!(
            provider.chat_with_retry_after(&hinted_request),
            provider.chat_with_retry_after(&no_hint_request),
            provider.chat_with_retry_after(&network_request),
        );
        let success = provider
            .chat_with_retry_after(&ChatRequest::new("success", "four"))
            .await;
        let captured = server.join().unwrap();
        assert!(
            matches!(hinted.0, Err(LlmError::Api { status: 503, .. })),
            "hinted call returned {hinted:?}; captured={captured:?}"
        );
        assert_eq!(hinted.1, Some(Duration::from_secs(60)));
        assert!(matches!(no_hint.0, Err(LlmError::Api { status: 503, .. })));
        assert_eq!(no_hint.1, None);
        assert!(matches!(network_error.0, Err(LlmError::Http(_))));
        assert_eq!(
            network_error.1, None,
            "a network error must not reuse another request's hint"
        );
        assert_eq!(
            provider.last_retry_after(),
            None,
            "no instance hint slot may remain"
        );
        assert!(success.0.is_ok());
        assert_eq!(
            success.1, None,
            "a successful call must carry no stale hint"
        );
        assert_eq!(captured.len(), 4);
    }

    #[tokio::test]
    async fn audit_2026_http_embedding_honors_retry_after() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let attempts = Arc::new(AtomicUsize::new(0));
        let server_attempts = Arc::clone(&attempts);
        let server = serve_n(listener, 2, move |_request| {
            if server_attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                Some((
                    503,
                    vec![("Retry-After".into(), "0".into())],
                    r#"{"error":{"message":"busy"}}"#.into(),
                ))
            } else {
                Some((
                    200,
                    Vec::new(),
                    r#"{"model":"m","data":[{"index":0,"embedding":[0.5]}]}"#.into(),
                ))
            }
        });
        let provider = OpenAiCompatibleProvider::new(
            ProviderConfig::new("k").with_base_url(format!("http://{address}/v1")),
            [],
        );
        let retry = llmrust::RetryProvider::new(Arc::new(provider), 1)
            .with_base_delay(30_000)
            .with_max_delay(30_000);
        let started = Instant::now();
        let response = retry
            .embed(&EmbeddingRequest::new("m", "hello"))
            .await
            .unwrap();
        let elapsed = started.elapsed();
        assert_eq!(response.data[0].embedding, vec![0.5]);
        assert!(
            elapsed < Duration::from_secs(5),
            "embedding retry must use the 1ms hint floor, not 30s local backoff: {elapsed:?}"
        );
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        assert_eq!(server.join().unwrap().len(), 2);
    }

    #[test]
    fn audit_2026_http_retry_after_rejects_invalid_calendar_and_signed_seconds() {
        for malformed in [
            "+5",
            "-5",
            "Wed, 31 Feb 2015 07:28:00 GMT",
            "Wed, 29 Feb 2015 07:28:00 GMT",
            "Wed, 31 Apr 2015 07:28:00 GMT",
            "Wed, 21 Oct 2015 07:28:60 GMT",
            "wed, 21 Oct 2015 07:28:00 GMT",
            "Wed, 21 oct 2015 07:28:00 GMT",
        ] {
            assert_eq!(parse_retry_after(malformed, 0), None, "{malformed}");
        }
        let base = 1_445_412_480_i64;
        assert_eq!(
            parse_retry_after("Wed, 21 Oct 2015 07:28:20 GMT", base),
            Some(Duration::from_secs(20))
        );
        assert!(parse_retry_after("Mon, 29 Feb 2016 00:00:00 GMT", 0).is_some());
    }

    struct AlwaysFailProvider {
        attempts: Arc<AtomicU32>,
    }

    #[async_trait]
    impl Provider for AlwaysFailProvider {
        async fn chat(&self, _req: &ChatRequest) -> Result<ChatResponse> {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            Err(LlmError::Api {
                status: 502,
                message: "transient".into(),
            })
        }

        async fn stream(
            &self,
            _req: &ChatRequest,
        ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
            Ok(Box::pin(stream::empty()))
        }
    }

    #[tokio::test]
    async fn audit_2026_http_with_retry_is_idempotent() {
        let attempts = Arc::new(AtomicU32::new(0));
        let client = llmrust::LmrsClient::new();
        client
            .set_custom(
                "count",
                Arc::new(AlwaysFailProvider {
                    attempts: Arc::clone(&attempts),
                }),
            )
            .await;
        client.with_retry(1).await;
        client.with_retry(3).await;
        let error = client.chat("count/m", "hello").await.unwrap_err();
        assert!(matches!(error, LlmError::Api { status: 502, .. }));
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            2,
            "repeated wrapping multiplied retry loops"
        );
        assert!(client
            .get_provider("count")
            .await
            .unwrap()
            .is_retry_provider());
    }

    #[tokio::test]
    async fn audit_2026_http_custom_openai_url_is_not_official_reasoning() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let provider = OpenAIProvider::new(
            ProviderConfig::new("k")
                .with_base_url(format!("http://{address}/v1"))
                .with_timeout_secs(1),
        );
        assert_eq!(
            provider.capabilities().reasoning.level,
            CapabilityLevel::Unsupported
        );
        let request = ChatRequest::new("o3", "hello").with_thinking(
            llmrust::types::ThinkingConfig::Enabled {
                budget_tokens: None,
            },
        );
        assert!(matches!(
            provider.stream(&request).await,
            Err(LlmError::Unsupported { .. })
        ));
        assert_eq!(listener.accept().unwrap_err().kind(), ErrorKind::WouldBlock);

        let custom_official_origin = OpenAIProvider::new(
            ProviderConfig::new("k").with_base_url("https://api.openai.com/v1"),
        );
        assert_eq!(
            custom_official_origin.capabilities().reasoning.level,
            CapabilityLevel::Unsupported
        );
        let official = OpenAIProvider::new(ProviderConfig::new("k"));
        assert_ne!(
            official.capabilities().reasoning.level,
            CapabilityLevel::Unsupported
        );
    }

    #[tokio::test]
    async fn audit_2026_http_google_model_is_path_segment_encoded() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let attempts = Arc::new(AtomicUsize::new(0));
        let server_attempts = Arc::clone(&attempts);
        let server = serve_n(listener, 2, move |_request| {
            if server_attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                Some((
                    200,
                    Vec::new(),
                    r#"{"modelVersion":"gemini-test","candidates":[{"content":{"parts":[{"text":"ok"}]},"finishReason":"STOP"}]}"#.into(),
                ))
            } else {
                Some((
                    200,
                    vec![("Content-Type".into(), "text/event-stream".into())],
                    "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"ok\"}]},\"finishReason\":\"STOP\"}]}\n\n".into(),
                ))
            }
        });
        let provider = GoogleProvider::new(
            ProviderConfig::new("AIza-secret").with_base_url(format!("http://{address}/v1beta/")),
        );
        let model = "gemini/proxy?x=1#frag name";
        provider
            .chat(&ChatRequest::new(model, "hello"))
            .await
            .unwrap();
        let mut stream = provider
            .stream(&ChatRequest::new(model, "hello"))
            .await
            .unwrap();
        while let Some(chunk) = stream.next().await {
            chunk.unwrap();
        }
        let captured = server.join().unwrap();
        let encoded = "gemini%2Fproxy%3Fx=1%23frag%20name";
        assert_eq!(
            captured[0].target,
            format!("/v1beta/models/{encoded}:generateContent")
        );
        assert_eq!(
            captured[1].target,
            format!("/v1beta/models/{encoded}:streamGenerateContent?alt=sse")
        );
        assert!(!captured[0].target.contains("AIza-secret"));
        assert!(captured[0]
            .headers
            .to_ascii_lowercase()
            .contains("aiza-secret"));
    }

    #[tokio::test]
    async fn audit_2026_http_base_url_rejects_userinfo_query_fragment() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut invalid = Vec::new();
        for provider_kind in 0..4 {
            invalid.push((
                provider_kind,
                format!("http://alice:topsecret@{address}/v1"),
            ));
            for suffix in ["?token=query-secret", "#fragment-secret", "?", "#"] {
                invalid.push((provider_kind, format!("http://{address}{suffix}")));
            }
        }
        for (provider_kind, base_url) in invalid {
            let config = ProviderConfig::new("k")
                .with_base_url(base_url)
                .with_timeout_secs(1);
            let provider: Arc<dyn Provider> = match provider_kind {
                0 => Arc::new(OpenAiCompatibleProvider::new(config, [])),
                1 => Arc::new(AnthropicProvider::new(config)),
                2 => Arc::new(GoogleProvider::new(config)),
                _ => Arc::new(OllamaProvider::new(config)),
            };
            let error = provider.chat(&req()).await.unwrap_err();
            assert!(
                matches!(error, LlmError::Parse(_)),
                "{provider_kind}: {error:?}"
            );
            let shown = error.to_string();
            for secret in ["alice", "topsecret", "query-secret", "fragment-secret"] {
                assert!(
                    !shown.contains(secret),
                    "base URL error leaked {secret}: {shown}"
                );
            }
        }
        assert_eq!(listener.accept().unwrap_err().kind(), ErrorKind::WouldBlock);
    }

    #[tokio::test]
    async fn audit_2026_http_base_url_trailing_slash_normalized() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = serve_n(listener, 2, |_request| {
            Some((200, Vec::new(), openai_chat_response()))
        });
        for base_url in [
            format!("http://{address}/v1/"),
            format!("http://{address}/v1///"),
        ] {
            let provider =
                OpenAiCompatibleProvider::new(ProviderConfig::new("k").with_base_url(base_url), []);
            provider.chat(&req()).await.unwrap();
        }
        let captured = server.join().unwrap();
        assert_eq!(captured.len(), 2);
        assert!(captured
            .iter()
            .all(|request| request.target == "/v1/chat/completions"));
    }
}

// ── B-018-PROVIDER: production-wire integrity audits ──────────────────────

mod audit_2026_provider_integrity_tests {
    use super::audit_2026_http_tests::{serve_n, ResponseSpec};
    use super::*;
    use futures::StreamExt;
    use llmrust::providers::anthropic::AnthropicProvider;
    use llmrust::providers::compat::OpenAiCompatibleProvider;
    use llmrust::providers::google::GoogleProvider;
    use llmrust::providers::ollama::OllamaProvider;
    use llmrust::types::{
        EmbeddingRequest, FinishReason, FunctionCall, Message, ResponseFormat, ToolCall,
    };
    use llmrust::ProviderConfig;
    use std::io::ErrorKind;
    use std::net::TcpListener;

    fn json_response(body: impl Into<String>) -> ResponseSpec {
        (
            200,
            vec![("Content-Type".into(), "application/json".into())],
            body.into(),
        )
    }

    fn sse_response(body: impl Into<String>) -> ResponseSpec {
        (
            200,
            vec![("Content-Type".into(), "text/event-stream".into())],
            body.into(),
        )
    }

    fn ndjson_response(body: impl Into<String>) -> ResponseSpec {
        (
            200,
            vec![("Content-Type".into(), "application/x-ndjson".into())],
            body.into(),
        )
    }

    fn request_json(request: &super::audit_2026_http_tests::CapturedRequest) -> serde_json::Value {
        serde_json::from_str(&request.body).expect("fixture request body must be JSON")
    }

    fn tool_call(id: &str, name: &str) -> ToolCall {
        ToolCall {
            id: id.to_string(),
            call_type: "function".to_string(),
            function: FunctionCall {
                name: name.to_string(),
                arguments: "{}".to_string(),
            },
        }
    }

    fn assert_no_success_terminal(chunks: &[Result<StreamChunk>]) {
        assert!(
            chunks.iter().any(|chunk| chunk.is_err()),
            "an incomplete/error stream must surface Err: {chunks:?}"
        );
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk.is_err() || !chunk.as_ref().unwrap().done),
            "an error stream must never emit done=true: {chunks:?}"
        );
    }

    #[tokio::test]
    async fn audit_2026_provider_integrity_anthropic_incomplete_tool_is_error() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let wire = concat!(
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"write_file\"}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"path\\\":\"}}\n\n",
            "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        );
        let server = serve_n(listener, 1, move |_| Some(sse_response(wire)));
        let provider = AnthropicProvider::new(
            ProviderConfig::new("test").with_base_url(format!("http://{address}/v1")),
        );
        let stream = provider
            .stream(&ChatRequest::new("claude-test", "write"))
            .await
            .unwrap();
        let chunks: Vec<Result<StreamChunk>> = stream.collect().await;
        assert_no_success_terminal(&chunks);
        assert!(chunks.iter().all(|chunk| chunk
            .as_ref()
            .map_or(true, |chunk| chunk.tool_calls.is_none())));
        assert_eq!(server.join().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn audit_2026_provider_integrity_gemini_ids_are_response_scoped() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let mut index = 0usize;
        let server = serve_n(listener, 3, move |_| {
            index += 1;
            let body = match index {
                1 => {
                    r#"{"modelVersion":"gemini-test","candidates":[{"content":{"parts":[{"functionCall":{"name":"first_tool","args":{"x":1}}}]},"finishReason":"STOP"}]}"#
                }
                2 => {
                    r#"{"modelVersion":"gemini-test","candidates":[{"content":{"parts":[{"functionCall":{"name":"second_tool","args":{"x":2}}}]},"finishReason":"STOP"}]}"#
                }
                _ => {
                    r#"{"modelVersion":"gemini-test","candidates":[{"content":{"parts":[{"text":"ok"}]},"finishReason":"STOP"}]}"#
                }
            };
            Some(json_response(body))
        });
        let provider = GoogleProvider::new(
            ProviderConfig::new("test").with_base_url(format!("http://{address}/v1beta")),
        );
        let first = provider
            .chat(&ChatRequest::new("gemini-test", "first"))
            .await
            .unwrap();
        let second = provider
            .chat(&ChatRequest::new("gemini-test", "second"))
            .await
            .unwrap();
        let first_call = &first.tool_calls.as_ref().unwrap()[0];
        let second_call = &second.tool_calls.as_ref().unwrap()[0];
        assert_eq!(first_call.function.name, "first_tool");
        assert_eq!(second_call.function.name, "second_tool");
        assert_ne!(first_call.id, second_call.id);
        assert_ne!(first_call.id, "call_0");
        assert_eq!(first.finish_reason, Some(FinishReason::Stop));
        assert_eq!(second.finish_reason, Some(FinishReason::Stop));

        // Legacy clients may still reuse call_0. Resolution is round-scoped,
        // so the first result must not be rewritten by the second round.
        let history = vec![
            Message::user("start"),
            Message::assistant_tool_calls(vec![tool_call("call_0", "first_tool")]),
            Message::tool("call_0", "first result"),
            Message::assistant_tool_calls(vec![tool_call("call_0", "second_tool")]),
            Message::tool("call_0", "second result"),
        ];
        provider
            .chat(&ChatRequest::from_messages("gemini-test", history))
            .await
            .unwrap();
        let captured = server.join().unwrap();
        let body = request_json(&captured[2]);
        let names: Vec<&str> = body["contents"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|content| content["parts"].as_array())
            .flatten()
            .filter_map(|part| part["functionResponse"]["name"].as_str())
            .collect();
        assert_eq!(names, ["first_tool", "second_tool"]);
    }

    #[tokio::test]
    async fn audit_2026_provider_integrity_gemini_blocked_200_is_error() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let mut index = 0usize;
        let server = serve_n(listener, 2, move |_| {
            index += 1;
            Some(json_response(if index == 1 {
                r#"{"promptFeedback":{"blockReason":"SAFETY"}}"#
            } else {
                r#"{"modelVersion":"gemini-test","candidates":[]}"#
            }))
        });
        let provider = GoogleProvider::new(
            ProviderConfig::new("test").with_base_url(format!("http://{address}/v1beta")),
        );
        assert!(provider
            .chat(&ChatRequest::new("gemini-test", "blocked"))
            .await
            .is_err());
        assert!(provider
            .chat(&ChatRequest::new("gemini-test", "empty"))
            .await
            .is_err());
        assert_eq!(server.join().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn audit_2026_provider_integrity_compat_empty_or_refusal_is_error() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let bodies = [
            r#"{"model":"test","choices":[],"usage":{"prompt_tokens":1,"completion_tokens":0,"total_tokens":1}}"#,
            r#"{"model":"test","choices":[{"message":{"role":"assistant","content":null,"refusal":"policy refusal"},"finish_reason":"stop"}]}"#,
            r#"{"model":"test","choices":[{"message":{"role":"assistant","content":null},"finish_reason":"stop"}]}"#,
        ];
        let mut index = 0usize;
        let server = serve_n(listener, 3, move |_| {
            let body = bodies[index];
            index += 1;
            Some(json_response(body))
        });
        let provider = OpenAiCompatibleProvider::new(
            ProviderConfig::new("test").with_base_url(format!("http://{address}/v1")),
            [],
        );
        for prompt in ["empty", "refusal", "null message"] {
            assert!(provider
                .chat(&ChatRequest::new("test", prompt))
                .await
                .is_err());
        }
        assert_eq!(server.join().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn audit_2026_provider_integrity_choices_null_usage_is_accepted() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let mut index = 0usize;
        let server = serve_n(listener, 2, move |_| {
            index += 1;
            if index == 1 {
                Some(json_response(
                    r#"{"model":"test","choices":null,"usage":{"prompt_tokens":3,"completion_tokens":0,"total_tokens":3}}"#,
                ))
            } else {
                Some(sse_response(concat!(
                    "data: {\"choices\":null,\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":0,\"total_tokens\":4}}\n\n",
                    "data: [DONE]\n\n",
                )))
            }
        });
        let provider = OpenAiCompatibleProvider::new(
            ProviderConfig::new("test").with_base_url(format!("http://{address}/v1")),
            [],
        );
        let chat = provider
            .chat(&ChatRequest::new("test", "usage"))
            .await
            .unwrap();
        assert_eq!(chat.usage.unwrap().total_tokens, 3);
        let mut stream = provider
            .stream(&ChatRequest::new("test", "usage"))
            .await
            .unwrap();
        let mut total = None;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.unwrap();
            if let Some(usage) = chunk.usage {
                total = Some(usage.total_tokens);
            }
        }
        assert_eq!(total, Some(4));
        assert_eq!(server.join().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn audit_2026_provider_integrity_embedding_index_set_is_complete() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let bodies = [
            r#"{"model":"test","data":[{"index":0,"embedding":[0.1]}]}"#,
            r#"{"model":"test","data":[{"index":0,"embedding":[0.1]},{"index":0,"embedding":[0.2]}]}"#,
            r#"{"model":"test","embeddings":[[0.1]]}"#,
        ];
        let mut index = 0usize;
        let server = serve_n(listener, 3, move |_| {
            let body = bodies[index];
            index += 1;
            Some(json_response(body))
        });
        let compat = OpenAiCompatibleProvider::new(
            ProviderConfig::new("test").with_base_url(format!("http://{address}/v1")),
            [],
        );
        let ollama =
            OllamaProvider::new(ProviderConfig::new("").with_base_url(format!("http://{address}")));
        let request = EmbeddingRequest::batch("test", ["a", "b"]);
        assert!(matches!(
            compat.embed(&request).await,
            Err(LlmError::Parse(_))
        ));
        assert!(matches!(
            compat.embed(&request).await,
            Err(LlmError::Parse(_))
        ));
        assert!(matches!(
            ollama.embed(&request).await,
            Err(LlmError::Parse(_))
        ));
        assert_eq!(server.join().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn audit_2026_provider_integrity_ollama_finish_and_error_envelope() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let mut index = 0usize;
        let server = serve_n(listener, 2, move |_| {
            index += 1;
            if index == 1 {
                Some(json_response(
                    r#"{"model":"test","message":{"role":"assistant","content":"partial"},"done":true,"done_reason":"length"}"#,
                ))
            } else {
                Some(ndjson_response(concat!(
                    "{\"error\":\"model not found\"}\n",
                    "{\"message\":{\"content\":\"must not surface\"},\"done\":true}\n",
                )))
            }
        });
        let provider =
            OllamaProvider::new(ProviderConfig::new("").with_base_url(format!("http://{address}")));
        let chat = provider
            .chat(&ChatRequest::new("test", "hello"))
            .await
            .unwrap();
        assert_eq!(chat.finish_reason, Some(FinishReason::Length));
        let stream = provider
            .stream(&ChatRequest::new("test", "hello"))
            .await
            .unwrap();
        let chunks: Vec<Result<StreamChunk>> = stream.collect().await;
        assert!(matches!(chunks.as_slice(), [Err(LlmError::Stream(_))]));
        assert_eq!(server.join().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn audit_2026_provider_integrity_invalid_tool_arguments_are_not_rewritten() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut call = tool_call("call_1", "write_file");
        call.function.arguments = "{bad json super-secret-token".to_string();
        let request = ChatRequest::from_messages(
            "test",
            vec![
                Message::user("write"),
                Message::assistant_tool_calls(vec![call]),
            ],
        );
        let anthropic = AnthropicProvider::new(
            ProviderConfig::new("test").with_base_url(format!("http://{address}/v1")),
        );
        let google = GoogleProvider::new(
            ProviderConfig::new("test").with_base_url(format!("http://{address}/v1beta")),
        );
        for error in [
            anthropic.chat(&request).await.unwrap_err(),
            google.chat(&request).await.unwrap_err(),
        ] {
            assert!(matches!(error, LlmError::Parse(_)), "{error:?}");
            assert!(!error.to_string().contains("super-secret-token"));
        }
        assert_eq!(listener.accept().unwrap_err().kind(), ErrorKind::WouldBlock);
    }

    #[tokio::test]
    async fn audit_2026_provider_integrity_gemini_chat_thought_not_in_content() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = serve_n(listener, 1, move |_| {
            Some(json_response(
                r#"{"modelVersion":"gemini-test","candidates":[{"content":{"parts":[{"text":"private thought","thought":true},{"text":"public answer"}]},"finishReason":"STOP"}]}"#,
            ))
        });
        let provider = GoogleProvider::new(
            ProviderConfig::new("test").with_base_url(format!("http://{address}/v1beta")),
        );
        let response = provider
            .chat(&ChatRequest::new("gemini-test", "hello"))
            .await
            .unwrap();
        assert_eq!(response.content, "public answer");
        assert_eq!(server.join().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn audit_2026_provider_integrity_gemini_bare_schema_not_double_wrapped() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = serve_n(listener, 2, move |_| {
            Some(json_response(
                r#"{"modelVersion":"gemini-test","candidates":[{"content":{"parts":[{"text":"ok"}]},"finishReason":"STOP"}]}"#,
            ))
        });
        let provider = GoogleProvider::new(
            ProviderConfig::new("test").with_base_url(format!("http://{address}/v1beta")),
        );
        let bare = serde_json::json!({
            "type": "object",
            "schema": {"type": "string"},
            "properties": {"schema": {"type": "string"}}
        });
        provider
            .chat(
                &ChatRequest::new("gemini-test", "bare")
                    .with_response_format(ResponseFormat::json_schema(bare.clone())),
            )
            .await
            .unwrap();
        let wrapped_inner = serde_json::json!({"schema": {"type": "object"}});
        provider
            .chat(
                &ChatRequest::new("gemini-test", "wrapped").with_response_format(
                    ResponseFormat::json_schema(serde_json::json!({
                        "name": "wrapped",
                        "schema": wrapped_inner.clone(),
                        "strict": true
                    })),
                ),
            )
            .await
            .unwrap();
        let captured = server.join().unwrap();
        assert_eq!(
            request_json(&captured[0])["generationConfig"]["responseSchema"],
            bare
        );
        assert_eq!(
            request_json(&captured[1])["generationConfig"]["responseSchema"],
            wrapped_inner
        );
    }

    #[tokio::test]
    async fn audit_2026_provider_integrity_gemini_stream_logprobs_not_dropped() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let provider = GoogleProvider::new(
            ProviderConfig::new("test").with_base_url(format!("http://{address}/v1beta")),
        );
        let request = ChatRequest::new("gemini-test", "hello")
            .with_logprobs(true)
            .with_top_logprobs(2);
        assert!(matches!(
            provider.stream(&request).await,
            Err(LlmError::Unsupported { feature, .. }) if feature == "logprobs"
        ));
        assert_eq!(listener.accept().unwrap_err().kind(), ErrorKind::WouldBlock);
    }
}
