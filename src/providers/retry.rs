//! Retry decorator for any [`Provider`].
//!
//! Wraps an existing provider and retries failed requests with exponential
//! backoff and jitter. Retryable errors:
//!
//! - HTTP 5xx (server errors — the upstream may recover)
//! - Network / transport errors (`LlmError::Http`)
//! - Stream connection errors
//!
//! Non-retryable errors (propagated immediately):
//!
//! - HTTP 4xx (client errors — your request is bad)
//! - `LlmError::Parse` (response format changed — retrying won't help)
//! - `LlmError::UnknownProvider` (wrong wiring)
//! - `LlmError::Unsupported` (feature not implemented — retrying won't help)
//!
//! # Example
//!
//! ```rust,ignore
//! let inner = OpenAIProvider::new(config);
//! let retrying = RetryProvider::new(Arc::new(inner) as Arc<dyn Provider>, 3);
//! // use retrying where a Provider is expected
//! ```

use async_trait::async_trait;
use futures::stream::BoxStream;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

use crate::providers::{LlmError, Provider, Result};
use crate::types::{ChatRequest, ChatResponse, EmbeddingRequest, EmbeddingResponse, StreamChunk};

// ── Constants ──────────────────────────────────────────────────────────────

const DEFAULT_BASE_DELAY_MS: u64 = 500; // 0.5 s initial back-off
const DEFAULT_MAX_DELAY_MS: u64 = 30_000; // 30 s ceiling

// ── Retry decision ─────────────────────────────────────────────────────────

/// Returns `true` when the error is (likely) transient and worth retrying.
///
/// ## Differences from [`crate::router`]'s `should_failover`
///
/// Both functions look similar but make **deliberately different choices** for
/// `429` and `UnknownProvider`:
///
/// | Error              | `should_retry` | `should_failover` | Rationale |
/// |--------------------|----------------|--------------------|-----------|
/// | 5xx                | ✅ yes          | ✅ yes              | Transient server error — retry or failover both help. |
/// | 429 (rate limit)   | ❌ no           | ✅ yes              | Retrying the **same deployment** when rate-limited only worsens the situation; but the Router can switch to a **different deployment** that may not be throttled. |
/// | `UnknownProvider`  | ❌ no           | ✅ yes              | A missing provider is a configuration error when retrying a **single** deployment — retrying won't fix it. But a Router with **multiple** deployments can skip the broken one and try the next. |
/// | `Parse`            | ❌ no           | ❌ no               | Malformed responses won't become valid on a second try (or a different deployment). |
///
/// **In short:** `should_retry` asks "should I try this same thing again?" while
/// `should_failover` asks "should I try a different thing instead?".
fn should_retry(e: &LlmError) -> bool {
    match e {
        // Network / transport errors are transient by nature.
        LlmError::Http(_) => true,
        // 5xx means the upstream is having trouble; it may recover.
        LlmError::Api { status, .. } if *status >= 500 => true,
        // Stream connection drops — worth one retry.
        LlmError::Stream(_) => true,
        // Everything else is a permanent failure.
        _ => false,
    }
}

// ── ERR-004：消费上游 `Retry-After` ────────────────────────────────────────
//
// 0.1.3 期**从不读取**上游的 `Retry-After`（全仓 `retry.?after` 零命中）：
// `backoff()` 是纯本地指数 + jitter。上游明说"等 30 秒再来"时，本地可能 500ms 就重试——
// 既白撞墙（会被继续限流），也把服务端压力叠加回去。
//
// 处置：**有上游指示时优先采用**，但**必须设上限**（卡内禁止"无上限等待"）——
// 恶意或错误的上游可以回一个夸张值（甚至 HTTP-date 在遥远的未来）来钉死客户端。

/// 上游指示的等待窗口**上限**（防恶意/异常的超长值）。
pub const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);

/// 当前 Unix 秒（HTTP-date 形态的 `Retry-After` 需要"现在"作基准）。
///
/// 抽出来是为了让 [`parse_retry_after`] 保持**纯函数**：基准由调用方传入，测试可复现。
pub(crate) fn now_unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// `Retry-After` 的两种合法格式（RFC 9110 §10.2.3）：
/// **delay-seconds**（非负整数秒）与 **HTTP-date**（IMF-fixdate）。
///
/// `now_unix_secs` 由调用方传入，使本函数**纯粹可测**。
///
/// 返回：
/// - `None` —— 值缺失、为空、或**无法解析**（畸形）：调用方回落到本地退避；
/// - `Some(d)` —— 解析出的等待窗口，**已按 [`MAX_RETRY_AFTER`] 截顶**；
///   HTTP-date 已过去时返回 `Some(0)`（= 立刻可重试，而非负数）。
pub fn parse_retry_after(value: &str, now_unix_secs: i64) -> Option<Duration> {
    let v = value.trim();
    if v.is_empty() {
        return None;
    }
    // ① delay-seconds
    if let Ok(secs) = v.parse::<u64>() {
        return Some(Duration::from_secs(secs).min(MAX_RETRY_AFTER));
    }
    // ② HTTP-date（IMF-fixdate，如 `Wed, 21 Oct 2015 07:28:00 GMT`）
    let target = http_date_to_unix(v)?;
    let delta = target.saturating_sub(now_unix_secs);
    if delta <= 0 {
        return Some(Duration::ZERO);
    }
    Some(Duration::from_secs(delta as u64).min(MAX_RETRY_AFTER))
}

/// 解析 IMF-fixdate 为 Unix 秒（纯函数；不引入日期库依赖）。
///
/// 只接受 RFC 9110 要求的 **GMT** 形态；星期几不参与换算（仅作装饰，故不校验其正确性）。
fn http_date_to_unix(s: &str) -> Option<i64> {
    // "Wed, 21 Oct 2015 07:28:00 GMT" → ["Wed","21","Oct","2015","07:28:00","GMT"]
    let cleaned = s.replace(',', " ");
    let parts: Vec<&str> = cleaned.split_whitespace().collect();
    if parts.len() != 6 || !parts[5].eq_ignore_ascii_case("GMT") {
        return None;
    }
    let day: u32 = parts[1].parse().ok()?;
    let month = match parts[2].to_ascii_lowercase().as_str() {
        "jan" => 1,
        "feb" => 2,
        "mar" => 3,
        "apr" => 4,
        "may" => 5,
        "jun" => 6,
        "jul" => 7,
        "aug" => 8,
        "sep" => 9,
        "oct" => 10,
        "nov" => 11,
        "dec" => 12,
        _ => return None,
    };
    let year: i64 = parts[3].parse().ok()?;
    let hms: Vec<&str> = parts[4].split(':').collect();
    if hms.len() != 3 {
        return None;
    }
    let h: i64 = hms[0].parse().ok()?;
    let mi: i64 = hms[1].parse().ok()?;
    let sec: i64 = hms[2].parse().ok()?;
    if !(1..=31).contains(&day)
        || !(0..24).contains(&h)
        || !(0..60).contains(&mi)
        || !(0..61).contains(&sec)
    {
        return None;
    }
    // H-2：**年份必须有界**。IMF-fixdate 定的是 4 位年（RFC 9110 §5.6.7 `year = 4DIGIT`），
    // 此前只解析不校验，于是巨年份会让 `days_from_civil` 里的 `era * 146_097` **i64 溢出**——
    // debug 下直接 panic（实测：`year = i64::MAX` → `attempt to multiply with overflow`），
    // release 下回绕出垃圾值。
    //
    // 为什么是 `None` 而不是"钳到某个日期"：一个**畸形**日期并不表达"立刻重试"，
    // 把它降级成 `0`（= 马上重发）等于**凭空造出一个语义**——与 `ERR-003` 同类错误。
    // 故：年份不在 4 位范围内 ⇒ 视为**不可解析**，交由调用方回落本地退避。
    if !(1000..=9999).contains(&year) {
        return None;
    }
    // 兜底：即便上面的范围断言将来被放宽，也不允许回绕（用饱和算术而非裸乘加）。
    let days = days_from_civil(year, month, day);
    let secs = days
        .saturating_mul(86_400)
        .saturating_add(h.saturating_mul(3_600))
        .saturating_add(mi.saturating_mul(60))
        .saturating_add(sec);
    Some(secs)
}

/// Howard Hinnant 的 `days_from_civil`（公历 → 自 1970-01-01 起的天数）。
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = ((m + 9) % 12) as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// 退避决策：**有上游指示时优先采用**，否则本地指数退避。
///
/// 返回 `(delay, source)`；`source` 供日志标注**等待窗口的来源**（DoD 要求"可观测"）。
pub fn backoff_with_hint(
    attempt: u32,
    base_ms: u64,
    max_ms: u64,
    upstream_hint: Option<Duration>,
) -> (Duration, &'static str) {
    match upstream_hint {
        Some(hint) => (
            hint.min(MAX_RETRY_AFTER).max(Duration::from_millis(1)),
            "upstream",
        ),
        None => (backoff(attempt, base_ms, max_ms), "local"),
    }
}

// ── Exponential back-off with jitter ───────────────────────────────────────

fn backoff(attempt: u32, base_ms: u64, max_ms: u64) -> Duration {
    // 2^attempt capped at 2^30 (~1 billion) to avoid u64 overflow.
    let exponent = 1u64 << attempt.min(30);
    let delay = base_ms.saturating_mul(exponent).min(max_ms);

    // "Equal jitter" — spread retries within [delay/2, delay] to avoid
    // thundering-herd. Uses `fastrand` for real randomness so multiple
    // clients retrying the same service don't synchronize.
    let quarter = delay / 4;
    let jitter = match quarter {
        0 => 0,
        q => fastrand::u64(0..q),
    };
    let actual = delay - quarter + jitter;

    Duration::from_millis(actual.max(1))
}

// ── RetryProvider ──────────────────────────────────────────────────────────

/// A [`Provider`] decorator that retries transient failures.
///
/// Internally holds an `Arc<dyn Provider>`, making it possible to wrap an
/// already-boxed provider without knowing its concrete type.
///
/// **Embeddings:** non-streaming [`Provider::embed`] requests are retried with
/// the same transient-error policy as [`Provider::chat`].
///
/// **Note on streaming:** `stream()` retries only the **initial connection**
/// (the call to `inner.stream(req)`). Once a byte stream is established,
/// mid-stream chunk errors are **not** retried — they are propagated to the
/// caller. If you need full mid-stream reliability, use `chat()` instead,
/// or implement application-level retry on the consuming end (e.g. re-drive
/// the prompt on failure).
pub struct RetryProvider {
    inner: Arc<dyn Provider>,
    max_retries: u32,
    base_delay_ms: u64,
    max_delay_ms: u64,
}

impl RetryProvider {
    /// Create a retry wrapper with `max_retries` retry attempts.
    ///
    /// The first retry waits ~`base_delay_ms`, then doubles each attempt up
    /// to `max_delay_ms`.
    pub fn new(inner: Arc<dyn Provider>, max_retries: u32) -> Self {
        Self {
            inner,
            max_retries,
            base_delay_ms: DEFAULT_BASE_DELAY_MS,
            max_delay_ms: DEFAULT_MAX_DELAY_MS,
        }
    }

    /// Customise the initial back-off delay (default: 500 ms).
    pub fn with_base_delay(mut self, ms: u64) -> Self {
        self.base_delay_ms = ms;
        self
    }

    /// Customise the maximum back-off delay (default: 30 s).
    pub fn with_max_delay(mut self, ms: u64) -> Self {
        self.max_delay_ms = ms;
        self
    }
}

#[async_trait]
impl Provider for RetryProvider {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse> {
        for attempt in 0..=self.max_retries {
            match self.inner.chat(req).await {
                Ok(resp) => {
                    if attempt > 0 {
                        tracing::info!(attempt, "retry succeeded");
                    }
                    return Ok(resp);
                }
                Err(e) => {
                    // The final attempt (attempt == max_retries) never retries —
                    // it returns immediately regardless of the error kind.
                    if !(should_retry(&e) && attempt < self.max_retries) {
                        return Err(e);
                    }
                    // ERR-004：优先采用上游明示的等待窗口（`Retry-After`），否则本地退避。
                    let (delay, delay_source) = backoff_with_hint(
                        attempt,
                        self.base_delay_ms,
                        self.max_delay_ms,
                        self.inner.last_retry_after(),
                    );
                    tracing::warn!(
                        attempt = attempt + 1,
                        max_retries = self.max_retries,
                        delay_ms = delay.as_millis() as u64,
                        delay_source, // "upstream" | "local" —— 等待窗口来源可观测（DoD）
                        error_kind = "transient",
                        "retrying transient failure"
                    );
                    sleep(delay).await;
                }
            }
        }
        // The loop body always returns on the final iteration, so this point
        // is unreachable.
        unreachable!("retry loop always returns on the final attempt")
    }

    async fn stream(&self, req: &ChatRequest) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        for attempt in 0..=self.max_retries {
            match self.inner.stream(req).await {
                Ok(stream) => {
                    if attempt > 0 {
                        tracing::info!(attempt, "retry succeeded (stream)");
                    }
                    return Ok(stream);
                }
                Err(e) => {
                    if !(should_retry(&e) && attempt < self.max_retries) {
                        return Err(e);
                    }
                    // ERR-004：同上（流式路径同样消费上游指示）。
                    let (delay, delay_source) = backoff_with_hint(
                        attempt,
                        self.base_delay_ms,
                        self.max_delay_ms,
                        self.inner.last_retry_after(),
                    );
                    tracing::warn!(
                        attempt = attempt + 1,
                        max_retries = self.max_retries,
                        delay_ms = delay.as_millis() as u64,
                        delay_source,
                        error_kind = "transient",
                        "retrying transient failure (stream)"
                    );
                    sleep(delay).await;
                }
            }
        }
        unreachable!("retry loop always returns on the final attempt")
    }

    async fn embed(&self, req: &EmbeddingRequest) -> Result<EmbeddingResponse> {
        for attempt in 0..=self.max_retries {
            match self.inner.embed(req).await {
                Ok(resp) => {
                    if attempt > 0 {
                        tracing::info!(attempt, "retry succeeded (embeddings)");
                    }
                    return Ok(resp);
                }
                Err(e) => {
                    if !(should_retry(&e) && attempt < self.max_retries) {
                        return Err(e);
                    }
                    let delay = backoff(attempt, self.base_delay_ms, self.max_delay_ms);
                    tracing::warn!(
                        attempt = attempt + 1,
                        max_retries = self.max_retries,
                        delay_ms = delay.as_millis() as u64,
                        error_kind = "transient",
                        "retrying transient failure (embeddings)"
                    );
                    sleep(delay).await;
                }
            }
        }
        unreachable!("retry loop always returns on the final attempt")
    }

    /// `H-1`：**装饰器必须转发被包 provider 的能力声明**。
    ///
    /// 不转发时，`LmrsClient::adjudicate`（`CAP-003`）拿到的是**默认实现**——
    /// `declared == false` ⇒ 裁决判为"未声明能力"⇒ **只 warn、不 Reject**
    /// ⇒ **开着 `with_retry()` 的客户端里，CAP-003 的保护静默失效**
    /// （实测：Ollama + `tools` 在裸 provider 下返回 `Unsupported`，包上 retry 后**变成发往网络**）。
    fn capabilities(&self) -> crate::providers::capabilities::Capabilities {
        self.inner.capabilities()
    }

    /// `H-1`：同上，转发协议名（日志与 `Capabilities::unknown` 都用它）。
    fn protocol_name(&self) -> &'static str {
        self.inner.protocol_name()
    }

    /// `H-1`：转发上游 `Retry-After` 提示。
    ///
    /// `RetryProvider` 自己的重试循环读的是 `self.inner.last_retry_after()`（见上），
    /// 但**嵌套包装**（retry 套 retry）或**外层还要读它**时，本方法必须把内层的值透出去，
    /// 否则链断在装饰器这一层。
    fn last_retry_after(&self) -> Option<std::time::Duration> {
        self.inner.last_retry_after()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use futures::stream;
    use futures::StreamExt;

    // ── ERR-004：`Retry-After` 的五类输入（DoD 点名） ──────────────────────
    //
    // 五类：① 无头/空 → None；② delay-seconds；③ HTTP-date；④ 畸形 → None；
    //       ⑤ 夸张值 → **截顶**（卡内禁止"无上限等待"）。

    // ── H-1：装饰器必须转发能力声明（否则 CAP-003 的保护静默失效） ─────────

    /// 一个最小 provider，用来验证装饰器**转发**被包对象的能力声明。
    struct DeclaringProvider;

    #[async_trait]
    impl Provider for DeclaringProvider {
        async fn chat(&self, _req: &ChatRequest) -> Result<ChatResponse> {
            Ok(ChatResponse::default())
        }
        async fn stream(
            &self,
            _req: &ChatRequest,
        ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
            Ok(Box::pin(stream::iter(vec![])))
        }
        fn protocol_name(&self) -> &'static str {
            "declaring"
        }
        fn capabilities(&self) -> crate::providers::capabilities::Capabilities {
            let mut c = crate::providers::capabilities::Capabilities::unknown("declaring");
            c.declared = true;
            c.tool_calling = crate::providers::capabilities::Capability::unsupported();
            c
        }
    }

    /// `H-1`：包了 retry 之后，**能力声明必须与内层逐位相同**（尤其 `declared`）。
    ///
    /// 修复前：`RetryProvider` 不实现 `capabilities()` ⇒ 落到默认实现 ⇒ `declared == false`
    /// ⇒ `CAP-003` 裁决把"已明确声明 unsupported"误判为"未声明" ⇒ **只 warn、不 Reject**。
    #[test]
    fn h1_decorator_forwards_capability_declaration() {
        let inner = DeclaringProvider;
        let wrapped = RetryProvider::new(Arc::new(DeclaringProvider), 2);
        assert_eq!(
            wrapped.capabilities(),
            inner.capabilities(),
            "H-1: the decorator must forward `capabilities()` verbatim"
        );
        assert!(
            wrapped.capabilities().declared,
            "`declared` must survive decoration"
        );
    }

    /// `H-1`：协议名同样必须转发（日志与 `Capabilities::unknown` 都用它）。
    #[test]
    fn h1_decorator_forwards_protocol_name() {
        let wrapped = RetryProvider::new(Arc::new(DeclaringProvider), 2);
        assert_eq!(wrapped.protocol_name(), "declaring");
    }

    /// `H-1`：`last_retry_after()` 也必须透出去（**嵌套包装**时否则断链）。
    #[test]
    fn h1_decorator_forwards_retry_after_hint() {
        struct HintProvider;
        #[async_trait]
        impl Provider for HintProvider {
            async fn chat(&self, _req: &ChatRequest) -> Result<ChatResponse> {
                Ok(ChatResponse::default())
            }
            async fn stream(
                &self,
                _req: &ChatRequest,
            ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
                Ok(Box::pin(stream::iter(vec![])))
            }
            fn last_retry_after(&self) -> Option<Duration> {
                Some(Duration::from_secs(7))
            }
        }
        let wrapped = RetryProvider::new(Arc::new(HintProvider), 2);
        assert_eq!(
            wrapped.last_retry_after(),
            Some(Duration::from_secs(7)),
            "H-1: the retry hint must survive decoration (nested wrapping)"
        );
    }

    // ── H-2：HTTP-date 年份溢出（外部审计发现；本条为**负例**） ─────────────
    //
    // 实测过的缺陷：`year` 只解析不校验 ⇒ 巨年份让 `days_from_civil` 的 `era * 146_097`
    // **i64 溢出**（debug 直接 panic：`attempt to multiply with overflow`；release 回绕）。

    /// **负例**：巨年份 / i64 上界 / 负年份 → **不 panic**，且**返回 `None`**（不猜、不造语义）。
    #[test]
    fn h2_absurd_years_return_none_without_panicking() {
        for y in [
            "99999999999",          // 11 位
            "9223372036854775807",  // i64::MAX
            "-9223372036854775808", // i64::MIN
            "-99999999999",         // 大负年份
            "0",                    // 0 年（非 4 位有效年）
            "999",                  // 3 位
            "10000",                // 5 位
        ] {
            let s = format!("Wed, 21 Oct {y} 07:28:00 GMT");
            let got = parse_retry_after(&s, 0);
            assert_eq!(
                got, None,
                "year `{y}` must be treated as MALFORMED (got {got:?}); \
                 silently degrading it to 0 would fabricate a `retry immediately` semantic"
            );
        }
    }

    /// 正向对照：**合法 4 位年**仍须照常解析（不得因加锁而误伤）。
    #[test]
    fn h2_valid_four_digit_years_still_parse() {
        let base = 1_445_412_480i64; // 2015-10-21T07:28:00Z
        assert_eq!(
            parse_retry_after("Wed, 21 Oct 2015 07:28:00 GMT", base),
            Some(Duration::ZERO)
        );
        assert_eq!(
            parse_retry_after("Wed, 21 Oct 2015 07:28:20 GMT", base),
            Some(Duration::from_secs(20))
        );
        // 边界年：1000 与 9999 都属合法 4 位年（不得被新锁误杀）。
        for y in ["1000", "9999"] {
            let s = format!("Wed, 21 Oct {y} 07:28:00 GMT");
            assert!(
                parse_retry_after(&s, base).is_some(),
                "year `{y}` is a valid 4-digit IMF-fixdate year"
            );
        }
    }

    /// 溢出兜底：即便将来放宽年份断言，也必须**饱和**而非回绕。
    #[test]
    fn h2_arithmetic_is_saturating_not_wrapping() {
        // 直接驱动内部换算，确认饱和语义（不 panic、不出现负值）。
        let days = days_from_civil(9999, 12, 31);
        let secs = days
            .saturating_mul(86_400)
            .saturating_add(23_i64.saturating_mul(3_600));
        assert!(secs > 0, "far-future date must stay positive, got {secs}");
        let d = parse_retry_after("Fri, 31 Dec 9999 23:59:59 GMT", 0);
        assert_eq!(
            d,
            Some(MAX_RETRY_AFTER),
            "a far-future valid date must be CAPPED, not overflowed"
        );
    }

    /// ① 缺失/空 → `None`（调用方回落到本地退避）。
    #[test]
    fn retry_after_absent_or_empty_yields_none() {
        assert_eq!(parse_retry_after("", 0), None);
        assert_eq!(parse_retry_after("   ", 0), None);
    }

    /// ② delay-seconds（RFC 9110 的第一种合法形态）。
    #[test]
    fn retry_after_delay_seconds_is_honored() {
        assert_eq!(parse_retry_after("30", 0), Some(Duration::from_secs(30)));
        assert_eq!(parse_retry_after(" 5 ", 0), Some(Duration::from_secs(5)));
        // 0 秒是合法的（"立刻可重试"），不是"缺失"。
        assert_eq!(parse_retry_after("0", 0), Some(Duration::ZERO));
    }

    /// ③ HTTP-date（第二种合法形态）：解析为**相对当前时刻**的窗口。
    ///
    /// 用固定基准时刻计算，保证可复现（不读系统时钟）。
    #[test]
    fn retry_after_http_date_is_honored_relative_to_now() {
        // 2015-10-21T07:28:00Z = 1445412480
        let base = 1_445_412_480i64;
        let got = parse_retry_after("Wed, 21 Oct 2015 07:28:00 GMT", base);
        assert_eq!(got, Some(Duration::ZERO), "same instant → 0, not negative");

        let later = parse_retry_after("Wed, 21 Oct 2015 07:28:20 GMT", base);
        assert_eq!(later, Some(Duration::from_secs(20)));
    }

    /// ③ 补：HTTP-date 已过期 → `Some(0)`（**不得**出现负数/下溢）。
    #[test]
    fn retry_after_past_http_date_clamps_to_zero() {
        let base = 1_445_412_480i64; // 2015-10-21T07:28:00Z
        let got = parse_retry_after("Wed, 21 Oct 2015 07:00:00 GMT", base);
        assert_eq!(got, Some(Duration::ZERO));
    }

    /// ④ 畸形输入 → `None`（不 panic、不猜测）。
    #[test]
    fn retry_after_malformed_yields_none() {
        for bad in [
            "soon",
            "-5",
            "3.5",
            "Wed, 21 FOO 2015 07:28:00 GMT",
            "Wed, 21 Oct 2015 07:28:00 UTC", // 非 GMT
            "21 Oct 2015 07:28:00",          // 缺 GMT
            "Wed, 21 Oct 2015 25:28:00 GMT", // 小时越界
            "Wed, 32 Oct 2015 07:28:00 GMT", // 日越界
        ] {
            assert_eq!(parse_retry_after(bad, 0), None, "`{bad}` must not parse");
        }
    }

    /// ⑤ 夸张值 → **截顶**到 [`MAX_RETRY_AFTER`]（防恶意/异常上游钉死客户端）。
    #[test]
    fn retry_after_absurd_values_are_capped() {
        // 一天
        assert_eq!(
            parse_retry_after("86400", 0),
            Some(MAX_RETRY_AFTER),
            "a huge delay-seconds must be capped"
        );
        // u64 上限附近的巨值也不得 panic / 不得穿透上限
        assert_eq!(
            parse_retry_after(&u64::MAX.to_string(), 0),
            Some(MAX_RETRY_AFTER)
        );
        // 远未来的 HTTP-date 同样截顶
        let base = 1_445_412_480i64; // 2015
        assert_eq!(
            parse_retry_after("Wed, 21 Oct 2115 07:28:00 GMT", base),
            Some(MAX_RETRY_AFTER),
            "an HTTP-date far in the future must be capped"
        );
    }

    /// **优先级**：有上游指示时采用上游值，且来源标注为 `upstream`；无则本地退避。
    #[test]
    fn upstream_hint_takes_precedence_over_local_backoff() {
        let hint = Some(Duration::from_secs(7));
        let (delay, source) = backoff_with_hint(0, 500, 30_000, hint);
        assert_eq!(delay, Duration::from_secs(7));
        assert_eq!(source, "upstream");

        let (delay, source) = backoff_with_hint(0, 500, 30_000, None);
        assert_eq!(source, "local");
        assert!(
            delay <= Duration::from_millis(500),
            "local backoff stays in its own envelope: {delay:?}"
        );
    }

    /// 上游值即便**绕过**解析器的截顶（直接传入）也不得穿透上限。
    #[test]
    fn hint_is_capped_even_if_passed_directly() {
        let (delay, source) = backoff_with_hint(0, 500, 30_000, Some(Duration::from_secs(9999)));
        assert_eq!(source, "upstream");
        assert_eq!(delay, MAX_RETRY_AFTER);
    }

    /// 上游指示为 0 时**不得**退化成 0 延迟忙等（下限 1ms）。
    #[test]
    fn zero_hint_does_not_become_a_busy_loop() {
        let (delay, _) = backoff_with_hint(0, 500, 30_000, Some(Duration::ZERO));
        assert!(delay >= Duration::from_millis(1), "got {delay:?}");
    }

    /// 本地退避的既有性质未被削弱（本卡只**加**优先路径，不推翻原设计）。
    #[test]
    fn local_backoff_still_doubles_and_is_capped() {
        let d0 = backoff(0, 100, 10_000);
        let d3 = backoff(3, 100, 10_000);
        assert!(d0 <= Duration::from_millis(100));
        assert!(d3 <= Duration::from_millis(800) && d3 >= Duration::from_millis(600));
        let d_huge = backoff(30, 100, 10_000);
        assert!(d_huge <= Duration::from_millis(10_000), "capped at max_ms");
    }

    // ── Mock provider that fails N times then succeeds ──

    struct FlakyProvider {
        fail_count: Arc<std::sync::atomic::AtomicU32>,
        max_fails: u32,
    }

    impl FlakyProvider {
        fn new(max_fails: u32) -> Self {
            Self {
                fail_count: Arc::new(std::sync::atomic::AtomicU32::new(0)),
                max_fails,
            }
        }
    }

    #[async_trait]
    impl Provider for FlakyProvider {
        async fn chat(&self, _req: &ChatRequest) -> Result<ChatResponse> {
            let fails = self
                .fail_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if fails < self.max_fails {
                Err(LlmError::Api {
                    status: 502,
                    message: "bad gateway".to_string(),
                })
            } else {
                Ok(ChatResponse {
                    content: "ok".to_string(),
                    model: "test".to_string(),
                    usage: None,
                    ..Default::default()
                })
            }
        }

        async fn stream(
            &self,
            _req: &ChatRequest,
        ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
            let fails = self
                .fail_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if fails < self.max_fails {
                Err(LlmError::Api {
                    status: 502,
                    message: "bad gateway".to_string(),
                })
            } else {
                Ok(Box::pin(stream::once(async {
                    Ok(StreamChunk {
                        delta: "ok".to_string(),
                        done: true,
                        ..Default::default()
                    })
                })))
            }
        }
    }

    fn dummy_req() -> ChatRequest {
        ChatRequest::new("test", "hello")
    }

    #[tokio::test]
    async fn succeeds_on_first_try() {
        let provider = Arc::new(FlakyProvider::new(0));
        let retry = RetryProvider::new(provider, 3);
        let resp = retry.chat(&dummy_req()).await.unwrap();
        assert_eq!(resp.content, "ok");
    }

    #[tokio::test]
    async fn retries_and_succeeds() {
        let provider = Arc::new(FlakyProvider::new(2));
        let retry = RetryProvider::new(provider, 3);
        let resp = retry.chat(&dummy_req()).await.unwrap();
        assert_eq!(resp.content, "ok");
    }

    #[tokio::test]
    async fn exhausts_retries() {
        let provider = Arc::new(FlakyProvider::new(10));
        let retry = RetryProvider::new(provider, 2); // only 2 retries
        let err = retry.chat(&dummy_req()).await.unwrap_err();
        assert!(matches!(err, LlmError::Api { status: 502, .. }));
    }

    #[tokio::test]
    async fn does_not_retry_4xx() {
        struct FourHundredProvider;

        #[async_trait]
        impl Provider for FourHundredProvider {
            async fn chat(&self, _: &ChatRequest) -> Result<ChatResponse> {
                Err(LlmError::Api {
                    status: 400,
                    message: "bad request".to_string(),
                })
            }
            async fn stream(
                &self,
                _: &ChatRequest,
            ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
                Err(LlmError::Api {
                    status: 400,
                    message: "bad request".to_string(),
                })
            }
        }

        let retry = RetryProvider::new(Arc::new(FourHundredProvider), 3);
        let err = retry.chat(&dummy_req()).await.unwrap_err();
        assert!(matches!(err, LlmError::Api { status: 400, .. }));
    }

    #[tokio::test]
    async fn stream_retries_and_succeeds() {
        let provider = Arc::new(FlakyProvider::new(2));
        let retry = RetryProvider::new(provider, 3);
        let mut stream = retry.stream(&dummy_req()).await.unwrap();
        let chunk = stream.next().await.unwrap().unwrap();
        assert_eq!(chunk.delta, "ok");
    }

    #[tokio::test]
    async fn backoff_produces_increasing_delays() {
        // With random jitter the exact values vary, but the *range* of
        // successive attempts must trend upward.
        for _ in 0..20 {
            let d0 = backoff(0, 500, 30_000);
            let _d1 = backoff(1, 500, 30_000);
            let d2 = backoff(2, 500, 30_000);
            // Upper bound of attempt-0 range < lower bound of attempt-2 range
            assert!(d0 <= Duration::from_millis(500));
            assert!(d2 >= Duration::from_millis(1500));
        }
    }

    #[tokio::test]
    async fn backoff_respects_max() {
        let d = backoff(30, 500, 5_000);
        assert!(d <= Duration::from_millis(5_000));
    }

    #[tokio::test]
    async fn backoff_jitter_is_random() {
        // Call backoff with the same attempt many times; at least some values
        // must differ (probability of all-equal with a real RNG is negligible).
        let values: Vec<u64> = (0..50)
            .map(|_| backoff(3, 500, 30_000).as_millis() as u64)
            .collect();
        let first = values[0];
        assert!(
            values.iter().any(|&v| v != first),
            "jitter should produce varying values, got all {first}"
        );
    }

    // ── embed retry tests ─────────────────────────────────────────

    struct EmbedOkProvider;

    #[async_trait]
    impl Provider for EmbedOkProvider {
        async fn chat(&self, _: &ChatRequest) -> Result<ChatResponse> {
            unimplemented!()
        }

        async fn stream(&self, _: &ChatRequest) -> Result<BoxStream<'static, Result<StreamChunk>>> {
            unimplemented!()
        }

        async fn embed(&self, req: &EmbeddingRequest) -> Result<EmbeddingResponse> {
            Ok(EmbeddingResponse {
                model: req.model.clone(),
                data: vec![crate::Embedding {
                    index: 0,
                    embedding: vec![0.1, 0.2],
                }],
                usage: None,
            })
        }
    }

    /// A provider that only implements chat/stream — embed uses the default unsupported.
    struct EmbedUnsupportedProvider;

    #[async_trait]
    impl Provider for EmbedUnsupportedProvider {
        async fn chat(&self, _: &ChatRequest) -> Result<ChatResponse> {
            Ok(ChatResponse::default())
        }

        async fn stream(&self, _: &ChatRequest) -> Result<BoxStream<'static, Result<StreamChunk>>> {
            Ok(Box::pin(stream::empty()))
        }
        // embed() is NOT overridden — uses default Unsupported
    }

    #[tokio::test]
    async fn retry_provider_delegates_embed() {
        let provider = Arc::new(EmbedOkProvider);
        let retry = RetryProvider::new(provider, 0);
        let resp = retry
            .embed(&EmbeddingRequest::new("test", "hello"))
            .await
            .expect("embed should succeed through RetryProvider");
        assert_eq!(resp.data.len(), 1);
        assert_eq!(resp.data[0].embedding, vec![0.1, 0.2]);
    }

    #[tokio::test]
    async fn retry_provider_keeps_unsupported_for_chat_only_provider() {
        let provider = Arc::new(EmbedUnsupportedProvider);
        let retry = RetryProvider::new(provider, 0);
        let err = retry
            .embed(&EmbeddingRequest::new("test", "hello"))
            .await
            .unwrap_err();
        assert!(
            matches!(err, LlmError::Unsupported { .. }),
            "chat-only provider should return Unsupported through RetryProvider, got: {err:?}"
        );
    }

    struct FlakyEmbedProvider {
        fail_count: Arc<std::sync::atomic::AtomicU32>,
        max_fails: u32,
    }

    impl FlakyEmbedProvider {
        fn new(max_fails: u32) -> Self {
            Self {
                fail_count: Arc::new(std::sync::atomic::AtomicU32::new(0)),
                max_fails,
            }
        }
    }

    #[async_trait]
    impl Provider for FlakyEmbedProvider {
        async fn chat(&self, _: &ChatRequest) -> Result<ChatResponse> {
            unimplemented!()
        }

        async fn stream(&self, _: &ChatRequest) -> Result<BoxStream<'static, Result<StreamChunk>>> {
            unimplemented!()
        }

        async fn embed(&self, req: &EmbeddingRequest) -> Result<EmbeddingResponse> {
            let fails = self
                .fail_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if fails < self.max_fails {
                Err(LlmError::Api {
                    status: 502,
                    message: "bad gateway".into(),
                })
            } else {
                Ok(EmbeddingResponse {
                    model: req.model.clone(),
                    data: vec![crate::Embedding {
                        index: 0,
                        embedding: vec![1.0],
                    }],
                    usage: None,
                })
            }
        }
    }

    #[tokio::test]
    async fn retry_provider_retries_transient_embedding_failure() {
        let provider = Arc::new(FlakyEmbedProvider::new(1)); // fail 1st, succeed 2nd
        let retry = RetryProvider::new(provider, 1);
        let resp = retry
            .embed(&EmbeddingRequest::new("test", "hello"))
            .await
            .expect("embed should succeed after retry");
        assert_eq!(resp.data[0].embedding, vec![1.0]);
    }
}
