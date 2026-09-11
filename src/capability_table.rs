//! 每模型能力/价格表（`CNT-001` / `CAP-007` R4）。
//!
//! 把"能力真相"从**散文**变成**机器可读**：`llmrust.models.json` 是唯一事实源，
//! `docs/CAPABILITIES.md` 里那段表格**由本模块渲染产出**，两者一旦漂移由
//! `tests/capability_table_guard.rs` 判红（人工改文档 = 红）。
//!
//! # 为什么要有这一层
//!
//! 能力此前只活在 `docs/CAPABILITIES.md` 的手写表格里——**说错与漂移没有任何机器能发现**
//! （与 llmrust `E-103`"未当场取证即陈述事实性细节"同源）。本模块提供：
//!
//! - [`ModelCapabilityTable::from_json`]：解析机读表（缺字段即错，不静默补默认值）；
//! - [`ModelCapabilityTable::render_markdown`]：**确定性**渲染（同一份数据永远产出同一段文本）；
//! - [`ModelCapabilityTable::consistency_problems`]：**表内自洽**检查（纯函数，可被负例直接驱动）。

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

/// 机读表解析/自洽错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableError {
    /// JSON 不合法。
    Json(String),
    /// 结构性缺字段。
    MissingField(String),
    /// 未识别的缓存模式。
    UnknownCacheMode(String),
}

impl std::fmt::Display for TableError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TableError::Json(m) => write!(f, "invalid models JSON: {m}"),
            TableError::MissingField(m) => write!(f, "missing required field: {m}"),
            TableError::UnknownCacheMode(m) => write!(f, "unknown cache mode: {m}"),
        }
    }
}

/// 缓存模式（逐模型数据，**不是全局常量**）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheMode {
    /// 无缓存。
    None,
    /// 自动命中（无需断点标记）。
    Auto,
    /// 必须显式打断点。
    Explicit,
    /// 两者皆可。
    Both,
}

/// 缓存规格。`min_cacheable_tokens = 0` 表示**官方未公布下限**（不是"无下限"）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CacheSpec {
    /// 模式。
    pub mode: CacheMode,
    /// 最小可缓存长度（tokens）；0 = 未公布。
    pub min_cacheable_tokens: u64,
    /// TTL 口径（枚举以字符串承载，逐家不同：`fixed_5m` / `none` / `unknown` …）。
    pub ttl: String,
    /// 淘汰机制（`expiry` / `lru` / `disk_units` / `unknown`）。
    pub eviction: String,
    /// 断点上限（有该字段即表示该家限制断点数，如 Anthropic = 4）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub breakpoint_limit: Option<u32>,
}

/// 价格（USD / 1k tokens）。三档缓存价**均为可选**：多数家写缓存不加价。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PriceRow {
    /// 输入价。
    pub prompt_per_1k: f64,
    /// 输出价。
    pub completion_per_1k: f64,
    /// 缓存命中（读）价。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_per_1k: Option<f64>,
    /// 写缓存 5 分钟档价。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_5m_per_1k: Option<f64>,
    /// 写缓存 1 小时档价。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_1h_per_1k: Option<f64>,
}

/// 一行 = 一个模型（`CAP-007` 要求的七类字段）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelCapability {
    /// 规范模型名。
    pub id: String,
    /// 厂商。
    pub provider: String,
    /// 协议。
    pub protocol: String,
    /// 上下文窗口。
    pub context_window: u64,
    /// 最大输出。
    pub max_output: u64,
    /// 生命周期（`active` / `beta` / `deprecated` / `retired`）。
    pub lifecycle: String,
    /// 价格（含缓存价）。
    pub pricing: PriceRow,
    /// 缓存规格。
    pub cache: CacheSpec,
    /// reasoning 支持度（`implemented` / `unsupported`）。
    pub reasoning: String,
    /// 工具调用。
    pub tool_calling: bool,
    /// 取证日期。
    pub verified_at: String,
    /// 来源。
    pub source: String,
}

/// 机读表。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelCapabilityTable {
    /// 表版本。
    pub version: String,
    /// 行。
    pub models: Vec<ModelCapability>,
}

impl ModelCapabilityTable {
    /// 从 JSON 文本解析。**缺字段即错**（不静默补默认值）。
    pub fn from_json(text: &str) -> Result<Self, TableError> {
        let table: ModelCapabilityTable =
            serde_json::from_str(text).map_err(|e| TableError::Json(e.to_string()))?;
        if table.models.is_empty() {
            return Err(TableError::MissingField("models (empty)".to_string()));
        }
        Ok(table)
    }

    /// 表内自洽检查（**纯函数**，供负例直接驱动）。
    ///
    /// 规则：
    /// 1. `mode == None` **不得**声明缓存读价（无缓存却给缓存价 = 矛盾）；
    /// 2. 价格非负（含缓存三档）；
    /// 3. `max_output ≤ context_window`；
    /// 4. 窗口 / 输出为正。
    ///
    /// 注：`min_cacheable_tokens = 0` / `ttl = "unknown"` **不算矛盾**——多家官方不公布下限，
    /// 把它判红会逼人编数字（台账原文："Do not inflate a number to turn a guard green."）。
    pub fn consistency_problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        for m in &self.models {
            let p = &m.pricing;
            if p.cache_read_per_1k.is_some() && m.cache.mode == CacheMode::None {
                problems.push(format!(
                    "{}: declares a cache read price but cache mode is `none` (contradiction)",
                    m.id
                ));
            }
            for (label, v) in [
                ("prompt_per_1k", Some(p.prompt_per_1k)),
                ("completion_per_1k", Some(p.completion_per_1k)),
                ("cache_read_per_1k", p.cache_read_per_1k),
                ("cache_write_5m_per_1k", p.cache_write_5m_per_1k),
                ("cache_write_1h_per_1k", p.cache_write_1h_per_1k),
            ] {
                if let Some(v) = v {
                    if !v.is_finite() || v < 0.0 {
                        problems.push(format!(
                            "{}: {label} must be finite and >= 0 (got {v})",
                            m.id
                        ));
                    }
                }
            }
            if m.context_window == 0 {
                problems.push(format!("{}: context_window must be > 0", m.id));
            }
            if m.max_output == 0 {
                problems.push(format!("{}: max_output must be > 0", m.id));
            }
            if m.max_output > m.context_window {
                problems.push(format!(
                    "{}: max_output ({}) must not exceed context_window ({})",
                    m.id, m.max_output, m.context_window
                ));
            }
        }
        problems
    }

    /// **确定性**渲染 `docs/CAPABILITIES.md` 中的每模型区块（不含哨兵行）。
    ///
    /// 同一份数据永远产出同一段文本——这正是"人工改动即红"能够成立的前提。
    pub fn render_markdown(&self) -> String {
        let mut sorted: Vec<&ModelCapability> = self.models.iter().collect();
        sorted.sort_by(|a, b| a.provider.cmp(&b.provider).then(a.id.cmp(&b.id)));

        let mut out = String::new();
        let _ = writeln!(out, "| Model | Provider | Context | Max out | Cache | Read price | Write 5m | Write 1h | Reasoning | Tools | Lifecycle |");
        let _ = writeln!(out, "|---|---|---:|---:|---|---:|---:|---:|---|---|---|");
        for m in sorted {
            let _ = writeln!(
                out,
                "| `{}` | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
                m.id,
                m.provider,
                m.context_window,
                m.max_output,
                cache_label(&m.cache),
                price_label(m.pricing.cache_read_per_1k),
                price_label(m.pricing.cache_write_5m_per_1k),
                price_label(m.pricing.cache_write_1h_per_1k),
                m.reasoning,
                if m.tool_calling { "yes" } else { "no" },
                m.lifecycle,
            );
        }
        out
    }
}

fn price_label(v: Option<f64>) -> String {
    match v {
        None => "—".to_string(),
        Some(v) => format!("{v:.7}"),
    }
}

fn cache_label(c: &CacheSpec) -> String {
    let mode = match c.mode {
        CacheMode::None => "none",
        CacheMode::Auto => "auto",
        CacheMode::Explicit => "explicit",
        CacheMode::Both => "auto+explicit",
    };
    if c.min_cacheable_tokens == 0 {
        format!("{mode} (min n/a)")
    } else {
        format!("{mode} (min {})", c.min_cacheable_tokens)
    }
}

/// `docs/CAPABILITIES.md` 中生成区块的哨兵（人工编辑区间 = 红）。
pub const GENERATED_BEGIN: &str =
    "<!-- BEGIN GENERATED: model-capabilities (llmrust.models.json) -->";
/// 生成区块结束哨兵。
pub const GENERATED_END: &str = "<!-- END GENERATED: model-capabilities -->";

/// 从 `docs/CAPABILITIES.md` 全文里抽出生成区块（不含哨兵行）。
///
/// 找不到哨兵返回 `None`（由门判红，**不静默通过**）。
pub fn extract_generated_block(doc: &str) -> Option<String> {
    let begin = doc.find(GENERATED_BEGIN)?;
    let after_begin = begin + GENERATED_BEGIN.len();
    let rest = &doc[after_begin..];
    let end = rest.find(GENERATED_END)?;
    Some(rest[..end].trim_matches('\n').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, mode: CacheMode, read: Option<f64>) -> ModelCapability {
        ModelCapability {
            id: id.to_string(),
            provider: "p".to_string(),
            protocol: "openai".to_string(),
            context_window: 1000,
            max_output: 100,
            lifecycle: "active".to_string(),
            pricing: PriceRow {
                prompt_per_1k: 0.001,
                completion_per_1k: 0.002,
                cache_read_per_1k: read,
                cache_write_5m_per_1k: None,
                cache_write_1h_per_1k: None,
            },
            cache: CacheSpec {
                mode,
                min_cacheable_tokens: 1024,
                ttl: "fixed_5m".to_string(),
                eviction: "expiry".to_string(),
                breakpoint_limit: None,
            },
            reasoning: "implemented".to_string(),
            tool_calling: true,
            verified_at: "2026-09-11".to_string(),
            source: "test".to_string(),
        }
    }

    fn table(models: Vec<ModelCapability>) -> ModelCapabilityTable {
        ModelCapabilityTable {
            version: "0.1.3".to_string(),
            models,
        }
    }

    #[test]
    fn consistent_table_has_no_problems() {
        let t = table(vec![row("m1", CacheMode::Auto, Some(0.0001))]);
        assert!(t.consistency_problems().is_empty());
    }

    /// 负例：声明缓存读价却声明"无缓存" → 必须红。
    #[test]
    fn cache_price_with_none_mode_is_a_problem() {
        let t = table(vec![row("m1", CacheMode::None, Some(0.0001))]);
        let problems = t.consistency_problems();
        assert_eq!(problems.len(), 1, "got {problems:?}");
        assert!(problems[0].contains("cache mode is `none`"), "{problems:?}");
    }

    /// 负例：负价 → 必须红。
    #[test]
    fn negative_price_is_a_problem() {
        let mut r = row("m1", CacheMode::Auto, Some(-1.0));
        r.pricing.prompt_per_1k = -0.5;
        let problems = table(vec![r]).consistency_problems();
        assert!(problems
            .iter()
            .any(|p| p.contains("must be finite and >= 0")));
    }

    /// 负例：max_output 超过 context_window → 必须红。
    #[test]
    fn max_output_above_window_is_a_problem() {
        let mut r = row("m1", CacheMode::Auto, None);
        r.context_window = 100;
        r.max_output = 101;
        let problems = table(vec![r]).consistency_problems();
        assert!(problems.iter().any(|p| p.contains("must not exceed")));
    }

    /// 渲染确定性：同数据两次渲染完全一致；且行序稳定（按 provider+id 排序）。
    #[test]
    fn render_is_deterministic_and_sorted() {
        let mut a = row("zzz", CacheMode::Auto, None);
        a.provider = "b".to_string();
        let mut b = row("aaa", CacheMode::Auto, None);
        b.provider = "a".to_string();
        let t1 = table(vec![a.clone(), b.clone()]);
        let t2 = table(vec![b, a]);
        assert_eq!(t1.render_markdown(), t2.render_markdown());
        let md = t1.render_markdown();
        let pos_a = md.find("`aaa`").unwrap();
        let pos_z = md.find("`zzz`").unwrap();
        assert!(pos_a < pos_z, "rows must be sorted by provider then id");
    }

    /// 哨兵抽取：正常、缺哨兵（None，不静默通过）。
    #[test]
    fn extract_generated_block_handles_missing_sentinels() {
        let doc = format!("head\n{GENERATED_BEGIN}\nrow1\n{GENERATED_END}\ntail\n");
        assert_eq!(extract_generated_block(&doc).as_deref(), Some("row1"));
        assert!(extract_generated_block("no sentinels here").is_none());
        assert!(extract_generated_block(GENERATED_BEGIN).is_none());
    }
}
