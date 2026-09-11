//! `CAP-007` 门：每模型能力/价格表 ↔ `docs/CAPABILITIES.md` 一致性。
//!
//! 三道：
//! 1. **表合法且自洽**——`llmrust.models.json` 可解析、七类字段齐全、无自相矛盾；
//! 2. **文档由表生成**——`docs/CAPABILITIES.md` 中哨兵区间内的文本**必须**等于
//!    [`llmrust::capability_table::ModelCapabilityTable::render_markdown`] 的产出
//!    （人工改动 = 红）；
//! 3. **负例证明会红**——三条纯函数级负例（缓存价配"无缓存模式"、负价、输出超窗口），
//!    外加"哨兵缺失不静默通过"。
//!
//! 本地跑：`cargo test --test capability_table_guard`

use std::fs;
use std::path::Path;

use llmrust::capability_table::{
    extract_generated_block, CacheMode, CacheSpec, ModelCapabilityTable, PriceRow, GENERATED_BEGIN,
    GENERATED_END,
};

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn load_table() -> ModelCapabilityTable {
    let path = repo_root().join("llmrust.models.json");
    let text = fs::read_to_string(&path).expect("llmrust.models.json must exist");
    ModelCapabilityTable::from_json(&text).expect("llmrust.models.json must parse")
}

/// 七类字段逐条在位（`CAP-007` DoD 1）。
#[test]
fn model_table_has_all_required_fields() {
    let table = load_table();
    assert!(
        !table.models.is_empty(),
        "table must have at least one model"
    );
    for m in &table.models {
        assert!(!m.id.is_empty(), "id must not be empty");
        assert!(
            !m.provider.is_empty(),
            "{}: provider must not be empty",
            m.id
        );
        assert!(
            !m.protocol.is_empty(),
            "{}: protocol must not be empty",
            m.id
        );
        assert!(m.context_window > 0, "{}: context_window", m.id);
        assert!(m.max_output > 0, "{}: max_output", m.id);
        assert!(!m.lifecycle.is_empty(), "{}: lifecycle", m.id);
        assert!(
            !m.reasoning.is_empty(),
            "{}: reasoning (pricing is a struct, always present)",
            m.id
        );
        assert!(!m.verified_at.is_empty(), "{}: verified_at", m.id);
        assert!(!m.source.is_empty(), "{}: source", m.id);
        // cache spec + pricing are structs, so presence is structural — assert the
        // cache mode is one of the declared variants by round-tripping the label.
        match m.cache.mode {
            CacheMode::None | CacheMode::Auto | CacheMode::Explicit | CacheMode::Both => {}
        }
    }
}

/// 表内自洽（`CAP-007` DoD 4 的正向侧）。
#[test]
fn model_table_is_self_consistent() {
    let problems = load_table().consistency_problems();
    assert!(
        problems.is_empty(),
        "model table problems:\n{}",
        problems.join("\n")
    );
}

/// **本门的核心**：`docs/CAPABILITIES.md` 的生成区块必须等于表的渲染结果。
///
/// 人工改文档里那段表格（哪怕一个字符）→ 本测试红。
#[test]
fn capabilities_md_generated_block_matches_table() {
    let table = load_table();
    let expected = table.render_markdown();

    let doc_path = repo_root().join("docs").join("CAPABILITIES.md");
    let doc = fs::read_to_string(&doc_path).expect("docs/CAPABILITIES.md must exist");

    let found = extract_generated_block(&doc).unwrap_or_else(|| {
        panic!(
            "docs/CAPABILITIES.md is missing the generated block sentinels.\n\
             Expected both of:\n  {GENERATED_BEGIN}\n  {GENERATED_END}"
        )
    });

    assert_eq!(
        found.trim(),
        expected.trim(),
        "docs/CAPABILITIES.md model-capabilities block drifted from llmrust.models.json.\n\
         The block is GENERATED — edit llmrust.models.json, not the doc."
    );
}

/// 生成区块必须真的在文档里出现过一次（防"哨兵在但内容为空"）。
#[test]
fn generated_block_is_present_and_non_empty() {
    let doc = fs::read_to_string(repo_root().join("docs").join("CAPABILITIES.md")).unwrap();
    assert_eq!(
        doc.matches(GENERATED_BEGIN).count(),
        1,
        "exactly one BEGIN sentinel"
    );
    assert_eq!(
        doc.matches(GENERATED_END).count(),
        1,
        "exactly one END sentinel"
    );
    let block = extract_generated_block(&doc).expect("block must be extractable");
    assert!(
        block.contains("| Model |"),
        "block must contain the generated header row"
    );
}

// ── 负例：门必须会红（SPCC §3："没有 negative test 的 guard 不算门禁"） ──

fn sample(mode: CacheMode, read: Option<f64>) -> llmrust::capability_table::ModelCapability {
    llmrust::capability_table::ModelCapability {
        id: "sample".to_string(),
        provider: "p".to_string(),
        protocol: "openai".to_string(),
        context_window: 1_000,
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
            min_cacheable_tokens: 256,
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

fn table_of(models: Vec<llmrust::capability_table::ModelCapability>) -> ModelCapabilityTable {
    ModelCapabilityTable {
        version: "0.1.3".to_string(),
        models,
    }
}

#[test]
fn negative_cache_price_without_cache_mode_is_reported() {
    let t = table_of(vec![sample(CacheMode::None, Some(0.0001))]);
    let problems = t.consistency_problems();
    assert_eq!(problems.len(), 1, "got {problems:?}");
    assert!(problems[0].contains("cache mode is `none`"), "{problems:?}");
}

#[test]
fn negative_price_is_reported() {
    let mut m = sample(CacheMode::Auto, Some(-1.0));
    m.pricing.prompt_per_1k = -0.001;
    let problems = table_of(vec![m]).consistency_problems();
    assert!(
        problems
            .iter()
            .any(|p| p.contains("must be finite and >= 0")),
        "got {problems:?}"
    );
}

#[test]
fn negative_max_output_above_window_is_reported() {
    let mut m = sample(CacheMode::Auto, None);
    m.context_window = 10;
    m.max_output = 11;
    let problems = table_of(vec![m]).consistency_problems();
    assert!(
        problems.iter().any(|p| p.contains("must not exceed")),
        "got {problems:?}"
    );
}

/// 文档漂移必须红：把真实区块里的一个字符改掉，比较必须不等。
///
/// 注意：不能用"加尾随空格"来构造漂移——`trim()` 会把它吃掉，那样的负例是假绿。
#[test]
fn negative_doc_drift_is_detected() {
    let table = load_table();
    let expected = table.render_markdown();
    let doc = fs::read_to_string(repo_root().join("docs").join("CAPABILITIES.md")).unwrap();
    let actual = extract_generated_block(&doc).expect("block must exist");
    assert_eq!(actual.trim(), expected.trim());

    // 改一个字符（把表头里的 `Context` 改成 `ContextX`），必须能被检出。
    let drifted = actual.replacen("| Context |", "| ContextX |", 1);
    assert_ne!(
        drifted, actual,
        "the fixture must actually change the block"
    );
    assert_ne!(
        drifted.trim(),
        expected.trim(),
        "a one-character manual edit to the generated block must be detectable"
    );
}

/// 哨兵缺失必须**不静默通过**（返回 None → 上层判红）。
#[test]
fn negative_missing_sentinels_do_not_pass_silently() {
    assert!(extract_generated_block("# Capabilities\n\nno sentinels").is_none());
    assert!(extract_generated_block(GENERATED_BEGIN).is_none());
    assert!(extract_generated_block(GENERATED_END).is_none());
}
