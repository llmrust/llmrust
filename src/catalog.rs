//! 官方服务商目录（`CNT-001` / `CAP-006` R3）。
//!
//! **两个正交维度**（设计输入 `#205` §3-1）：
//! - [`ModelSpec`] —— **谁造的模型**（厂商、窗口、上限、生命周期）；
//! - [`AccessPath`] —— **从哪接**（协议、端点、密钥环境变量、区域/渠道、可接模型清单）。
//!
//! 分开建模的理由（实测）：接入路径的前缀**不是**模型厂商。例如聚合表里 `dashscope/glm-5.1`
//! 指"智谱的模型经阿里百炼接入"，而不是"阿里的模型"；同一个 `glm-5.1` 可同时存在于
//! `dashscope/...`（百炼）与 `zai/...`（智谱官方）两条路径下。
//!
//! # 边界（Owner 令「只支持官方」）
//!
//! - **只接**厂商**官方端点**与**官方网关**；
//! - **不接**第三方转售 / 托管 / 聚合（`novita` / `deepinfra` / `azure_ai` / `bedrock` /
//!   `openrouter` / `vercel_ai_gateway` 等）；
//! - **不新增协议族**（§1.6 非目标读法 (a)）：本目录只使用**既有** [`Protocol`] 变体，
//!   不新增 `src/providers/*.rs`。需要新协议变体的接入（如 OpenCode Zen 的 `zen`）**转出**另单。

use serde::{Deserialize, Serialize};

/// 既有协议族（与 `src/providers/*` 一一对应；**新增变体须单独立卡并修订 §1.6**）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    /// OpenAI 兼容（`/v1/chat/completions`）。
    OpenAi,
    /// Anthropic Messages（`/v1/messages`）。
    Anthropic,
    /// OpenAI Responses API。
    Responses,
    /// Google Gemini。
    Google,
    /// Ollama 本地。
    Ollama,
}

/// 模型生命周期（照 maka `ModelMetadata` 的取值域收敛）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    /// 在售。
    #[default]
    Active,
    /// 测试档。
    Beta,
    /// 已宣布废弃，仍可用。
    Deprecated,
    /// 已下线（目录保留以解释历史请求）。
    Retired,
}

/// **维度一：模型本体**（谁造的、多大）。
///
/// `Serialize` only: the catalog is compile-time `const` data, never decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ModelSpec {
    /// 规范模型名（如 `glm-5.1`）。
    pub id: &'static str,
    /// 造它的厂商（如 `zhipu`）——**不是**接入平台。
    pub vendor: &'static str,
    /// 上下文窗口（tokens）。
    pub context_window: u32,
    /// 最大输出（tokens）。
    pub max_output: u32,
    /// 生命周期。
    pub lifecycle: Lifecycle,
}

/// 区域（同一厂商的国内/国际端点往往不同）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Region {
    /// 中国大陆。
    Cn,
    /// 国际。
    Global,
}

/// 渠道（按量付费 vs 订阅计划；两者端点与计费不同）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    /// 按量付费标准 API。
    Api,
    /// 官方网关/订阅档（如 CommandCode 的官方 Provider 档）。
    Gateway,
}

/// **维度二：接入路径**（从哪接、怎么认证、能接哪些模型）。
///
/// `Serialize` only: compile-time `const` data, never decoded (`&'static [&'static str]`
/// cannot implement `Deserialize`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct AccessPath {
    /// 路径标识（如 `zai-global` / `mimo-anthropic`）。
    pub id: &'static str,
    /// 走哪个协议适配器。
    pub protocol: Protocol,
    /// 端点基址。
    pub base_url: &'static str,
    /// 密钥环境变量名。
    pub key_env: &'static str,
    /// 区域。
    pub region: Region,
    /// 渠道。
    pub channel: Channel,
    /// 该路径可接的模型清单（指向 [`ModelSpec::id`]）。
    pub models: &'static [&'static str],
}

/// 模型目录（维度一）。
///
/// 只收**本组织需要的**常用模型；厂商官方定价与窗口以各自官方文档为准，
/// **逐模型**记录（读价倍率实测跨度 0.0083× ~ 0.5×，禁止全局默认倍率 —— 见 `#205` §3-2）。
pub const MODELS: &[ModelSpec] = &[
    ModelSpec {
        id: "claude-sonnet-4-6",
        vendor: "anthropic",
        context_window: 200_000,
        max_output: 64_000,
        lifecycle: Lifecycle::Active,
    },
    ModelSpec {
        id: "gpt-5.6",
        vendor: "openai",
        context_window: 400_000,
        max_output: 128_000,
        lifecycle: Lifecycle::Active,
    },
    ModelSpec {
        id: "deepseek-v4-pro",
        vendor: "deepseek",
        context_window: 1_000_000,
        max_output: 384_000,
        lifecycle: Lifecycle::Active,
    },
    ModelSpec {
        id: "qwen3.7-max",
        vendor: "alibaba",
        context_window: 262_144,
        max_output: 32_768,
        lifecycle: Lifecycle::Active,
    },
    ModelSpec {
        id: "glm-5.1",
        vendor: "zhipu",
        context_window: 202_745,
        max_output: 131_072,
        lifecycle: Lifecycle::Active,
    },
    ModelSpec {
        id: "mimo-v2.5-pro",
        vendor: "xiaomi",
        context_window: 262_144,
        max_output: 32_768,
        lifecycle: Lifecycle::Active,
    },
    ModelSpec {
        id: "step-3.7-flash",
        vendor: "stepfun",
        context_window: 262_144,
        max_output: 256_000,
        lifecycle: Lifecycle::Active,
    },
    ModelSpec {
        id: "grok-4.6",
        vendor: "xai",
        context_window: 500_000,
        max_output: 131_072,
        lifecycle: Lifecycle::Active,
    },
];

/// 接入路径目录（维度二）。**只列官方端点/官方网关。**
pub const ACCESS_PATHS: &[AccessPath] = &[
    AccessPath {
        id: "anthropic-official",
        protocol: Protocol::Anthropic,
        base_url: "https://api.anthropic.com",
        key_env: "ANTHROPIC_API_KEY",
        region: Region::Global,
        channel: Channel::Api,
        models: &["claude-sonnet-4-6"],
    },
    AccessPath {
        id: "openai-official",
        protocol: Protocol::OpenAi,
        base_url: "https://api.openai.com/v1",
        key_env: "OPENAI_API_KEY",
        region: Region::Global,
        channel: Channel::Api,
        models: &["gpt-5.6"],
    },
    AccessPath {
        id: "deepseek-openai",
        protocol: Protocol::OpenAi,
        base_url: "https://api.deepseek.com",
        key_env: "DEEPSEEK_API_KEY",
        region: Region::Cn,
        channel: Channel::Api,
        models: &["deepseek-v4-pro"],
    },
    AccessPath {
        id: "deepseek-anthropic",
        protocol: Protocol::Anthropic,
        base_url: "https://api.deepseek.com/anthropic",
        key_env: "DEEPSEEK_API_KEY",
        region: Region::Cn,
        channel: Channel::Api,
        models: &["deepseek-v4-pro"],
    },
    AccessPath {
        id: "qwen-cn",
        protocol: Protocol::OpenAi,
        base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
        key_env: "QWEN_API_KEY",
        region: Region::Cn,
        channel: Channel::Api,
        models: &["qwen3.7-max"],
    },
    AccessPath {
        id: "qwen-global",
        protocol: Protocol::OpenAi,
        base_url: "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
        key_env: "QWEN_API_KEY",
        region: Region::Global,
        channel: Channel::Api,
        models: &["qwen3.7-max"],
    },
    AccessPath {
        id: "glm-cn",
        protocol: Protocol::OpenAi,
        base_url: "https://open.bigmodel.cn/api/paas/v4",
        key_env: "GLM_API_KEY",
        region: Region::Cn,
        channel: Channel::Api,
        models: &["glm-5.1"],
    },
    AccessPath {
        id: "zai-global",
        protocol: Protocol::OpenAi,
        base_url: "https://api.z.ai/api/paas/v4",
        key_env: "ZAI_API_KEY",
        region: Region::Global,
        channel: Channel::Api,
        models: &["glm-5.1"],
    },
    AccessPath {
        id: "mimo-api",
        protocol: Protocol::OpenAi,
        base_url: "https://api.xiaomimimo.com/v1",
        key_env: "MIMO_API_KEY",
        region: Region::Cn,
        channel: Channel::Api,
        models: &["mimo-v2.5-pro"],
    },
    AccessPath {
        id: "mimo-anthropic",
        protocol: Protocol::Anthropic,
        base_url: "https://api.xiaomimimo.com/anthropic",
        key_env: "MIMO_API_KEY",
        region: Region::Cn,
        channel: Channel::Api,
        models: &["mimo-v2.5-pro"],
    },
    AccessPath {
        id: "stepfun-api",
        protocol: Protocol::OpenAi,
        base_url: "https://api.stepfun.com/v1",
        key_env: "STEPFUN_API_KEY",
        region: Region::Cn,
        channel: Channel::Api,
        models: &["step-3.7-flash"],
    },
    AccessPath {
        id: "stepfun-api-anthropic",
        protocol: Protocol::Anthropic,
        base_url: "https://api.stepfun.com",
        key_env: "STEPFUN_API_KEY",
        region: Region::Cn,
        channel: Channel::Api,
        models: &["step-3.7-flash"],
    },
    AccessPath {
        id: "xai-official",
        protocol: Protocol::OpenAi,
        base_url: "https://api.x.ai/v1",
        key_env: "XAI_API_KEY",
        region: Region::Global,
        channel: Channel::Api,
        models: &["grok-4.6"],
    },
    AccessPath {
        id: "commandcode-provider-openai",
        protocol: Protocol::OpenAi,
        base_url: "https://api.commandcode.ai/provider/v1",
        key_env: "CC_API_KEY",
        region: Region::Global,
        channel: Channel::Gateway,
        models: &["claude-sonnet-4-6", "gpt-5.6"],
    },
    AccessPath {
        id: "commandcode-provider-anthropic",
        protocol: Protocol::Anthropic,
        base_url: "https://api.commandcode.ai/provider",
        key_env: "CC_API_KEY",
        region: Region::Global,
        channel: Channel::Gateway,
        models: &["claude-sonnet-4-6"],
    },
];

/// 按 id 查模型。
pub fn model(id: &str) -> Option<&'static ModelSpec> {
    MODELS.iter().find(|m| m.id == id)
}

/// 按 id 查接入路径。
pub fn access_path(id: &str) -> Option<&'static AccessPath> {
    ACCESS_PATHS.iter().find(|p| p.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn model_ids_are_unique() {
        let mut seen = HashSet::new();
        for m in MODELS {
            assert!(seen.insert(m.id), "duplicate model id: {}", m.id);
        }
    }

    #[test]
    fn access_path_ids_are_unique() {
        let mut seen = HashSet::new();
        for p in ACCESS_PATHS {
            assert!(seen.insert(p.id), "duplicate access path id: {}", p.id);
        }
    }

    /// 目录自洽：每条路径引用的模型必须存在于 [`MODELS`]（防止写错名字的"幽灵模型"）。
    #[test]
    fn every_access_path_model_exists() {
        for p in ACCESS_PATHS {
            for id in p.models {
                assert!(
                    model(id).is_some(),
                    "access path `{}` references unknown model `{id}`",
                    p.id
                );
            }
        }
    }

    /// 目录自洽：模型不得引用不存在的接入路径（反向覆盖，与 #209 的反向门同精神）。
    #[test]
    fn every_model_is_reachable_through_some_path() {
        for m in MODELS {
            assert!(
                ACCESS_PATHS.iter().any(|p| p.models.contains(&m.id)),
                "model `{}` is in MODELS but no access path serves it",
                m.id
            );
        }
    }

    /// 「只支持官方」的负例侧：目录里不得出现已知第三方转售/聚合域。
    #[test]
    fn catalog_excludes_third_party_aggregators() {
        const FORBIDDEN: &[&str] = &[
            "novita",
            "deepinfra",
            "azure_ai",
            "bedrock",
            "openrouter",
            "vercel_ai_gateway",
            "huggingface",
            "nvidia",
        ];
        for p in ACCESS_PATHS {
            for bad in FORBIDDEN {
                assert!(
                    !p.base_url.contains(bad) && !p.id.contains(bad),
                    "third-party aggregator `{bad}` must not appear in the catalog (path `{}`)",
                    p.id
                );
            }
        }
    }

    /// 窗口与输出上限必须为正（防手滑写 0）。
    #[test]
    fn model_windows_are_positive() {
        for m in MODELS {
            assert!(m.context_window > 0, "{}: context_window must be > 0", m.id);
            assert!(m.max_output > 0, "{}: max_output must be > 0", m.id);
            assert!(
                m.max_output <= m.context_window,
                "{}: max_output must not exceed context_window",
                m.id
            );
        }
    }
}
