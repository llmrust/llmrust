//! Provider 能力声明载体（`CNT-001` / `CAP-002`）。
//!
//! 0.1.3 期能力真相只活在 `docs/CAPABILITIES.md` 的 Markdown 表格里——**说错与漂移无机器可发现**。
//! 本模块为能力声明建立**代码载体**：`Provider::capabilities()` 的返回类型。
//!
//! # 五级验证层级（SPCC §6.2，沿用 0.1.3 并在此**建模**）
//!
//! | 级别 | 含义 |
//! |---|---|
//! | [`CapabilityLevel::Implemented`] | llmrust 已实现该字段/协议的映射 |
//! | [`CapabilityLevel::Verified`] | 且有**核验证据**（须附日期与证据类型） |
//! | [`CapabilityLevel::ModelDependent`] | 取决于上游模型，llmrust 只做转译 |
//! | [`CapabilityLevel::Unsupported`] | 明确不支持（失败要**响亮**，不得静默丢弃） |
//! | [`CapabilityLevel::PassthroughOnly`] | 仅原样透传，不做语义保证 |
//!
//! # 非破坏（`CAP-002` §禁止范围）
//!
//! - [`Capabilities`] 标注 `#[non_exhaustive]`（§5.3 不可协商项）：后续加字段不算破坏；
//! - `Provider::capabilities()` **带默认实现**，下游自定义 Provider **不实现也能编译**。

use serde::{Deserialize, Serialize};

/// 能力验证层级（五级，钉死取值域）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityLevel {
    /// 已实现映射。
    Implemented,
    /// 已实现**且有证据**（必须附 [`Verified`]）。
    Verified,
    /// 取决于上游模型。
    ModelDependent,
    /// 明确不支持（失败要响亮）。
    Unsupported,
    /// 仅透传，不做语义保证。
    PassthroughOnly,
}

/// 证据类型（`verified` 级别必须声明其一）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceKind {
    /// 本地夹具（录制/合成的报文）。
    LocalFixture,
    /// 真实端点（真凭据实跑）。
    LiveEndpoint,
}

/// `verified` 的证据本体：**日期 + 类型**（§6.2 明文要求）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Verified {
    /// 证据类型。
    pub kind: EvidenceKind,
    /// 核验日期（`YYYY-MM-DD`）。
    pub date: &'static str,
}

/// 单个能力的声明。
///
/// `verified` 必须携带证据；构造期即校验，**非法组合不可表达**（见 [`Capability::verified`]）。
///
/// **只 `Serialize` 不 `Deserialize`**：能力声明是**运行时真相**（`&'static str`），
/// 反序列化它没有意义（且 `&'static str` 无法安全借用反序列化输入）。方向是
/// **代码 → JSON/文档**（`CAP-004` 的三处一致门即按此方向校验）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Capability {
    /// 层级。
    pub level: CapabilityLevel,
    /// 仅当 `level == Verified` 时存在。
    pub verified: Option<Verified>,
}

impl Capability {
    /// 已实现（无证据）。
    pub const fn implemented() -> Self {
        Self {
            level: CapabilityLevel::Implemented,
            verified: None,
        }
    }

    /// 取决于上游模型。
    pub const fn model_dependent() -> Self {
        Self {
            level: CapabilityLevel::ModelDependent,
            verified: None,
        }
    }

    /// 不支持。
    pub const fn unsupported() -> Self {
        Self {
            level: CapabilityLevel::Unsupported,
            verified: None,
        }
    }

    /// 仅透传。
    pub const fn passthrough_only() -> Self {
        Self {
            level: CapabilityLevel::PassthroughOnly,
            verified: None,
        }
    }

    /// 已核验（**必须**给证据——没有"无证据的 verified"）。
    pub const fn verified(kind: EvidenceKind, date: &'static str) -> Self {
        Self {
            level: CapabilityLevel::Verified,
            verified: Some(Verified { kind, date }),
        }
    }

    /// 自洽：`Verified` 必须有证据，非 `Verified` 不得带证据。
    pub const fn is_coherent(&self) -> bool {
        match self.level {
            CapabilityLevel::Verified => self.verified.is_some(),
            _ => self.verified.is_none(),
        }
    }
}

/// 一个 Provider 的能力集合。
///
/// **`#[non_exhaustive]` 不可协商**（§5.3）：0.1.x 内新增能力字段不得成为破坏性变更。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Capabilities {
    /// 该 Provider 走哪个协议族（如 `openai-compatible`）。
    pub protocol: &'static str,
    /// 非流式 chat。
    pub chat: Capability,
    /// 流式 chat。
    pub stream: Capability,
    /// 工具调用（非流式）。
    pub tool_calling: Capability,
    /// 工具调用（流式）。
    pub tool_calling_stream: Capability,
    /// 图像输入。
    pub image_input: Capability,
    /// 嵌入。
    pub embeddings: Capability,
    /// reasoning（请求字段 + 流式 thinking + usage 映射）。
    pub reasoning: Capability,
    /// prompt cache（断点发送能力，`CAP-005`）。
    pub prompt_cache: Capability,
}

impl Capabilities {
    /// 最小构造：全 [`CapabilityLevel::Unsupported`]（下游 Provider 的默认声明）。
    ///
    /// 默认实现**故意保守**：未声明 = 不支持，而不是"默认支持"。
    /// 这与 §6.3"静默失败禁令"同向——宁可让调用方看见 `Unsupported`，不要让它以为成功。
    pub const fn unknown(protocol: &'static str) -> Self {
        Self {
            protocol,
            chat: Capability::unsupported(),
            stream: Capability::unsupported(),
            tool_calling: Capability::unsupported(),
            tool_calling_stream: Capability::unsupported(),
            image_input: Capability::unsupported(),
            embeddings: Capability::unsupported(),
            reasoning: Capability::unsupported(),
            prompt_cache: Capability::unsupported(),
        }
    }

    /// 自洽检查：每个字段的 `verified` 证据规则都成立。
    pub fn coherence_problems(&self) -> Vec<String> {
        let fields: [(&str, &Capability); 8] = [
            ("chat", &self.chat),
            ("stream", &self.stream),
            ("tool_calling", &self.tool_calling),
            ("tool_calling_stream", &self.tool_calling_stream),
            ("image_input", &self.image_input),
            ("embeddings", &self.embeddings),
            ("reasoning", &self.reasoning),
            ("prompt_cache", &self.prompt_cache),
        ];
        let mut problems = Vec::new();
        if self.protocol.trim().is_empty() {
            problems.push("protocol must not be empty".to_string());
        }
        for (name, cap) in fields {
            if !cap.is_coherent() {
                problems.push(format!(
                    "{name}: level is {:?} but verified evidence {} (verified requires evidence; \
                     non-verified must not carry any)",
                    cap.level,
                    if cap.verified.is_some() {
                        "is present"
                    } else {
                        "is absent"
                    }
                ));
            }
            if let Some(v) = cap.verified {
                if v.date.len() != 10 || v.date.as_bytes().get(4) != Some(&b'-') {
                    problems.push(format!(
                        "{name}: evidence date `{}` is not YYYY-MM-DD",
                        v.date
                    ));
                }
            }
        }
        problems
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_is_conservative_not_optimistic() {
        let c = Capabilities::unknown("openai-compatible");
        assert_eq!(c.chat.level, CapabilityLevel::Unsupported);
        assert_eq!(c.prompt_cache.level, CapabilityLevel::Unsupported);
        assert!(c.coherence_problems().is_empty());
    }

    #[test]
    fn verified_requires_evidence_and_others_forbid_it() {
        assert!(Capability::verified(EvidenceKind::LiveEndpoint, "2026-09-11").is_coherent());
        assert!(!Capability {
            level: CapabilityLevel::Verified,
            verified: None,
        }
        .is_coherent());
        assert!(!Capability {
            level: CapabilityLevel::Implemented,
            verified: Some(Verified {
                kind: EvidenceKind::LocalFixture,
                date: "2026-09-11"
            }),
        }
        .is_coherent());
    }

    #[test]
    fn coherence_reports_bad_dates() {
        let mut c = Capabilities::unknown("p");
        c.chat = Capability::verified(EvidenceKind::LocalFixture, "2026/09/11");
        let problems = c.coherence_problems();
        assert_eq!(problems.len(), 1, "got {problems:?}");
        assert!(problems[0].contains("not YYYY-MM-DD"), "{problems:?}");
    }

    #[test]
    fn coherence_reports_empty_protocol() {
        let c = Capabilities::unknown("  ");
        assert!(c
            .coherence_problems()
            .iter()
            .any(|p| p.contains("protocol")));
    }

    /// 序列化方向：代码 → JSON（`CAP-004` 的三处一致门用的就是这一方向）。
    ///
    /// **不做反序列化往返**：能力声明是运行时真相（`&'static str`），
    /// 反序列化它既无意义也无法安全借用输入；本测试钉住**输出形状**。
    #[test]
    fn capabilities_serialize_to_the_expected_shape() {
        let mut c = Capabilities::unknown("anthropic");
        c.prompt_cache = Capability::verified(EvidenceKind::LiveEndpoint, "2026-09-11");
        let v: serde_json::Value = serde_json::to_value(c).unwrap();

        assert_eq!(v["protocol"], "anthropic");
        assert_eq!(v["chat"]["level"], "unsupported");
        // verified 级别必须把证据一并序列化出来（日期 + 类型）。
        assert_eq!(v["prompt_cache"]["level"], "verified");
        assert_eq!(v["prompt_cache"]["verified"]["kind"], "live-endpoint");
        assert_eq!(v["prompt_cache"]["verified"]["date"], "2026-09-11");
    }

    /// `#[non_exhaustive]` 的存在性**无法在 crate 内直接断言**，故由
    /// `tests/capability_freeze.rs` 以"下游视角"证明（见 `CAP-004`）。
    #[test]
    fn levels_are_five_and_stable() {
        let all = [
            CapabilityLevel::Implemented,
            CapabilityLevel::Verified,
            CapabilityLevel::ModelDependent,
            CapabilityLevel::Unsupported,
            CapabilityLevel::PassthroughOnly,
        ];
        assert_eq!(all.len(), 5, "SPCC §6.2 pins exactly five levels");
    }
}
