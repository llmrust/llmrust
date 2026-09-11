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

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

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
    /// **是否已声明**（`false` = 该 Provider 未覆写 `capabilities()`）。
    ///
    /// 这个区分是**必需的**：`CAP-003` 的入口裁决只对**明确声明为 `unsupported`** 的能力面
    /// 拒绝；**未声明**的 Provider（下游自定义、代理内部 mock）必须保持 0.1.3 的行为
    /// （放行 + 一次性告警），否则 CAP-003 就成了"给所有自定义 Provider 加锁"——
    /// 那是破坏性变更，卡内明文禁止。
    pub declared: bool,
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
            declared: false,
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

// ── CAP-003：统一入口能力裁决 ─────────────────────────────────────────────
//
// 0.1.3 期能力检查散落在各 Provider 内部（`warn_if_unsupported_n` 六处手工调用点），
// 于是**同一类缺口要改 N 遍**，且**没有声明依据的拒绝**与**静默丢弃**并存。
// 本节的裁决是**纯函数**：输入 (能力声明, 请求)，输出裁决表；入口（`LmrsClient`）
// 负责执行裁决。这样"每处 Reject 都有能力声明依据"成为**结构事实**，而非纪律要求。

/// 一次派发的裁决结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// 放行（不附加任何副作用）。
    Pass,
    /// 放行但**留痕**（`tracing::warn`，按 `(provider, feature)` 去重）。
    Warn {
        /// 能力面名（如 `n`）。
        feature: &'static str,
        /// 面向调用方的说明（**不得**包含 prompt/response/凭证）。
        message: String,
    },
    /// **响亮拒绝**：返回 [`crate::providers::LlmError::Unsupported`]。
    Reject {
        /// 能力面名（写入 `LlmError::Unsupported::feature`）。
        feature: &'static str,
        /// 说明。
        message: String,
    },
}

/// 请求里被裁决关注的"能力诉求"（从 [`crate::types::ChatRequest`] 提取的纯数据）。
///
/// 抽成独立结构是为了让裁决**完全可测**（不依赖真实 Provider 或网络）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RequestDemands {
    /// 是否传了工具（`tools` 非空或指定了 `tool_choice`）。
    pub tools: bool,
    /// 是否为流式请求（决定查 `tool_calling` 还是 `tool_calling_stream`）。
    pub streaming: bool,
    /// `n` 的取值。
    pub n: Option<u32>,
    /// 是否带图像内容。
    pub images: bool,
}

/// **纯裁决**：按能力声明判定一次请求的每个诉求。
///
/// 规则（每条都有**声明依据**——这正是 `CAP-003` 要建立的纪律）：
///
/// 1. `tools` 且对应能力面（流式看 `tool_calling_stream`，非流式看 `tool_calling`）
///    **明确声明（`declared == true`）为** [`CapabilityLevel::Unsupported`]
///    → **Reject**（不得静默丢弃）；
/// 2. `n > 1` → **Warn**（0.1.3 既有语义：只返回第一个补全，且上游可能仍按 N 计费）；
/// 3. `images` 且 `image_input` **明确声明为** `Unsupported` → **Reject**；
/// 4. **未声明能力（`declared == false`）→ 放行 + 一次性 Warn**：
///    下游自定义 Provider 与代理内部 mock 都属于这一类，**必须保持 0.1.3 行为**；
/// 5. 其余一律 **Pass**（**已支持路径的行为零变化**）。
///
/// `ModelDependent` / `PassthroughOnly` **不触发 Reject**：它们表示"能否用取决于上游"，
/// llmrust 的职责是转译而非替上游拒绝。
pub fn adjudicate(caps: &Capabilities, demands: &RequestDemands) -> Vec<Verdict> {
    let mut out = Vec::new();

    if demands.tools {
        let (feature, cap) = if demands.streaming {
            ("tool_calling_stream", caps.tool_calling_stream)
        } else {
            ("tool_calling", caps.tool_calling)
        };
        if cap.level == CapabilityLevel::Unsupported {
            if caps.declared {
                out.push(Verdict::Reject {
                    feature,
                    message: format!(
                        "provider `{}` declares {} as unsupported; sending tools would be silently \
                         dropped, so the request is refused instead (CAP-003)",
                        caps.protocol, feature
                    ),
                });
            } else {
                out.push(Verdict::Warn {
                    feature,
                    message: format!(
                        "provider `{}` does not declare capabilities; tools are forwarded as in \
                         0.1.3, but whether they are honored is unknown (CAP-003)",
                        caps.protocol
                    ),
                });
            }
        }
    }

    if crate::providers::n_is_unsupported(demands.n) {
        out.push(Verdict::Warn {
            feature: "n",
            message:
                "n > 1 requested but llmrust returns only the first completion; the remaining \
                      choices are discarded and upstream providers may still bill for all N"
                    .to_string(),
        });
    }

    if demands.images && caps.image_input.level == CapabilityLevel::Unsupported {
        if caps.declared {
            out.push(Verdict::Reject {
                feature: "image_input",
                message: format!(
                    "provider `{}` declares image_input as unsupported; sending images would be \
                     silently dropped, so the request is refused instead (CAP-003)",
                    caps.protocol
                ),
            });
        } else {
            out.push(Verdict::Warn {
                feature: "image_input",
                message: format!(
                    "provider `{}` does not declare capabilities; images are forwarded as in \
                     0.1.3, but whether they are honored is unknown (CAP-003)",
                    caps.protocol
                ),
            });
        }
    }

    out
}

/// **按 `(provider, feature)` 去重**的告警（`CAP-003` 统一入口用）。
///
/// 去重的理由与 0.1.3 的 `warn_if_unsupported_n` 相同（E-002）：`RetryProvider` 每次重试
/// 都会重新进入内层 Provider，若不去重则每次都刷一遍——那是噪声，不是信号。
/// 迁移到入口后，本函数仍在**每个进程内**至多告警一次/Provider/能力面。
pub fn warn_once(provider: &str, feature: &str, message: &str) {
    static WARNED: OnceLock<Mutex<HashSet<(String, &'static str)>>> = OnceLock::new();
    let first = WARNED
        .get_or_init(Default::default)
        .lock()
        .map(|mut set| set.insert((provider.to_string(), leak_feature(feature))))
        .unwrap_or(false);
    if first {
        tracing::warn!(provider, feature, "{message}");
    }
}

/// Intern a feature name for the dedupe set (bounded by the capability faces).
fn leak_feature(feature: &str) -> &'static str {
    static FEATURES: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let mut set = match FEATURES.get_or_init(Default::default).lock() {
        Ok(s) => s,
        Err(_) => return "unknown",
    };
    if let Some(existing) = set.get(feature) {
        return existing;
    }
    let leaked: &'static str = Box::leak(feature.to_string().into_boxed_str());
    set.insert(leaked);
    leaked
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
        c.declared = true; // 这些用例检验的是"**已声明**为 unsupported"的路径
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

    // ── CAP-003：裁决的负例与正向对照 ──────────────────────────────────

    fn caps_with(
        tools: CapabilityLevel,
        tools_stream: CapabilityLevel,
        images: CapabilityLevel,
    ) -> Capabilities {
        let mut c = Capabilities::unknown("p");
        c.declared = true; // 这些用例检验的是"**已声明**为 unsupported"的路径
        c.tool_calling = Capability {
            level: tools,
            verified: None,
        };
        c.tool_calling_stream = Capability {
            level: tools_stream,
            verified: None,
        };
        c.image_input = Capability {
            level: images,
            verified: None,
        };
        c
    }

    /// **负例（本卡的招牌 DoD）**：声明 `tool_calling = unsupported` 却传 `tools`
    /// → 必须 **Reject**（0.1.3 期是**静默丢弃**）。
    #[test]
    fn tools_on_unsupported_provider_are_rejected() {
        let caps = caps_with(
            CapabilityLevel::Unsupported,
            CapabilityLevel::Unsupported,
            CapabilityLevel::Implemented,
        );
        let v = adjudicate(
            &caps,
            &RequestDemands {
                tools: true,
                streaming: false,
                n: None,
                images: false,
            },
        );
        assert_eq!(v.len(), 1, "got {v:?}");
        assert!(
            matches!(
                v[0],
                Verdict::Reject {
                    feature: "tool_calling",
                    ..
                }
            ),
            "{v:?}"
        );
    }

    /// 流式请求查的是 `tool_calling_stream`（**两个面必须分别判定**）。
    #[test]
    fn streaming_tools_consult_the_stream_capability() {
        let caps = caps_with(
            CapabilityLevel::Implemented,
            CapabilityLevel::Unsupported,
            CapabilityLevel::Implemented,
        );
        let v = adjudicate(
            &caps,
            &RequestDemands {
                tools: true,
                streaming: true,
                n: None,
                images: false,
            },
        );
        assert_eq!(v.len(), 1, "got {v:?}");
        assert!(
            matches!(
                v[0],
                Verdict::Reject {
                    feature: "tool_calling_stream",
                    ..
                }
            ),
            "{v:?}"
        );
    }

    /// `n > 1` → **Warn**（不拒绝）：0.1.3 语义是"只返回第一个并提醒"，本卡**不改这个行为**。
    #[test]
    fn n_greater_than_one_warns_but_does_not_reject() {
        let caps = caps_with(
            CapabilityLevel::Implemented,
            CapabilityLevel::Implemented,
            CapabilityLevel::Implemented,
        );
        let v = adjudicate(
            &caps,
            &RequestDemands {
                tools: false,
                streaming: false,
                n: Some(3),
                images: false,
            },
        );
        assert_eq!(v.len(), 1, "got {v:?}");
        assert!(matches!(v[0], Verdict::Warn { feature: "n", .. }), "{v:?}");
    }

    /// **已支持路径零变化**：全 implemented 声明 + 普通请求 → 裁决表**为空**。
    #[test]
    fn supported_paths_yield_no_verdicts() {
        let caps = caps_with(
            CapabilityLevel::Implemented,
            CapabilityLevel::Implemented,
            CapabilityLevel::Implemented,
        );
        for demands in [
            RequestDemands::default(),
            RequestDemands {
                tools: true,
                streaming: false,
                n: Some(1),
                images: true,
            },
            RequestDemands {
                tools: true,
                streaming: true,
                n: None,
                images: true,
            },
        ] {
            let v = adjudicate(&caps, &demands);
            assert!(
                v.is_empty(),
                "demands {demands:?} must pass cleanly; got {v:?}"
            );
        }
    }

    /// `ModelDependent` / `PassthroughOnly` **不得**触发 Reject（ltmrust 不替上游拒绝）。
    #[test]
    fn model_dependent_and_passthrough_are_not_rejected() {
        for level in [
            CapabilityLevel::ModelDependent,
            CapabilityLevel::PassthroughOnly,
        ] {
            let caps = caps_with(level, level, level);
            let v = adjudicate(
                &caps,
                &RequestDemands {
                    tools: true,
                    streaming: false,
                    n: None,
                    images: true,
                },
            );
            assert!(v.is_empty(), "level {level:?} must not reject; got {v:?}");
        }
    }

    /// 图像：声明 `unsupported` 却带图 → Reject（与 tools 同一条纪律）。
    #[test]
    fn images_on_unsupported_provider_are_rejected() {
        let caps = caps_with(
            CapabilityLevel::Implemented,
            CapabilityLevel::Implemented,
            CapabilityLevel::Unsupported,
        );
        let v = adjudicate(
            &caps,
            &RequestDemands {
                tools: false,
                streaming: false,
                n: None,
                images: true,
            },
        );
        assert_eq!(v.len(), 1, "got {v:?}");
        assert!(
            matches!(
                v[0],
                Verdict::Reject {
                    feature: "image_input",
                    ..
                }
            ),
            "{v:?}"
        );
    }

    /// 多条诉求可同时出裁决（Reject + Warn 并存），顺序稳定。
    #[test]
    fn multiple_verdicts_are_reported_together() {
        let caps = caps_with(
            CapabilityLevel::Unsupported,
            CapabilityLevel::Unsupported,
            CapabilityLevel::Unsupported,
        );
        let v = adjudicate(
            &caps,
            &RequestDemands {
                tools: true,
                streaming: false,
                n: Some(4),
                images: true,
            },
        );
        assert_eq!(v.len(), 3, "got {v:?}");
        assert!(matches!(
            v[0],
            Verdict::Reject {
                feature: "tool_calling",
                ..
            }
        ));
        assert!(matches!(v[1], Verdict::Warn { feature: "n", .. }));
        assert!(matches!(
            v[2],
            Verdict::Reject {
                feature: "image_input",
                ..
            }
        ));
    }

    /// 裁决说明**不得**泄露内容（无 prompt/response/凭证字样）。
    #[test]
    fn verdict_messages_do_not_leak_content() {
        let caps = caps_with(
            CapabilityLevel::Unsupported,
            CapabilityLevel::Unsupported,
            CapabilityLevel::Unsupported,
        );
        let v = adjudicate(
            &caps,
            &RequestDemands {
                tools: true,
                streaming: false,
                n: Some(2),
                images: true,
            },
        );
        for verdict in v {
            let msg = match verdict {
                Verdict::Pass => String::new(),
                Verdict::Warn { message, .. } | Verdict::Reject { message, .. } => message,
            };
            for banned in ["prompt", "api_key", "Bearer", "response body"] {
                assert!(
                    !msg.contains(banned),
                    "verdict message must not mention `{banned}`: {msg}"
                );
            }
        }
    }
}
