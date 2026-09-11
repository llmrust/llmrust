//! `CAP-004`：**能力矩阵三处一致门**。
//!
//! 三处载体，任一漂移即 CI 红：
//!
//! 1. **运行时值** —— `Provider::capabilities()`（`CAP-002` 建立）；
//! 2. **机读声明** —— `llmrust.capabilities.json`；
//! 3. **人读文档** —— `docs/CAPABILITIES.md` 的 provider 支持矩阵。
//!
//! # 口径（关键，写死在此以免各处理解不一）
//!
//! `llmrust.capabilities.json` 自述："*support* means **llmrust maps the field or protocol**"。
//! 因此映射为：
//!
//! | 运行时层级 | ⇒ "llmrust 已映射" |
//! |---|---|
//! | `Implemented` / `Verified` / `ModelDependent` / `PassthroughOnly` | **true** |
//! | `Unsupported` | **false** |
//!
//! `ModelDependent` 归 **true** 是刻意的：它表示"llmrust 会转译，能否生效取决于上游"——
//! 这与 JSON 的 true 语义一致；**上游差异由文档的免责声明承载，不由本矩阵承载**。
//! （若把它当 false，openrouter 的 `tool_calling = ModelDependent` 会与 JSON 的 `true` 冲突，
//! 而那冲突是**口径错**，不是真漂移。）
//!
//! 复用 `tests/api_freeze.rs` 的 **fail-closed** 模式：解析失败/缺字段即红，不静默跳过。
//!
//! 本地跑：`cargo test --test capability_freeze`

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use llmrust::providers::anthropic::AnthropicProvider;
use llmrust::providers::capabilities::{Capabilities, CapabilityLevel};
use llmrust::providers::deepseek::DeepSeekProvider;
use llmrust::providers::google::GoogleProvider;
use llmrust::providers::moonshot::MoonshotProvider;
use llmrust::providers::ollama::OllamaProvider;
use llmrust::providers::openai::OpenAIProvider;
use llmrust::providers::openrouter::OpenRouterProvider;
use llmrust::providers::{Provider, ProviderConfig};

fn cfg() -> ProviderConfig {
    ProviderConfig {
        api_key: "k".to_string(),
        base_url: None,
        timeout_secs: None,
        custom_headers: None,
    }
}

/// 七家 Provider（键名与 JSON / 文档列名同源）。
fn providers() -> Vec<(&'static str, Box<dyn Provider>)> {
    vec![
        (
            "openai",
            Box::new(OpenAIProvider::new(cfg())) as Box<dyn Provider>,
        ),
        ("deepseek", Box::new(DeepSeekProvider::new(cfg()))),
        ("moonshot", Box::new(MoonshotProvider::new(cfg()))),
        ("openrouter", Box::new(OpenRouterProvider::new(cfg()))),
        ("anthropic", Box::new(AnthropicProvider::new(cfg()))),
        ("google", Box::new(GoogleProvider::new(cfg()))),
        ("ollama", Box::new(OllamaProvider::new(cfg()))),
    ]
}

/// 本门比较的六个能力面（JSON 里有对应布尔位的那些）。
const FACES: &[&str] = &[
    "chat",
    "stream",
    "tool_calling",
    "tool_calling_stream",
    "image_input",
    "reasoning",
];

/// **纯函数**：层级 ⇒ "llmrust 已映射"（口径见模块文档）。
pub fn level_means_mapped(level: CapabilityLevel) -> bool {
    !matches!(level, CapabilityLevel::Unsupported)
}

/// 取某 provider 某能力面的运行时层级。
pub fn runtime_level(caps: &Capabilities, face: &str) -> CapabilityLevel {
    match face {
        "chat" => caps.chat.level,
        "stream" => caps.stream.level,
        "tool_calling" => caps.tool_calling.level,
        "tool_calling_stream" => caps.tool_calling_stream.level,
        "image_input" => caps.image_input.level,
        "reasoning" => caps.reasoning.level,
        other => panic!("unknown capability face `{other}`"),
    }
}

/// **纯函数**：运行时声明 vs JSON 声明的漂移（空 = 一致）。
///
/// `json_status`：`Some(bool)` 表示 JSON 该面有布尔位；`None` 表示 JSON 用其他结构承载
/// （如 `reasoning` 是 `{status: ...}`）——此时**必须**由调用方转成布尔后传入，不得静默跳过。
pub fn runtime_vs_json_problems(
    provider: &str,
    caps: &Capabilities,
    json_bools: &BTreeMap<String, bool>,
) -> Vec<String> {
    let mut out = Vec::new();
    for face in FACES {
        let Some(&json_value) = json_bools.get(*face) else {
            out.push(format!(
                "{provider}.{face}: llmrust.capabilities.json has no declaration for this face \
                 (fail-closed: a face the runtime declares must be declared in the JSON too)"
            ));
            continue;
        };
        let runtime = level_means_mapped(runtime_level(caps, face));
        if runtime != json_value {
            out.push(format!(
                "{provider}.{face}: runtime capabilities() says {runtime:?} (level {:?}) but \
                 llmrust.capabilities.json says {json_value:?}",
                runtime_level(caps, face)
            ));
        }
    }
    out
}

/// 文档矩阵单元格 → 布尔（`✅` ⇒ true，`➖` ⇒ false；后缀如 `(data: URL only)` 容忍）。
///
/// 无法判读的单元格返回 `None` → 由门判红（**不静默通过**）。
pub fn parse_doc_cell(cell: &str) -> Option<bool> {
    let t = cell.trim();
    if t.starts_with('✅') {
        Some(true)
    } else if t.starts_with('➖') {
        Some(false)
    } else {
        None
    }
}

/// 文档矩阵的行标签 → JSON 能力面。
pub fn doc_row_to_face(row_label: &str) -> Option<&'static str> {
    let l = row_label.trim().trim_matches('*').to_lowercase();
    if l.starts_with("chat") {
        Some("chat")
    } else if l.starts_with("stream") {
        Some("stream")
    } else if l.starts_with("tool calling (chat)") {
        Some("tool_calling")
    } else if l.starts_with("tool calling (stream)") {
        Some("tool_calling_stream")
    } else if l.starts_with("image input") {
        Some("image_input")
    } else if l.starts_with("reasoning") {
        Some("reasoning")
    } else {
        None
    }
}

/// 文档列名 → JSON provider 键。
pub fn doc_column_to_provider(col: &str) -> Option<&'static str> {
    match col.trim() {
        "OpenAI" => Some("openai"),
        "DeepSeek" => Some("deepseek"),
        "Moonshot" => Some("moonshot"),
        "OpenRouter" => Some("openrouter"),
        "Anthropic" => Some("anthropic"),
        "Gemini" => Some("google"),
        "Ollama" => Some("ollama"),
        _ => None,
    }
}

/// **纯函数**：文档单元格 vs JSON 的漂移（空 = 一致）。
pub fn doc_vs_json_problems(
    columns: &[&str],
    rows: &[(&str, Vec<String>)],
    json: &BTreeMap<String, BTreeMap<String, bool>>,
) -> Vec<String> {
    let mut out = Vec::new();
    for (row_label, cells) in rows {
        let Some(face) = doc_row_to_face(row_label) else {
            out.push(format!(
                "docs/CAPABILITIES.md row `{row_label}` has no mapping to a capability face \
                 (fail-closed: unmapped rows would silently escape this gate)"
            ));
            continue;
        };
        if cells.len() != columns.len() {
            out.push(format!(
                "row `{row_label}`: {} cells for {} columns",
                cells.len(),
                columns.len()
            ));
            continue;
        }
        for (col, cell) in columns.iter().zip(cells.iter()) {
            let Some(provider) = doc_column_to_provider(col) else {
                out.push(format!("unknown documentation column `{col}`"));
                continue;
            };
            let Some(doc_value) = parse_doc_cell(cell) else {
                out.push(format!(
                    "{provider}.{face}: unreadable documentation cell `{}`",
                    cell.trim()
                ));
                continue;
            };
            let json_value = json
                .get(provider)
                .and_then(|m| m.get(face))
                .copied()
                .unwrap_or(false);
            if doc_value != json_value {
                out.push(format!(
                    "{provider}.{face}: docs/CAPABILITIES.md says {doc_value:?} but \
                     llmrust.capabilities.json says {json_value:?}"
                ));
            }
        }
    }
    out
}

// ── 门本体 ────────────────────────────────────────────────────────────────

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// 读 JSON，转成 `provider → face → bool`（`reasoning` 由 `status` 归一化）。
fn json_matrix() -> BTreeMap<String, BTreeMap<String, bool>> {
    let text = fs::read_to_string(repo_root().join("llmrust.capabilities.json"))
        .expect("llmrust.capabilities.json must exist");
    let json: serde_json::Value =
        serde_json::from_str(&text).expect("llmrust.capabilities.json must be valid JSON");
    let providers = json["providers"]
        .as_object()
        .expect("capabilities.json must contain a providers object");

    let mut out = BTreeMap::new();
    for (name, p) in providers {
        let mut faces = BTreeMap::new();
        for face in [
            "chat",
            "stream",
            "tool_calling",
            "tool_calling_stream",
            "image_input",
        ] {
            let v = p[face]
                .as_bool()
                .unwrap_or_else(|| panic!("{name}.{face} must be a boolean in capabilities.json"));
            faces.insert(face.to_string(), v);
        }
        // reasoning 以对象承载：`status = implemented|unsupported`
        let status = p["reasoning"]["status"]
            .as_str()
            .unwrap_or_else(|| panic!("{name}.reasoning.status must be a string"));
        faces.insert("reasoning".to_string(), status == "implemented");
        out.insert(name.clone(), faces);
    }
    out
}

/// **门 1/3**：运行时 `capabilities()` == `llmrust.capabilities.json`。
#[test]
fn runtime_declaration_matches_the_json_inventory() {
    let json = json_matrix();
    let mut problems = Vec::new();
    for (name, p) in providers() {
        let caps = p.capabilities();
        let faces = json
            .get(name)
            .unwrap_or_else(|| panic!("{name} missing from capabilities.json"));
        problems.extend(runtime_vs_json_problems(name, &caps, faces));
    }
    assert!(
        problems.is_empty(),
        "runtime/JSON capability drift (CAP-004):\n{}",
        problems.join("\n")
    );
}

/// **门 2/3**：`docs/CAPABILITIES.md` 矩阵 == JSON。
#[test]
fn documentation_matrix_matches_the_json_inventory() {
    let doc = fs::read_to_string(repo_root().join("docs").join("CAPABILITIES.md"))
        .expect("docs/CAPABILITIES.md must exist");
    let json = json_matrix();

    // 解析 provider 矩阵：表头行 + 逐行能力行。
    let mut columns: Option<Vec<&str>> = None;
    let mut rows: Vec<(&str, Vec<String>)> = Vec::new();
    for line in doc.lines() {
        let t = line.trim();
        if !t.starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = t.trim_matches('|').split('|').map(str::trim).collect();
        if cells.first() == Some(&"Capability") {
            columns = Some(cells[1..].to_vec());
            continue;
        }
        if columns.is_none() || t.contains("---") {
            continue;
        }
        let label = cells[0];
        if doc_row_to_face(label).is_some() {
            let owned: Vec<String> = cells[1..].iter().map(|s| s.to_string()).collect();
            // SAFETY: label 来自 &doc，生命周期与 doc 相同。
            let label_ref: &str = label;
            rows.push((label_ref, owned));
        }
    }

    let columns = columns.expect("docs/CAPABILITIES.md must contain the provider matrix header");
    assert!(
        !rows.is_empty(),
        "no capability rows parsed from docs/CAPABILITIES.md (parse failure must not pass silently)"
    );

    let problems = doc_vs_json_problems(&columns, &rows, &json);
    assert!(
        problems.is_empty(),
        "documentation/JSON capability drift (CAP-004):\n{}",
        problems.join("\n")
    );
}

/// **门 3/3**：三处的 provider 键集必须一致（漏一家就是漂移）。
#[test]
fn all_three_carriers_cover_the_same_providers() {
    let json = json_matrix();
    let runtime: Vec<&str> = providers().into_iter().map(|(n, _)| n).collect();

    let doc = fs::read_to_string(repo_root().join("docs").join("CAPABILITIES.md")).unwrap();
    let header = doc
        .lines()
        .find(|l| l.trim().starts_with("| Capability |"))
        .expect("matrix header");
    let doc_providers: Vec<&str> = header
        .trim()
        .trim_matches('|')
        .split('|')
        .map(str::trim)
        .skip(1)
        .filter_map(doc_column_to_provider)
        .collect();

    for name in &runtime {
        assert!(
            json.contains_key(*name),
            "{name} missing from capabilities.json"
        );
        assert!(
            doc_providers.contains(name),
            "{name} missing from docs/CAPABILITIES.md matrix"
        );
    }
    assert_eq!(
        runtime.len(),
        doc_providers.len(),
        "runtime has {} providers but the doc matrix has {} columns",
        runtime.len(),
        doc_providers.len()
    );
}

/// 验证层级在文档中可见（`CAP-004` DoD 明文）。
#[test]
fn verification_levels_are_visible_in_the_documentation() {
    let doc = fs::read_to_string(repo_root().join("docs").join("CAPABILITIES.md")).unwrap();
    let lowered = doc.to_lowercase();
    for term in ["verified", "local-fixture", "live-endpoint"] {
        assert!(
            lowered.contains(term),
            "docs/CAPABILITIES.md must document the verification levels (missing `{term}`)"
        );
    }
}

// ── 负例：门必须会红（SPCC §3） ───────────────────────────────────────────

fn json_map(pairs: &[(&str, bool)]) -> BTreeMap<String, bool> {
    pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

fn full_json(pairs: &[(&str, bool)]) -> BTreeMap<String, BTreeMap<String, bool>> {
    let mut faces = BTreeMap::new();
    for (k, v) in pairs {
        faces.insert(k.to_string(), *v);
    }
    let mut out = BTreeMap::new();
    out.insert("openai".to_string(), faces);
    out
}

/// 负例一：**运行时改了、JSON 没改** → 必须红。
#[test]
fn negative_runtime_json_drift_is_reported() {
    let mut caps = Capabilities::unknown("openai-compatible");
    caps.declared = true;
    caps.chat = llmrust::providers::capabilities::Capability::implemented();
    caps.stream = llmrust::providers::capabilities::Capability::implemented();
    caps.tool_calling = llmrust::providers::capabilities::Capability::implemented();
    caps.tool_calling_stream = llmrust::providers::capabilities::Capability::implemented();
    caps.image_input = llmrust::providers::capabilities::Capability::implemented();
    caps.reasoning = llmrust::providers::capabilities::Capability::implemented();

    // JSON 说 tool_calling 为 false，运行时说 true → 红。
    let mut json = json_map(&[
        ("chat", true),
        ("stream", true),
        ("tool_calling", false),
        ("tool_calling_stream", true),
        ("image_input", true),
        ("reasoning", true),
    ]);
    let problems = runtime_vs_json_problems("openai", &caps, &json);
    assert_eq!(problems.len(), 1, "got {problems:?}");
    assert!(problems[0].contains("tool_calling"), "{problems:?}");

    // 反向：JSON 说 true、运行时说 unsupported → 同样红。
    json.insert("tool_calling".to_string(), true);
    caps.tool_calling = llmrust::providers::capabilities::Capability::unsupported();
    let problems = runtime_vs_json_problems("openai", &caps, &json);
    assert_eq!(problems.len(), 1, "got {problems:?}");
}

/// 负例二：**JSON 缺一个面** → 必须红（fail-closed，不静默跳过）。
#[test]
fn negative_missing_face_in_json_is_reported() {
    // 夹具必须在 chat/stream 上先与 JSON 一致，否则报出来的是**别的**漂移，
    // 这条负例就测不到"缺面"本身（首版正是栽在这里：报 6 条而非 4 条）。
    let mut caps = Capabilities::unknown("p");
    caps.declared = true;
    caps.chat = llmrust::providers::capabilities::Capability::implemented();
    caps.stream = llmrust::providers::capabilities::Capability::implemented();

    let json = json_map(&[("chat", true), ("stream", true)]); // 缺 4 个面
    let problems = runtime_vs_json_problems("p", &caps, &json);
    assert_eq!(problems.len(), 4, "got {problems:?}");
    assert!(
        problems.iter().all(|p| p.contains("no declaration")),
        "{problems:?}"
    );
}

/// 负例三：**文档 ✅ 而 JSON false**（文档吹牛）→ 必须红。
#[test]
fn negative_doc_overclaim_is_reported() {
    let columns = vec!["OpenAI"];
    let rows = vec![("**chat**", vec!["✅".to_string()])];
    let json = full_json(&[("chat", false)]);
    let problems = doc_vs_json_problems(&columns, &rows, &json);
    assert_eq!(problems.len(), 1, "got {problems:?}");
    assert!(
        problems[0].contains("docs/CAPABILITIES.md says true"),
        "{problems:?}"
    );
}

/// 负例四：**文档 ➖ 而 JSON true**（文档隐瞒）→ 必须红。
#[test]
fn negative_doc_underclaim_is_reported() {
    let columns = vec!["Ollama"];
    let rows = vec![("**tool calling (chat)**", vec!["➖".to_string()])];
    let mut faces = BTreeMap::new();
    faces.insert("tool_calling".to_string(), true);
    let mut json = BTreeMap::new();
    json.insert("ollama".to_string(), faces);
    let problems = doc_vs_json_problems(&columns, &rows, &json);
    assert_eq!(problems.len(), 1, "got {problems:?}");
    assert!(problems[0].contains("says false"), "{problems:?}");
}

/// 负例五：**单元格不可判读 / 行无映射 / 列未知** → 必须红（不静默通过）。
#[test]
fn negative_unreadable_cells_are_reported() {
    let columns = vec!["OpenAI", "Nope"];
    let rows = vec![("**chat**", vec!["?".to_string(), "✅".to_string()])];
    let json = full_json(&[("chat", true)]);
    let problems = doc_vs_json_problems(&columns, &rows, &json);
    assert!(
        problems.iter().any(|p| p.contains("unreadable")),
        "{problems:?}"
    );
    assert!(
        problems
            .iter()
            .any(|p| p.contains("unknown documentation column")),
        "{problems:?}"
    );
}

/// 正向对照：三处一致时**不得**报任何问题。
#[test]
fn consistent_carriers_produce_no_problems() {
    let mut caps = Capabilities::unknown("openai-compatible");
    caps.declared = true;
    caps.chat = llmrust::providers::capabilities::Capability::implemented();
    caps.stream = llmrust::providers::capabilities::Capability::implemented();
    caps.tool_calling = llmrust::providers::capabilities::Capability::implemented();
    caps.tool_calling_stream = llmrust::providers::capabilities::Capability::implemented();
    caps.image_input = llmrust::providers::capabilities::Capability::implemented();
    caps.reasoning = llmrust::providers::capabilities::Capability::implemented();
    let json = json_map(&[
        ("chat", true),
        ("stream", true),
        ("tool_calling", true),
        ("tool_calling_stream", true),
        ("image_input", true),
        ("reasoning", true),
    ]);
    assert!(runtime_vs_json_problems("openai", &caps, &json).is_empty());

    let columns = vec!["OpenAI", "Ollama"];
    let rows = vec![
        ("**chat**", vec!["✅".to_string(), "✅".to_string()]),
        (
            "**image input**",
            vec!["✅ (data: URL only)".to_string(), "➖".to_string()],
        ),
    ];
    let mut ollama = BTreeMap::new();
    ollama.insert("chat".to_string(), true);
    ollama.insert("image_input".to_string(), false);
    let mut json = full_json(&[("chat", true), ("image_input", true)]);
    json.insert("ollama".to_string(), ollama);
    assert!(doc_vs_json_problems(&columns, &rows, &json).is_empty());
}
