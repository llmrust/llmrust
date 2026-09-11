//! `GRD-003` 第 ④ 项：**门禁准入检查**（negative-test admission）。
//!
//! 规格原文（`GRD-003` 执行步骤 ④）："建立准入检查：**新增 guard 文件若无对应
//! negative test，CI 失败**"。本文件即该准入。
//!
//! # 规则（三条，皆为机器判定）
//!
//! 1. **登记制**：`tests/` 下凡形如 `*_guard.rs` 的文件**必须**登记在
//!    `tests/guard_registry.json` 的 `guards` 里 —— 新写一个 guard 忘了配负例，
//!    本门**直接红**；
//! 2. **不得空口**：登记项声明的 `negative_tests` 名单，必须在该文件里**真的存在同名 `fn`**
//!    —— 声明一个不存在的负例 = 红（防止"写在 JSON 里就算有"）；
//! 3. **债要显式**：尚无负例的 guard 必须列进 `debt`（带 anchor），
//!    **不得静默留白** —— 静默的缺口正是本卡要治的病。
//!
//! 判定逻辑抽成纯函数（[`admission_problems`]），故负例可直接驱动，不必造真文件。
//!
//! 本地跑：`cargo test --test guard_admission`

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

/// `admission_problems` 的输入：一个 guard 文件的事实面。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardFacts {
    /// `tests/` 下的文件名（如 `foo_guard.rs`）。
    pub file: String,
    /// 该文件里真实存在的 `fn` 名集合。
    pub declared_fns: BTreeSet<String>,
}

/// 登记项（来自 `guard_registry.json`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardEntry {
    /// 文件名。
    pub file: String,
    /// 声明的负例测试名。
    pub negative_tests: Vec<String>,
}

/// **纯函数**：给定"磁盘事实"与"登记表"，返回全部违规。
///
/// 参数：
/// - `guard_suffix`：判定"形如 guard"的后缀（如 `_guard.rs`）；
/// - `on_disk`：`tests/` 下所有 `.rs` 文件名（全量，不只 guard）；
/// - `facts`：每个 guard 文件的 `fn` 集合；
/// - `entries`：登记表 `guards`；
/// - `debt`：登记的债（文件名集合）。
pub fn admission_problems(
    guard_suffix: &str,
    on_disk: &BTreeSet<String>,
    facts: &BTreeMap<String, BTreeSet<String>>,
    entries: &[GuardEntry],
    debt: &BTreeSet<String>,
) -> Vec<String> {
    let mut problems = Vec::new();

    // 规则 1：guard 形状的文件必须登记（新 guard 无登记 → 红）。
    let registered: BTreeSet<&str> = entries.iter().map(|e| e.file.as_str()).collect();
    for f in on_disk {
        if f.ends_with(guard_suffix) && !registered.contains(f.as_str()) && !debt.contains(f) {
            problems.push(format!(
                "{f}: guard-shaped test file is NOT registered in guard_registry.json \
                 (a new guard without a negative test must not pass admission)"
            ));
        }
    }

    // 规则 2：登记项声明的负例必须真实存在；且至少要有一条。
    for e in entries {
        if !on_disk.contains(&e.file) {
            problems.push(format!(
                "{}: registered but the file does not exist",
                e.file
            ));
            continue;
        }
        if e.negative_tests.is_empty() {
            problems.push(format!(
                "{}: registered with ZERO negative tests (a guard without a negative test is decoration)",
                e.file
            ));
        }
        let Some(fns) = facts.get(&e.file) else {
            problems.push(format!("{}: could not read declared fns", e.file));
            continue;
        };
        for nt in &e.negative_tests {
            if !fns.contains(nt) {
                problems.push(format!(
                    "{}: declares negative test `{nt}` but no such `fn` exists in the file \
                     (declared ≠ present)",
                    e.file
                ));
            }
        }
    }

    // 规则 3：债必须可解（文件存在），且不得与 guards 重复登记。
    for d in debt {
        if !on_disk.contains(d) {
            problems.push(format!("{d}: listed as debt but the file does not exist"));
        }
        if registered.contains(d.as_str()) {
            problems.push(format!("{d}: listed both as a guard and as debt"));
        }
    }

    problems
}

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn fn_names(src: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in src.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("fn ").or_else(|| {
            t.strip_prefix("pub fn ")
                .or_else(|| t.strip_prefix("pub(crate) fn "))
        }) {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                out.insert(name);
            }
        }
    }
    out
}

/// **准入门**（在真实仓库上跑）。
#[test]
fn guard_admission_holds() {
    let tests_dir = repo_root().join("tests");
    let registry_path = tests_dir.join("guard_registry.json");
    let registry_text =
        fs::read_to_string(&registry_path).expect("tests/guard_registry.json must exist");
    let registry: serde_json::Value =
        serde_json::from_str(&registry_text).expect("guard_registry.json must be valid JSON");
    let suffix = registry["guard_file_suffix"]
        .as_str()
        .expect("registry must declare guard_file_suffix")
        .to_string();

    let mut on_disk = BTreeSet::new();
    let mut facts: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for entry in fs::read_dir(&tests_dir).expect("tests/ must be readable") {
        let path = entry.expect("dir entry").path();
        if path.extension().is_some_and(|e| e == "rs") {
            let name = path
                .file_name()
                .expect("file name")
                .to_string_lossy()
                .to_string();
            let src = fs::read_to_string(&path).unwrap_or_default();
            facts.insert(name.clone(), fn_names(&src));
            on_disk.insert(name);
        }
    }

    let entries: Vec<GuardEntry> = registry["guards"]
        .as_array()
        .expect("registry.guards must be an array")
        .iter()
        .map(|g| GuardEntry {
            file: g["file"].as_str().expect("guard.file").to_string(),
            negative_tests: g["negative_tests"]
                .as_array()
                .expect("guard.negative_tests")
                .iter()
                .map(|v| v.as_str().expect("negative test name").to_string())
                .collect(),
        })
        .collect();

    let debt: BTreeSet<String> = registry["debt"]
        .as_array()
        .expect("registry.debt must be an array")
        .iter()
        .map(|d| d["file"].as_str().expect("debt.file").to_string())
        .collect();

    let problems = admission_problems(&suffix, &on_disk, &facts, &entries, &debt);
    assert!(
        problems.is_empty(),
        "Guard admission problems (GRD-003 ④):\n{}",
        problems.join("\n")
    );
}

// ── 负例：准入检查自身必须会红（SPCC §3） ──────────────────────────────

fn facts(pairs: &[(&str, &[&str])]) -> BTreeMap<String, BTreeSet<String>> {
    pairs
        .iter()
        .map(|(f, fns)| {
            (
                f.to_string(),
                fns.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>(),
            )
        })
        .collect()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// 负例一：**新加一个 guard 文件但没登记** → 必须红。
#[test]
fn negative_unregistered_guard_is_reported() {
    let on_disk = set(&["new_guard.rs"]);
    let f = facts(&[("new_guard.rs", &["some_test"])]);
    let problems = admission_problems("_guard.rs", &on_disk, &f, &[], &set(&[]));
    assert_eq!(problems.len(), 1, "got {problems:?}");
    assert!(problems[0].contains("NOT registered"), "{problems:?}");
}

/// 负例二：**声明的负例在文件里不存在**（空口） → 必须红。
#[test]
fn negative_declared_but_absent_test_is_reported() {
    let on_disk = set(&["a_guard.rs"]);
    let f = facts(&[("a_guard.rs", &["real_check"])]);
    let entries = vec![GuardEntry {
        file: "a_guard.rs".to_string(),
        negative_tests: vec!["ghost_negative_test".to_string()],
    }];
    let problems = admission_problems("_guard.rs", &on_disk, &f, &entries, &set(&[]));
    assert_eq!(problems.len(), 1, "got {problems:?}");
    assert!(problems[0].contains("no such `fn` exists"), "{problems:?}");
}

/// 负例三：**登记了却零负例** → 必须红（装饰门）。
#[test]
fn negative_zero_negative_tests_is_reported() {
    let on_disk = set(&["a_guard.rs"]);
    let f = facts(&[("a_guard.rs", &["only_positive"])]);
    let entries = vec![GuardEntry {
        file: "a_guard.rs".to_string(),
        negative_tests: vec![],
    }];
    let problems = admission_problems("_guard.rs", &on_disk, &f, &entries, &set(&[]));
    assert!(
        problems.iter().any(|p| p.contains("ZERO negative tests")),
        "{problems:?}"
    );
}

/// 负例四：**登记了不存在的文件 / 同一文件既登记为 guard 又登记为债** → 必须红。
#[test]
fn negative_ghost_and_double_registration_are_reported() {
    let on_disk = set(&["a_guard.rs"]);
    let f = facts(&[("a_guard.rs", &["neg"])]);
    let entries = vec![
        GuardEntry {
            file: "ghost_guard.rs".to_string(),
            negative_tests: vec!["neg".to_string()],
        },
        GuardEntry {
            file: "a_guard.rs".to_string(),
            negative_tests: vec!["neg".to_string()],
        },
    ];
    let problems = admission_problems("_guard.rs", &on_disk, &f, &entries, &set(&["a_guard.rs"]));
    assert!(
        problems.iter().any(|p| p.contains("does not exist")),
        "{problems:?}"
    );
    assert!(
        problems
            .iter()
            .any(|p| p.contains("both as a guard and as debt")),
        "{problems:?}"
    );
}

/// 正向对照：登记齐全、负例真实存在、债可解 → **不得**报违规。
#[test]
fn well_formed_registry_passes() {
    let on_disk = set(&["a_guard.rs", "b_guard.rs", "helper.rs"]);
    let f = facts(&[
        ("a_guard.rs", &["neg_a"]),
        ("b_guard.rs", &["neg_b"]),
        ("helper.rs", &["whatever"]),
    ]);
    let entries = vec![
        GuardEntry {
            file: "a_guard.rs".to_string(),
            negative_tests: vec!["neg_a".to_string()],
        },
        GuardEntry {
            file: "b_guard.rs".to_string(),
            negative_tests: vec!["neg_b".to_string()],
        },
    ];
    let problems = admission_problems("_guard.rs", &on_disk, &f, &entries, &set(&[]));
    assert!(problems.is_empty(), "got {problems:?}");
}
