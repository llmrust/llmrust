//! Prompt-cache 断点策略（CNT-001 / `CAP-005`）。
//!
//! 本模块**独立于任何 provider 文件**存在，原因有二：
//! 1. `src/types.rs` 与 `src/providers/anthropic.rs` 均在 `tests/hotspot_ledger.json`
//!    的热点台账上（SPCC §4.4：**只许缩小、不许增长**），故新能力一律落在新文件；
//! 2. 缓存能力后续还要扩到目录/价格（`CAP-006`/`CAP-007`），先立模块再长。
//!
//! 分工：
//! - **策略面**（公开）：[`CacheRetention`] 三档 + [`CachePolicy`]，调用方只说"要不要、哪一档"；
//! - **打点面**（库内约定）：断点落在 `system` 末块 / 最后一个工具定义 / 最后一条消息的末块，
//!   调用方**不逐块指定**。

use serde::{Deserialize, Serialize};

use crate::providers::anthropic::{
    AnthropicContentBlock, AnthropicMessage, AnthropicMessageContent, AnthropicTool,
};
use crate::types::ChatRequest;

/// Anthropic（与 Bedrock Claude）对 `cache_control` 标记的**硬上限**。
///
/// 上游原话：*"A maximum of 4 blocks with cache_control may be provided."*
/// 本地即按此设计，不把非法请求发出去等上游拒。
pub const MAX_CACHE_BREAKPOINTS: usize = 4;

/// 库内约定：缓存断点最多落在这三处。
const CONVENTION_BREAKPOINTS: usize = 3;

// 编译期自证：约定断点数**恒不超**硬上限 —— 故运行期无需再挡。
// 若将来放开"调用方自定义断点"（可超 4），须在**构造期**补 Err 门（见 #205 提卡方案 §三）。
const _: () = assert!(CONVENTION_BREAKPOINTS <= MAX_CACHE_BREAKPOINTS);

/// Prompt-cache 保留档位（三档，钉死取值域；与 pi 的 `CacheRetention` 同构）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CacheRetention {
    /// 不发送任何缓存标记（默认）。
    #[default]
    None,
    /// 默认档（Anthropic = 5 分钟）。
    Short,
    /// 长档（Anthropic = 1 小时；仅当该模型支持时才应指定）。
    Long,
}

/// Prompt-cache 策略。
///
/// **断点位置由库内约定决定**，调用方只说"要不要、哪一档"。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachePolicy {
    /// 保留档位。
    pub retention: CacheRetention,
}

impl CachePolicy {
    /// 以给定档位构造策略。
    pub fn new(retention: CacheRetention) -> Self {
        Self { retention }
    }

    /// 是否真的需要发送缓存断点。
    pub fn is_enabled(&self) -> bool {
        !matches!(self.retention, CacheRetention::None)
    }
}

/// 按库内约定施加缓存断点：
/// ① `system` 末块（唯一途径：把 system 由字符串升级为内容块数组）
/// ② **最后一个工具定义**
/// ③ **最后一条消息的末块**（纯字符串内容升级为块数组才能挂标记）
///
/// 返回处置后的 `system`；未启用策略时原样返回（**发出字节不变**）。
pub(crate) fn apply_cache_policy(
    req: &ChatRequest,
    system: Option<String>,
    messages: &mut [AnthropicMessage],
    tools: Option<&mut Vec<AnthropicTool>>,
) -> Option<AnthropicSystem> {
    let Some(control) = req
        .cache
        .as_ref()
        .filter(|policy| policy.is_enabled())
        .and_then(|policy| AnthropicCacheControl::from_retention(policy.retention))
    else {
        return system.map(AnthropicSystem::Text);
    };

    // ① system 末块
    let system = system.map(|text| {
        AnthropicSystem::Blocks(vec![AnthropicContentBlock::text_cached(
            text,
            control.clone(),
        )])
    });

    // ② 最后一个工具定义
    if let Some(tools) = tools {
        if let Some(last) = tools.last_mut() {
            last.cache_control = Some(control.clone());
        }
    }

    // ③ 最后一条消息的末块
    if let Some(last) = messages.last_mut() {
        match &mut last.content {
            AnthropicMessageContent::Text(text) => {
                let text = std::mem::take(text);
                last.content =
                    AnthropicMessageContent::Blocks(vec![AnthropicContentBlock::text_cached(
                        text, control,
                    )]);
            }
            AnthropicMessageContent::Blocks(blocks) => {
                if let Some(block) = blocks.last_mut() {
                    *block.cache_control_mut() = Some(control);
                }
            }
        }
    }

    system
}

// ---- 自 `providers/anthropic.rs` 迁入（CNT-001 / CAP-005：热点文件不增长）----

/// 块级缓存标记。`{"type":"ephemeral"}`，长档时附 `"ttl":"1h"`。
#[derive(Debug, Clone, Serialize)]
pub(crate) struct AnthropicCacheControl {
    #[serde(rename = "type")]
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    ttl: Option<&'static str>,
}

impl AnthropicCacheControl {
    /// 按保留档位构造标记（`Short` → 默认 5 分钟档；`Long` → 1 小时档）。
    pub(crate) fn from_retention(retention: crate::cache_control::CacheRetention) -> Option<Self> {
        match retention {
            crate::cache_control::CacheRetention::None => None,
            crate::cache_control::CacheRetention::Short => Some(Self {
                kind: "ephemeral",
                ttl: None,
            }),
            crate::cache_control::CacheRetention::Long => Some(Self {
                kind: "ephemeral",
                ttl: Some("1h"),
            }),
        }
    }
}

impl AnthropicContentBlock {
    /// 带缓存标记的文本块（`CNT-001` / `CAP-005` 约定的落点）。
    pub(crate) fn text_cached(text: String, cache_control: AnthropicCacheControl) -> Self {
        AnthropicContentBlock::Text {
            text,
            cache_control: Some(cache_control),
        }
    }
    /// 该块的缓存标记（`None` = 未标记）。
    pub(crate) fn cache_control_mut(&mut self) -> &mut Option<AnthropicCacheControl> {
        match self {
            AnthropicContentBlock::Text { cache_control, .. }
            | AnthropicContentBlock::Image { cache_control, .. }
            | AnthropicContentBlock::ToolUse { cache_control, .. }
            | AnthropicContentBlock::ToolResult { cache_control, .. } => cache_control,
        }
    }
}

/// Claude 的 `system` 既可以是纯字符串，也可以是**内容块数组** ——
/// 后者是给 system 打缓存断点的唯一途径（CNT-001 / `CAP-005`）。
#[derive(Serialize)]
#[serde(untagged)]
pub(crate) enum AnthropicSystem {
    Text(String),
    Blocks(Vec<AnthropicContentBlock>),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::anthropic::build_body_json;
    use crate::types::{Message, Tool};

    fn cached_request(retention: CacheRetention) -> ChatRequest {
        let mut req = ChatRequest::from_messages(
            "claude",
            vec![Message::system("be brief"), Message::user("hi")],
        );
        req.tools = Some(vec![Tool::function(
            "lookup",
            Some("look something up".to_string()),
            serde_json::json!({"type": "object"}),
        )]);
        req.cache = Some(CachePolicy::new(retention));
        req
    }

    /// 未启用策略时：**一个标记都不发**，且 `system` 仍是纯字符串（与旧行为逐字节一致）。
    #[test]
    fn cache_policy_absent_sends_no_marker_and_keeps_system_as_string() {
        let mut req = cached_request(CacheRetention::Short);
        req.cache = None;
        let json = build_body_json(&req);
        assert!(
            !json.contains("cache_control"),
            "未启用策略时不得出现标记：{json}"
        );
        assert!(
            json.contains(r#""system":"be brief""#),
            "system 应保持字符串形态：{json}"
        );
    }

    /// `CacheRetention::None` 与不传策略等效（钉死取值域边界）。
    #[test]
    fn cache_policy_none_is_equivalent_to_absent() {
        let req = cached_request(CacheRetention::None);
        let json = build_body_json(&req);
        assert!(!json.contains("cache_control"));
        assert!(json.contains(r#""system":"be brief""#));
    }

    /// 短档：按库内约定打 **3** 个断点，且不带 `ttl`（Anthropic 默认 5 分钟档）。
    #[test]
    fn cache_policy_short_marks_three_convention_breakpoints() {
        let req = cached_request(CacheRetention::Short);
        let json = build_body_json(&req);
        assert_eq!(
            json.matches(r#""cache_control""#).count(),
            CONVENTION_BREAKPOINTS,
            "断点数应等于约定值：{json}"
        );
        assert!(json.contains(r#""type":"ephemeral""#), "{json}");
        assert!(!json.contains(r#""ttl""#), "短档不得带 ttl：{json}");
        assert!(
            json.contains(r#""system":[{"#),
            "system 应为块数组（否则无处挂标记）：{json}"
        );
    }

    /// 长档：标记附 `"ttl":"1h"`。
    #[test]
    fn cache_policy_long_adds_one_hour_ttl() {
        let req = cached_request(CacheRetention::Long);
        let json = build_body_json(&req);
        assert!(json.contains(r#""ttl":"1h""#), "{json}");
        assert_eq!(
            json.matches(r#""cache_control""#).count(),
            CONVENTION_BREAKPOINTS
        );
    }

    /// 无 system 时：断点落在"工具 + 末条消息"两处，不因缺 system 报错或补空块。
    #[test]
    fn cache_policy_without_system_marks_tool_and_last_message_only() {
        let mut req = cached_request(CacheRetention::Short);
        req.messages
            .retain(|m| !matches!(m.role, crate::types::Role::System));
        let json = build_body_json(&req);
        assert!(
            !json.contains(r#""system""#),
            "无 system 时不应出现该键：{json}"
        );
        assert_eq!(json.matches(r#""cache_control""#).count(), 2, "{json}");
    }

    /// 断点数**恒不超**硬上限（编译期已 `assert!`；此处留人可读的运行期留痕）。
    #[test]
    fn cache_policy_marks_at_most_convention_breakpoints() {
        let req = cached_request(CacheRetention::Short);
        let json = build_body_json(&req);
        assert!(json.matches(r#""cache_control""#).count() <= MAX_CACHE_BREAKPOINTS);
    }
}
