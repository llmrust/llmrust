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
//! # 对账的扩面（B-AUD-1 / AUD#17 / AUD#21）
//!
//! 两条对账轴、两种边界（见卡内 architect 裁定）：
//!
//! - **运行时 ↔ JSON**（[`FACES`]，`runtime_declaration_matches_the_json_inventory`）：
//!   仅限 `Capabilities` 结构体**已有**的运行时可验证面——`embeddings` / `prompt_cache`
//!   自本卡入列；`json_mode` / `json_schema` / `logprobs` / `system_message` /
//!   `multi_turn` **不**入列（`Capabilities` 无这些面，不为文档对账反向加结构体字段）。
//! - **文档 ↔ JSON**（[`doc_row_to_face`]，`documentation_matrix_matches_the_json_inventory`）：
//!   矩阵行映射扩到 json mode / JSON schema / logprobs / system message / multi-turn /
//!   embeddings 六行；`prompt_cache` 刻意**不**开文档矩阵行（它走上一条轴）。无载体行
//!   （`custom base URL` 类）走 [`DOC_ROWS_WITHOUT_CARRIER`] **显式白名单**，未知行判红。
//!
//! 复用 `tests/api_freeze.rs` 的 **fail-closed** 模式（SPCC §9.2 第 1 条）：解析失败/缺字段
//! 即红，不静默跳过。矩阵行收集处**不做任何映射/白名单预过滤**（F-1 返工修复：修复
//! 前未映射行在收集处被预过滤掉，纯函数的判红分支从未被真实文件路径驱动过）：矩阵
//! 表内的全部行都进对账——未映射且未白名单的矩阵行判红、缺 `embeddings` /
//! `prompt_cache` 键的 JSON 判红、矩阵缺任一 JSON 侧已知面的行也判红（F-2 行集完整性）。
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

/// 本门比较的八个能力面（`Capabilities` 结构体上运行时可验证、且 JSON 有布尔位的那些）。
///
/// AUD#21 前这里只有 6 面——`embeddings` / `prompt_cache` 虽在
/// `src/providers/capabilities.rs` 的结构体里（运行时可验证），却从未进过运行时 ↔ JSON
/// 门。两键自本卡入列；`json_mode` / `json_schema` / `logprobs` / `system_message` /
/// `multi_turn` 是**文档 ↔ JSON** 轴的面（见 [`doc_row_to_face`]），刻意不进本数组。
const FACES: &[&str] = &[
    "chat",
    "stream",
    "tool_calling",
    "tool_calling_stream",
    "image_input",
    "reasoning",
    "embeddings",
    "prompt_cache",
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
        "embeddings" => caps.embeddings.level,
        "prompt_cache" => caps.prompt_cache.level,
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

/// 矩阵行标签归一化（去掉 `**bold**` 星号、首尾空白，统一小写）。
fn normalize_doc_row(label: &str) -> String {
    label.trim().trim_matches('*').to_lowercase()
}

/// 文档矩阵的行标签 → JSON 能力面（**docs ↔ JSON 对账的行全集**）。
///
/// AUD#21 扩面：`json mode` / `json schema` / `logprobs` / `system message` /
/// `multi_turn` / `embeddings` 六行全部纳入——这些面 JSON 里有布尔位，修复前它们
/// 一旦漂移没有门会红（AUD#17 正是这么漏的：openrouter json_schema/logprobs 的
/// 文档 ➖ 对 JSON true，无人报）。`prompt_cache` 刻意不在此列（JSON 有位、运行时
/// 可验证，但本卡不开新的文档矩阵行——见 [`FACES`] 的运行时轴）。
pub fn doc_row_to_face(row_label: &str) -> Option<&'static str> {
    let l = normalize_doc_row(row_label);
    if l.starts_with("chat") {
        Some("chat")
    } else if l.starts_with("stream") {
        Some("stream")
    } else if l.starts_with("tool calling (chat)") {
        Some("tool_calling")
    } else if l.starts_with("tool calling (stream)") {
        Some("tool_calling_stream")
    } else if l.starts_with("json mode") {
        Some("json_mode")
    } else if l.starts_with("json schema") {
        Some("json_schema")
    } else if l.starts_with("logprobs") {
        Some("logprobs")
    } else if l.starts_with("image input") {
        Some("image_input")
    } else if l.starts_with("system message") {
        Some("system_message")
    } else if l.starts_with("multi-turn") {
        Some("multi_turn")
    } else if l.starts_with("embeddings") {
        Some("embeddings")
    } else if l.starts_with("reasoning") {
        Some("reasoning")
    } else {
        None
    }
}

/// 文档矩阵中**无载体行**的显式白名单（AUD#21：未映射 ≠ 静默跳过）。
///
/// 这些行描述的是**客户端配置行为**，在 `llmrust.capabilities.json` 与
/// `Provider::capabilities()` 里都没有对应声明位，故无从对账："custom base URL"
/// 对全部 7 家 provider 都成立（`ProviderConfig::base_url` / `set_openai_compatible`
/// / `set_custom`），不是任何一家的能力面。白名单是封闭集合：矩阵里出现任何
/// 既不映射到面、又不在名单里的行，[`doc_vs_json_problems`] 必须判红
/// （`negative_unmapped_matrix_row_is_reported` 钉死这一点）。新增无载体行
/// 必须同步改这里——静默放行的口子一个都不留。
const DOC_ROWS_WITHOUT_CARRIER: &[&str] = &["custom base URL"];

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
///
/// AUD#21 行口径：矩阵行**要么**映射到一个能力面（[`doc_row_to_face`]），**要么**在
/// [`DOC_ROWS_WITHOUT_CARRIER`] 白名单里显式登记，**否则判红**——「未映射行静默
/// 跳过」正是 #21 的漏洞形态（修复前 `custom base URL` 行就是这么逃门的，一个
/// 真正的未知行也会同样逃走）。
pub fn doc_vs_json_problems(
    columns: &[&str],
    rows: &[(&str, Vec<String>)],
    json: &BTreeMap<String, BTreeMap<String, bool>>,
) -> Vec<String> {
    let mut out = Vec::new();
    for (row_label, cells) in rows {
        let Some(face) = doc_row_to_face(row_label) else {
            let normalized = normalize_doc_row(row_label);
            if DOC_ROWS_WITHOUT_CARRIER
                .iter()
                .any(|w| normalize_doc_row(w) == normalized)
            {
                // 显式白名单：无载体行（取舍说明见 DOC_ROWS_WITHOUT_CARRIER）。
                continue;
            }
            out.push(format!(
                "docs/CAPABILITIES.md row `{row_label}` has no mapping to a capability face \
                 and is not whitelisted as a carrier-less row (fail-closed: unmapped rows \
                 would silently escape this gate)"
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

/// **纯函数**：docs ↔ JSON **行集完整性**（F-2 返工）——JSON 侧每个开有文档行的面
/// 都必须在矩阵里有对应行，缺行判红（空 = 完整）。
///
/// QA 对抗层实测：修复前从 `docs/CAPABILITIES.md` 删掉整行（如 system message）后
/// 门 2 全绿——逐格比对只看**出现过的行**，没人断言"该出现的行都在"，文档
/// "Any drift … fails CI" 的声称在删行形态下不成立。
///
/// 期望集直接取 [`json_matrix`] 的读取集（JSON 侧已知面），仅剔除刻意不开文档行的
/// `prompt_cache`（ruling 见 [`FACES`] 文档）——不另抄一份面名清单，清单不会两处漂移。
pub fn doc_row_set_problems(
    json: &BTreeMap<String, BTreeMap<String, bool>>,
    rows: &[(&str, Vec<String>)],
) -> Vec<String> {
    let expected: Vec<&str> = json
        .values()
        .next()
        .map(|faces| faces.keys().map(String::as_str).collect())
        .unwrap_or_default();
    let seen: Vec<&str> = rows
        .iter()
        .filter_map(|(label, _)| doc_row_to_face(label))
        .collect();
    expected
        .into_iter()
        .filter(|face| *face != "prompt_cache" && !seen.contains(face))
        .map(|face| {
            format!(
                "docs/CAPABILITIES.md matrix has no row for `{face}` (row-set completeness: \
                 every JSON-side face except `prompt_cache` must have a matrix row — deleting \
                 a row is drift too)"
            )
        })
        .collect()
}

// ── 门本体 ────────────────────────────────────────────────────────────────

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// 读 JSON，转成 `provider → face → bool`。
///
/// `reasoning` 由 `status` 归一化，其余面要求**布尔位**。读取集 = **文档 ↔ JSON 行全集**
/// （`doc_row_to_face` 能映射的全部面：json mode / JSON schema / logprobs /
/// system message / multi-turn / embeddings / prompt_cache + 运行时轴的六个面）。
/// AUD#21 起这些位**全部必填**——缺位直接 panic（fail-closed，见模块文档）。
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
            "json_mode",
            "json_schema",
            "logprobs",
            "system_message",
            "multi_turn",
            "embeddings",
            "prompt_cache",
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

/// 解析 `docs/CAPABILITIES.md` 的 provider 矩阵：表头列 + **矩阵表内的全部数据行**。
///
/// F-1 返工（QA 对抗层实测）：修复前收集处用 `is_mapped_or_whitelisted` 预过滤，未映射
/// 且未白名单的行根本进不了 [`doc_vs_json_problems`]——纯函数的未知行判红分支只被
/// 单元测试构造的输入驱动过，从未被真实文件路径驱动过（把 `**telepathy**` 行插进真实
/// 矩阵，门 2 照样全绿）。修复后**收集处零过滤**：表内每一行都进对账，未知行由纯
/// 函数判红（`negative_real_doc_unmapped_row_injection_is_reported` 以真实文件钉死）。
///
/// 收集范围严格限定在 `Capability` 表头所在的**这一张表**——首个不再以 `|` 开头的行
/// 即表尾；文档后面的采样参数表 / 错误映射表 / 生成模型表**不是**能力矩阵，不进对账。
/// 门内全行判红、门外全表排除，两边都不留灰色地带。
/// 文档矩阵的一行：行标签 + 逐列单元格（与 [`doc_vs_json_problems`] 的入参同形）。
type DocMatrixRow<'a> = (&'a str, Vec<String>);

/// [`parse_doc_matrix`] 的解析结果：表头列（未见表头为 `None`）+ 全部数据行。
type DocMatrix<'a> = (Option<Vec<&'a str>>, Vec<DocMatrixRow<'a>>);

fn parse_doc_matrix(doc: &str) -> DocMatrix<'_> {
    let mut columns: Option<Vec<&str>> = None;
    let mut rows: Vec<(&str, Vec<String>)> = Vec::new();
    let mut in_matrix = false;
    for line in doc.lines() {
        let t = line.trim();
        if !t.starts_with('|') {
            if in_matrix {
                break; // 矩阵表结束：后面的表格不属于能力矩阵。
            }
            continue;
        }
        let cells: Vec<&str> = t.trim_matches('|').split('|').map(str::trim).collect();
        if cells.first() == Some(&"Capability") {
            columns = Some(cells[1..].to_vec());
            in_matrix = true;
            continue;
        }
        if !in_matrix || t.contains("---") {
            continue;
        }
        // label 借自 doc，随返回值一起返回（调用方的 doc 活得比返回值久）。
        let owned: Vec<String> = cells[1..].iter().map(|s| s.to_string()).collect();
        rows.push((cells[0], owned));
    }
    (columns, rows)
}

/// 门 2 的对账全链路（单元格漂移 + 未映射行 + 行集完整性）。
///
/// 门测试与文件级回归测试（F-1 / F-2）共用这一条链路——同一份代码，不可能各说各话。
fn doc_matrix_problems(doc: &str, json: &BTreeMap<String, BTreeMap<String, bool>>) -> Vec<String> {
    let (columns, rows) = parse_doc_matrix(doc);
    let columns = columns.expect("docs/CAPABILITIES.md must contain the provider matrix header");
    assert!(
        !rows.is_empty(),
        "no capability rows parsed from docs/CAPABILITIES.md (parse failure must not pass silently)"
    );
    let mut problems = doc_vs_json_problems(&columns, &rows, json);
    problems.extend(doc_row_set_problems(json, &rows));
    problems
}

/// **门 2/3**：`docs/CAPABILITIES.md` 矩阵 == JSON。
#[test]
fn documentation_matrix_matches_the_json_inventory() {
    let doc = fs::read_to_string(repo_root().join("docs").join("CAPABILITIES.md"))
        .expect("docs/CAPABILITIES.md must exist");
    let problems = doc_matrix_problems(&doc, &json_matrix());
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
    caps.embeddings = llmrust::providers::capabilities::Capability::implemented();
    caps.prompt_cache = llmrust::providers::capabilities::Capability::implemented();

    // JSON 说 tool_calling 为 false，运行时说 true → 红（其余 7 面--含 AUD#21 入列的
    // embeddings / prompt_cache--先对齐，保证这条负例测到的就是 tool_calling 本身）。
    let mut json = json_map(&[
        ("chat", true),
        ("stream", true),
        ("tool_calling", false),
        ("tool_calling_stream", true),
        ("image_input", true),
        ("reasoning", true),
        ("embeddings", true),
        ("prompt_cache", true),
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
    // 这条负例就测不到"缺面"本身（首版正是栽在这里）。
    // AUD#21 扩面后 FACES 共 8 面：这里 JSON 只给 2 面 → 6 条"no declaration"。
    let mut caps = Capabilities::unknown("p");
    caps.declared = true;
    caps.chat = llmrust::providers::capabilities::Capability::implemented();
    caps.stream = llmrust::providers::capabilities::Capability::implemented();

    let json = json_map(&[("chat", true), ("stream", true)]); // 缺 6 个面
    let problems = runtime_vs_json_problems("p", &caps, &json);
    assert_eq!(problems.len(), 6, "got {problems:?}");
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
    caps.embeddings = llmrust::providers::capabilities::Capability::implemented();
    caps.prompt_cache = llmrust::providers::capabilities::Capability::implemented();
    let json = json_map(&[
        ("chat", true),
        ("stream", true),
        ("tool_calling", true),
        ("tool_calling_stream", true),
        ("image_input", true),
        ("reasoning", true),
        ("embeddings", true),
        ("prompt_cache", true),
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

// ── AUD#17 / AUD#21（B-AUD-1）：扩面后的门本身必须有钉子 ────────────────────

/// 负例（AUD#21 行口径）：**未知矩阵行必须红，白名单行不得被误伤**。
///
/// 修复前的漏洞形态就是"未映射行静默跳过"——`telepathy` 行无论 JSON 怎么写都没人管；
/// 修复后它必须被 [`doc_vs_json_problems`] 点名，而显式登记的无载体行
/// （`custom base URL`）继续放行。
#[test]
fn negative_unmapped_matrix_row_is_reported() {
    // 未知行（没有任何能力面映射，也不在白名单）→ 红。
    let columns = vec!["OpenAI"];
    let rows = vec![
        ("**telepathy**", vec!["✅".to_string()]),
        ("**custom base URL**", vec!["✅".to_string()]),
    ];
    let json = full_json(&[("chat", true)]);
    let problems = doc_vs_json_problems(&columns, &rows, &json);
    assert_eq!(problems.len(), 1, "got {problems:?}");
    assert!(
        problems[0].contains("telepathy") && problems[0].contains("whitelisted"),
        "{problems:?}"
    );
    // 白名单行不得出现在问题里（未被报告）。
    assert!(
        !problems.iter().any(|p| p.contains("custom base URL")),
        "whitelisted carrier-less row must not be reported: {problems:?}"
    );
}

/// 负例（AUD#17 的门必须会红）：**JSON schema 行 openrouter 文档格 ✅ 而 JSON false**
/// → 问题输出必须点名 `openrouter.json_schema`。
///
/// 修复前 json_schema / logprobs 两行不在 doc_row_to_face 里，这种漂移根本没人比
/// （这正是 #17 的盲区：docs ➖ vs JSON true 的真实漂移也存在了数个版本）。
#[test]
fn negative_json_schema_cell_drift_is_reported() {
    let columns = vec!["OpenRouter"];
    let rows = vec![("**JSON schema**", vec!["✅".to_string()])];
    let json = full_json(&[("json_schema", false)]);
    // 把键名换到 openrouter 名下（full_json 固定用 openai 键）。
    let mut json = json;
    let faces = json.remove("openai").expect("full_json inserts openai");
    json.insert("openrouter".to_string(), faces);
    let problems = doc_vs_json_problems(&columns, &rows, &json);
    assert_eq!(problems.len(), 1, "got {problems:?}");
    assert!(
        problems[0].contains("openrouter.json_schema"),
        "{problems:?}"
    );
}

/// 负例（AUD#21 fail-closed 强制键存在）：**运行时七家 provider 对缺
/// `embeddings` / `prompt_cache` 键的 JSON 副本必须报漂移**。
///
/// 这是新键的准入证明：JSON 若没补上这两键，本门会红——所以它们不可能被忘掉。
#[test]
fn negative_missing_embeddings_or_prompt_cache_keys_are_reported() {
    for (name, p) in providers() {
        let caps = p.capabilities();
        // 每家都构造"只缺 embeddings / prompt_cache 两键"的 JSON：其余 6 面全按
        // 运行时真值填（保证报出来的就是缺键，而不是别的漂移）。
        let runtime_bool = |face: &str| level_means_mapped(runtime_level(&caps, face));
        let mut faces = BTreeMap::new();
        for face in [
            "chat",
            "stream",
            "tool_calling",
            "tool_calling_stream",
            "image_input",
            "reasoning",
        ] {
            faces.insert(face.to_string(), runtime_bool(face));
        }
        let problems = runtime_vs_json_problems(name, &caps, &faces);
        assert_eq!(
            problems.len(),
            2,
            "{name}: expected exactly the 2 missing-face problems, got {problems:?}"
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains(&format!("{name}.embeddings"))),
            "{name}: {problems:?}"
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains(&format!("{name}.prompt_cache"))),
            "{name}: {problems:?}"
        );
        assert!(
            problems.iter().all(|p| p.contains("no declaration")),
            "{name}: {problems:?}"
        );
    }
}

/// 正向钉子（AUD#17 方向裁定的正向对照）：读**真实两载体文件**，断言 JSON 里
/// openrouter `json_schema == true`、`logprobs == true`，且 docs 矩阵对应格为 ✅。
///
/// 方向依据（卡内 architect 裁定）：compat.rs 对全部 OpenAiCompatibleProvider 透传
/// `response_format` / `logprobs` / `top_logprobs`，openrouter.rs 是其薄包装——
/// 以 JSON=true 为准，docs ➖ 改 ✅（truth-sync），不反向改 JSON。
#[test]
fn openrouter_json_schema_and_logprobs_are_true_in_json_and_documented() {
    let json = json_matrix();
    let openrouter = json
        .get("openrouter")
        .expect("openrouter must exist in llmrust.capabilities.json");
    assert_eq!(
        openrouter.get("json_schema"),
        Some(&true),
        "ruling: openrouter.json_schema is true (compat.rs forwards response_format)"
    );
    assert_eq!(
        openrouter.get("logprobs"),
        Some(&true),
        "ruling: openrouter.logprobs is true (compat.rs forwards logprobs/top_logprobs)"
    );

    // docs 矩阵：JSON schema / logprobs 行的 OpenRouter 列必须是 ✅。
    let doc = fs::read_to_string(repo_root().join("docs").join("CAPABILITIES.md"))
        .expect("docs/CAPABILITIES.md must exist");
    let mut columns: Option<Vec<&str>> = None;
    let mut checked_rows = 0usize;
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
        let face = doc_row_to_face(cells[0]);
        if face != Some("json_schema") && face != Some("logprobs") {
            continue;
        }
        let face = face.expect("checked above");
        let columns = columns.as_ref().expect("header seen before rows");
        for (col, cell) in columns.iter().zip(cells[1..].iter()) {
            if doc_column_to_provider(col) == Some("openrouter") {
                let doc_value = parse_doc_cell(cell)
                    .unwrap_or_else(|| panic!("unreadable openrouter cell `{}`", cell.trim()));
                let json_value = openrouter
                    .get(face)
                    .copied()
                    .unwrap_or_else(|| panic!("openrouter.{face} must exist in the JSON matrix"));
                assert_eq!(
                    doc_value, json_value,
                    "openrouter.{face}: docs/CAPABILITIES.md must say ✅ (truth-sync per ruling)"
                );
                checked_rows += 1;
            }
        }
    }
    assert_eq!(
        checked_rows, 2,
        "expected to check both json_schema and logprobs rows for openrouter"
    );
}

// ── F-1 / F-2（QA 对抗层返工）：文件级回归，真实 docs/CAPABILITIES.md 驱动 ───

/// F-1 文件级回归：向**真实**矩阵注入未映射且未白名单的行（`**telepathy**`）后，
/// 门 2 的**全链路**（收集 → 逐格对账 → 行集完整性）必须判红。
///
/// 修复前的漏洞形态：`negative_unmapped_matrix_row_is_reported` 只驱动过纯函数；真实
/// 文件路径上，行收集处的预过滤把未知行直接丢掉，门全绿（QA 实测：插入 `**telepathy**
/// | ✅×7` 后 `cargo test --test capability_freeze` exit 0）。本测试读真实文件、在内存中
/// 注入同形行，走门 2 的同一条链路 [`doc_matrix_problems`]——收集处若再现任何静默
/// 丢弃，这里立刻红。
#[test]
fn negative_real_doc_unmapped_row_injection_is_reported() {
    let doc = fs::read_to_string(repo_root().join("docs").join("CAPABILITIES.md"))
        .expect("docs/CAPABILITIES.md must exist");
    let json = json_matrix();

    // 前提：未注入的真实文件必须零问题（门 2 本身绿，负例才有意义）。
    assert!(
        doc_matrix_problems(&doc, &json).is_empty(),
        "fixture sanity: the real docs/CAPABILITIES.md must be drift-free"
    );

    // 注入 QA 复测口径的同形行（`**telepathy**` | ✅ ×7）到矩阵表内。
    let mut injected = String::with_capacity(doc.len() + 64);
    let mut hit = false;
    for line in doc.lines() {
        injected.push_str(line);
        injected.push('\n');
        if !hit && line.trim().starts_with("| **system message**") {
            injected.push_str("| **telepathy** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |\n");
            hit = true;
        }
    }
    assert!(
        hit,
        "fixture sanity: the system message row must exist in the real matrix"
    );

    let problems = doc_matrix_problems(&injected, &json);
    assert_eq!(problems.len(), 1, "got {problems:?}");
    assert!(
        problems[0].contains("telepathy") && problems[0].contains("whitelisted"),
        "{problems:?}"
    );
}

/// F-2 文件级回归：从**真实**矩阵删除一个已映射行（`**system message**`）后，门 2
/// 的全链路必须判红（由 [`doc_row_set_problems`] 行集完整性补上的轴）。
///
/// 修复前的漏洞形态：逐格比对只覆盖**出现过的行**，删行后没有任何断言发现"该在的
/// 行不在了"（QA 实测：删除 system message 行后 `cargo test --test capability_freeze`
/// exit 0）——docs/CAPABILITIES.md 的 "Any drift … fails CI" 声称在删行形态下不成立。
#[test]
fn negative_real_doc_deleted_matrix_row_is_reported() {
    let doc = fs::read_to_string(repo_root().join("docs").join("CAPABILITIES.md"))
        .expect("docs/CAPABILITIES.md must exist");
    let json = json_matrix();

    // 前提：未删行的真实文件必须零问题。
    assert!(
        doc_matrix_problems(&doc, &json).is_empty(),
        "fixture sanity: the real docs/CAPABILITIES.md must be drift-free"
    );

    // 删除 QA 复测口径指名的整行（`**system message**`，7 格）。
    let mut deleted = String::with_capacity(doc.len());
    let mut removed = false;
    for line in doc.lines() {
        if line.trim().starts_with("| **system message**") {
            removed = true;
            continue;
        }
        deleted.push_str(line);
        deleted.push('\n');
    }
    assert!(
        removed,
        "fixture sanity: the system message row must exist in the real matrix"
    );

    let problems = doc_matrix_problems(&deleted, &json);
    assert_eq!(problems.len(), 1, "got {problems:?}");
    assert!(
        problems[0].contains("system_message") && problems[0].contains("no row"),
        "{problems:?}"
    );
}
