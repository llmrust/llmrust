//! Architecture guards for CI-003 (SPCC §4.3 / §4.4).
//!
//! Machine-enforced boundaries. Three guards:
//!   1. `dependency_edges_are_respected` — forbidden intra-crate `use` edges.
//!   2. `hotspot_files_do_not_grow` — files on the §4.4 hotspot ledger may not
//!      net-grow beyond their recorded baseline (and may not shrink silently
//!      either — bidirectional since #203/GRD-001).
//!   3. `hotspot_coverage_is_complete` — **every** `src/**/*.rs` at or above the
//!      ledger `threshold` MUST be listed on the ledger (#209). Without this the
//!      ledger is only a "named list": an unlisted oversized file escapes both
//!      the grow and the shrink check (e.g. `ollama.rs` at 805 lines).
//!
//! Run locally with: `cargo test --test architecture_guard`

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

/// Forbidden dependency edges, derived verbatim from SPCC §4.3.
/// Each entry is `(file, forbidden_use_prefix)`: any `use`/`pub use` line in
/// `file` that contains `forbidden_use_prefix` is a violation.
fn forbidden_edges() -> Vec<(&'static str, &'static str)> {
    vec![
        // §4.3: types.rs must not depend on provider/client/router/proxy.
        ("src/types.rs", "crate::providers"),
        ("src/types.rs", "crate::proxy"),
        ("src/types.rs", "crate::router"),
        ("src/types.rs", "crate::LmrsClient"),
        ("src/types.rs", "crate::client"),
        // §4.3: a Provider must not depend on proxy or router.
        ("src/providers/mod.rs", "crate::proxy"),
        ("src/providers/mod.rs", "crate::router"),
        ("src/providers/compat.rs", "crate::proxy"),
        ("src/providers/compat.rs", "crate::router"),
        ("src/providers/google.rs", "crate::proxy"),
        ("src/providers/google.rs", "crate::router"),
        ("src/providers/anthropic.rs", "crate::proxy"),
        ("src/providers/anthropic.rs", "crate::router"),
        ("src/providers/openai.rs", "crate::proxy"),
        ("src/providers/openai.rs", "crate::router"),
        ("src/providers/deepseek.rs", "crate::proxy"),
        ("src/providers/deepseek.rs", "crate::router"),
        ("src/providers/moonshot.rs", "crate::proxy"),
        ("src/providers/moonshot.rs", "crate::router"),
        ("src/providers/ollama.rs", "crate::proxy"),
        ("src/providers/ollama.rs", "crate::router"),
        ("src/providers/openrouter.rs", "crate::proxy"),
        ("src/providers/openrouter.rs", "crate::router"),
        ("src/providers/retry.rs", "crate::proxy"),
        ("src/providers/retry.rs", "crate::router"),
        // §4.3: http.rs / stream_util.rs must not depend on a concrete Provider.
        ("src/providers/http.rs", "crate::providers::openai"),
        ("src/providers/http.rs", "crate::providers::anthropic"),
        ("src/providers/http.rs", "crate::providers::google"),
        ("src/providers/http.rs", "crate::providers::deepseek"),
        ("src/providers/http.rs", "crate::providers::moonshot"),
        ("src/providers/http.rs", "crate::providers::ollama"),
        ("src/providers/http.rs", "crate::providers::openrouter"),
        ("src/providers/http.rs", "crate::providers::compat"),
        ("src/providers/http.rs", "crate::providers::retry"),
        ("src/providers/stream_util.rs", "crate::providers::openai"),
        (
            "src/providers/stream_util.rs",
            "crate::providers::anthropic",
        ),
        ("src/providers/stream_util.rs", "crate::providers::google"),
        ("src/providers/stream_util.rs", "crate::providers::deepseek"),
        ("src/providers/stream_util.rs", "crate::providers::moonshot"),
        ("src/providers/stream_util.rs", "crate::providers::ollama"),
        (
            "src/providers/stream_util.rs",
            "crate::providers::openrouter",
        ),
        ("src/providers/stream_util.rs", "crate::providers::compat"),
        ("src/providers/stream_util.rs", "crate::providers::retry"),
        // §4.3: lib.rs / default feature must not pull in proxy-only deps.
        ("src/lib.rs", "axum"),
        ("src/lib.rs", "tower"),
    ]
}

/// Strip `//` line comments and `/* … */` block comments from Rust source.
///
/// Needed before scanning for `use` statements: a commented-out import
/// (`// use crate::proxy::Router;` or `/* use crate::proxy::Router; */`) is NOT a
/// dependency edge and must not be reported (false positive). The previous
/// line-prefix scanner flagged it.
pub fn strip_comments(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    let mut in_line = false;
    let mut in_block = false;
    while i < bytes.len() {
        let c = bytes[i];
        if in_line {
            if c == b'\n' {
                in_line = false;
                out.push('\n');
            }
            i += 1;
            continue;
        }
        if in_block {
            if c == b'*' && bytes.get(i + 1) == Some(&b'/') {
                in_block = false;
                i += 2;
            } else {
                if c == b'\n' {
                    out.push('\n');
                }
                i += 1;
            }
            continue;
        }
        if c == b'/' && bytes.get(i + 1) == Some(&b'/') {
            in_line = true;
            i += 2;
            continue;
        }
        if c == b'/' && bytes.get(i + 1) == Some(&b'*') {
            in_block = true;
            i += 2;
            continue;
        }
        // Keep bytes as-is; multi-byte UTF-8 passes through untouched.
        out.push(c as char);
        i += 1;
    }
    out
}

/// Collect every `use` **statement** in the source, whitespace-normalized.
///
/// Robust against the bypass forms the line-prefix scanner missed
/// (`GRD-004`):
/// - `pub use …`, `pub(crate) use …`, `pub(super) use …` (any visibility);
/// - attributes on the preceding line (`#[cfg(test)] use …`);
/// - `use` inside a macro invocation body (`some_macro! { use …; }`);
/// - **multi-line** `use` statements (collected up to the terminating `;`).
///
/// Comment content is removed first, so commented-out imports are ignored.
pub fn use_statements(src: &str) -> Vec<String> {
    let cleaned = strip_comments(src);
    let mut out = Vec::new();
    let chars: Vec<char> = cleaned.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        // Find the token `use` at a word boundary.
        if chars[i] == 'u'
            && i + 3 <= chars.len()
            && chars[i..(i + 3).min(chars.len())]
                .iter()
                .collect::<String>()
                == "use"
            && (i == 0 || !is_ident_char(chars[i - 1]))
            && (i + 3 == chars.len() || !is_ident_char(chars[i + 3]))
        {
            // Collect until the terminating ';' (multi-line safe).
            let start = i;
            let mut j = i;
            let mut stmt = String::new();
            let mut depth = 0i32;
            while j < chars.len() {
                let ch = chars[j];
                if ch == '{' {
                    depth += 1;
                } else if ch == '}' {
                    depth -= 1;
                }
                if ch == ';' && depth <= 0 {
                    break;
                }
                stmt.push(ch);
                j += 1;
            }
            let normalized = stmt.split_whitespace().collect::<Vec<_>>().join(" ");
            if !normalized.is_empty() {
                out.push(normalized);
            }
            i = j.max(start + 3);
            continue;
        }
        i += 1;
    }
    out
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// **纯函数**：给定 `(file, source)` 与禁用边表，返回违规。
///
/// 抽出来是为了让每种绕过形态都能被负例直接驱动，不必造真文件。
pub fn edge_violations(files: &[(String, String)], edges: &[(String, String)]) -> Vec<String> {
    let mut violations = Vec::new();
    for (file, src) in files {
        for stmt in use_statements(src) {
            // 匹配用"去空白形态"：跨行 use 归一化后会带上空格
            // （`use crate::\n proxy::Router;` → `use crate:: proxy::Router`），
            // 直接 contains("crate::proxy") 会漏判。
            let compact: String = stmt.chars().filter(|c| !c.is_whitespace()).collect();
            for (edge_file, prefix) in edges {
                if edge_file == file && compact.contains(prefix.as_str()) {
                    violations.push(format!("{file}: `{stmt}` (forbidden edge `{prefix}`)"));
                }
            }
        }
    }
    violations
}

/// 读取全部边引用文件，**保留读取失败的事实**：`None` = 文件缺失或不可读。
///
/// 读取失败不允许在收集阶段被吞掉——fail-closed 的前提是守卫能看见
/// 「有边指向的文件读不到」这件事本身（SPCC §9.2 第 1 条）。模块被移动或
/// 改名后，指向它的禁用边绝不能静默失效。
fn read_edge_sources(edge_files: &BTreeSet<String>) -> Vec<(String, Option<String>)> {
    edge_files
        .iter()
        .map(|file| {
            let src = fs::read_to_string(Path::new(file));
            (file.clone(), src.ok())
        })
        .collect()
}

/// **纯函数**：依赖边守卫的 fail-closed 判定核心（#15）。
///
/// 与 [`edge_violations`] 的分工：[`edge_violations`] 只判「读到的源码里有没有
/// 禁用边」；本函数在其上再立一条——**边引用的文件缺失/不可读，本身就是违规**。
///
/// 旧实现把 `fs::read_to_string` 的 `Err` 分支静默吞掉后直接跳过该文件：
/// 模块被移动/改名后，指向它的禁用边全体失效，守卫却仍然全绿（fail-open，
/// 违反 SPCC §9.2 第 1 条「守卫判门 fail-closed」）。fail-closed 的含义是：
/// 读不到输入就产出违规，而不是安静地绿。
pub fn dependency_edge_problems(
    sources: &[(String, Option<String>)],
    edges: &[(String, String)],
) -> Vec<String> {
    let readable: Vec<(String, String)> = sources
        .iter()
        .filter_map(|(file, src)| src.as_ref().map(|s| (file.clone(), s.clone())))
        .collect();
    let mut problems = edge_violations(&readable, edges);
    for (file, src) in sources {
        if src.is_none() {
            problems.push(format!(
                "{file}: edge-referenced file is unreadable (missing or not readable after a \
                 move/rename); fail-closed per SPCC §9.2 item 1 — an unreadable input is a \
                 violation, never a silently skipped check"
            ));
        }
    }
    problems
}

#[test]
fn dependency_edges_are_respected() {
    let edges: Vec<(String, String)> = forbidden_edges()
        .into_iter()
        .map(|(f, p)| (f.to_string(), p.to_string()))
        .collect();
    let edge_files: BTreeSet<String> = edges.iter().map(|(f, _)| f.clone()).collect();

    let sources = read_edge_sources(&edge_files);
    let violations = dependency_edge_problems(&sources, &edges);
    assert!(
        violations.is_empty(),
        "Dependency-edge violations found (SPCC §4.3):\n{}",
        violations.join("\n")
    );
}

/// #15 负例：**边引用的文件缺失（如模块被移动/改名）时，守卫必须红**。
///
/// 与 [`dependency_edges_are_respected`] 走完全相同的判定路径
/// （`read_edge_sources` → `dependency_edge_problems`），唯一差别是边表里多了一条
/// 指向不存在文件的边：产出的违规必须非空、且点到该文件名。把 fail-closed
/// 逻辑回退成「读不到的文件静默跳过」的任何形态，本用例都必须红。
#[test]
fn dependency_edge_guard_fails_closed_on_missing_edge_file() {
    let missing = "src/does_not_exist_renamed_module.rs";
    assert!(
        !Path::new(missing).exists(),
        "fixture precondition: {missing} must not exist on disk \
         (it stands for a moved/renamed module)"
    );
    let edges: Vec<(String, String)> = vec![
        ("src/types.rs".to_string(), "crate::proxy".to_string()),
        (missing.to_string(), "crate::proxy".to_string()),
    ];
    let edge_files: BTreeSet<String> = edges.iter().map(|(f, _)| f.clone()).collect();

    let problems = dependency_edge_problems(&read_edge_sources(&edge_files), &edges);
    assert!(
        !problems.is_empty(),
        "an edge-referenced file that cannot be read MUST produce a violation \
         (fail-closed, SPCC §9.2 item 1); a silently skipped unreadable file is \
         exactly the #15 fail-open hole, got {problems:?}"
    );
    assert!(
        problems.iter().any(|p| p.contains(missing)),
        "the violations must name the missing edge file {missing}, got {problems:?}"
    );
}

#[test]
fn hotspot_files_do_not_grow() {
    let json =
        fs::read_to_string("tests/hotspot_ledger.json").expect("hotspot_ledger.json must exist");
    let ledger: serde_json::Value = serde_json::from_str(&json).expect("ledger must be valid JSON");
    let baseline = ledger["baseline_lines"]
        .as_object()
        .expect("ledger must contain baseline_lines object");

    let mut violations = Vec::new();
    for (file, value) in baseline {
        let expected = value
            .as_u64()
            .expect("baseline must be an integer line count");
        let path = Path::new(file);
        let current = if path.exists() {
            fs::read_to_string(path).unwrap_or_default().lines().count() as u64
        } else {
            0
        };
        if current != expected {
            violations.push(format!(
                "{file}: {current} lines != baseline {expected} (hotspot drift in either direction forbidden by SPCC §4.4; shrink = REL-003 truncation class)"
            ));
        }
    }
    assert!(
        violations.is_empty(),
        "Hotspot-ledger violations:\n{}",
        violations.join("\n")
    );
}

/// Sanity: the ledger itself is internally consistent (every entry is a real,
/// existing source file at or above the ledger's own `threshold`).
#[test]
fn hotspot_ledger_is_consistent() {
    let json = fs::read_to_string("tests/hotspot_ledger.json").unwrap();
    let ledger: serde_json::Value = serde_json::from_str(&json).unwrap();
    // Threshold is a single source of truth (#209): a missing/invalid threshold is
    // itself a ledger problem — fail closed instead of silently defaulting.
    let threshold = ledger["threshold"].as_u64();
    let baseline = ledger["baseline_lines"].as_object().unwrap();
    let mut problems = Vec::new();
    let Some(threshold) = threshold else {
        problems.push("threshold: missing or not an integer".to_string());
        assert!(
            problems.is_empty(),
            "Ledger problems:\n{}",
            problems.join("\n")
        );
        return;
    };
    for (file, value) in baseline {
        let lines = value.as_u64().unwrap();
        if lines < threshold {
            problems.push(format!("{file}: {lines} < threshold {threshold}"));
        }
        if !Path::new(file).exists() {
            problems.push(format!("{file}: file does not exist"));
        }
    }
    assert!(
        problems.is_empty(),
        "Ledger problems:\n{}",
        problems.join("\n")
    );

    // NOTE (AUD#15): the scrub-map that used to sit here (written into, never
    // read — dead code) was removed. The intent it claimed — "the guard must not
    // silently skip a moved/renamed edge file" — is now actually enforced, and
    // fail-closed, by the dependency-edge guard above: see
    // `dependency_edge_problems` and its negative test
    // `dependency_edge_guard_fails_closed_on_missing_edge_file`.
}

/// Recursively collect `(repo-relative path, line count)` for every `.rs` file
/// under `root`. Paths use `/` separators so they match ledger keys on every OS.
fn rust_files_with_line_counts(root: &Path) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let rel = path.to_string_lossy().replace('\\', "/");
                let lines = fs::read_to_string(&path)
                    .unwrap_or_default()
                    .lines()
                    .count() as u64;
                out.push((rel, lines));
            }
        }
    }
    out
}

/// Pure coverage rule (#209): every file at or above `threshold` MUST be listed.
/// Extracted from the test so a negative case can drive it directly.
fn coverage_violations(
    threshold: u64,
    listed: &BTreeSet<String>,
    files: &[(String, u64)],
) -> Vec<String> {
    files
        .iter()
        .filter(|(file, lines)| *lines >= threshold && !listed.contains(file))
        .map(|(file, lines)| {
            format!(
                "{file}: {lines} lines >= threshold {threshold} but is NOT on hotspot_ledger baseline_lines \
                 (unlisted oversized file — escapes both grow and shrink checks; register it, do not raise the threshold)"
            )
        })
        .collect()
}

/// #209 guard: the ledger must be a full boundary, not a hand-maintained name list.
#[test]
fn hotspot_coverage_is_complete() {
    let json = fs::read_to_string("tests/hotspot_ledger.json").expect("ledger must exist");
    let ledger: serde_json::Value = serde_json::from_str(&json).expect("ledger must be valid JSON");
    let threshold = ledger["threshold"]
        .as_u64()
        .expect("ledger must declare an integer threshold (no silent default)");
    let listed: BTreeSet<String> = ledger["baseline_lines"]
        .as_object()
        .expect("ledger must contain baseline_lines object")
        .keys()
        .cloned()
        .collect();

    let files = rust_files_with_line_counts(Path::new("src"));
    assert!(
        !files.is_empty(),
        "no .rs files found under src/ — the coverage scan is not looking where it thinks it is"
    );
    let violations = coverage_violations(threshold, &listed, &files);
    assert!(
        violations.is_empty(),
        "Hotspot coverage violations (SPCC §4.4, #209):\n{}",
        violations.join("\n")
    );
}

/// Negative test for the #209 coverage rule: an unlisted file at/above the
/// threshold MUST be reported. Without this the guard would be decoration
/// (SPCC §3: a guard without a negative test is not a gate).
#[test]
fn hotspot_coverage_detects_unlisted_oversized_file() {
    let listed: BTreeSet<String> = ["src/listed_big.rs".to_string()].into_iter().collect();
    let files = vec![
        ("src/listed_big.rs".to_string(), 900_u64),
        ("src/unlisted_big.rs".to_string(), 500_u64), // exactly at threshold → must be flagged
        ("src/small.rs".to_string(), 499_u64),        // just below → must NOT be flagged
    ];
    let violations = coverage_violations(500, &listed, &files);
    assert_eq!(
        violations.len(),
        1,
        "expected exactly the unlisted oversized file to be flagged, got: {violations:?}"
    );
    assert!(
        violations[0].contains("src/unlisted_big.rs"),
        "the flagged file must be the unlisted one, got: {}",
        violations[0]
    );
}

// ── GRD-004 负例：每种绕过形态都必须被拦（SPCC §3） ──────────────────────

fn edges(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(f, p)| (f.to_string(), p.to_string()))
        .collect()
}

fn files(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(f, s)| (f.to_string(), s.to_string()))
        .collect()
}

const TARGET: &str = "src/types.rs";
const FORBIDDEN: &str = "crate::proxy";

/// 形态 1：朴素 `use`（基线，旧扫描器也能抓到）。
#[test]
fn grd004_plain_use_is_caught() {
    let v = edge_violations(
        &files(&[(TARGET, "use crate::proxy::Router;\n")]),
        &edges(&[(TARGET, FORBIDDEN)]),
    );
    assert_eq!(v.len(), 1, "got {v:?}");
}

/// 形态 2：`pub use`（旧扫描器抓得到，钉住不回归）。
#[test]
fn grd004_pub_use_is_caught() {
    let v = edge_violations(
        &files(&[(TARGET, "pub use crate::proxy::Router;\n")]),
        &edges(&[(TARGET, FORBIDDEN)]),
    );
    assert_eq!(v.len(), 1, "got {v:?}");
}

/// 形态 3：**`pub(crate) use`** —— 旧扫描器 `starts_with("pub use ")` 抓不到。
#[test]
fn grd004_pub_crate_use_is_caught() {
    let v = edge_violations(
        &files(&[(TARGET, "pub(crate) use crate::proxy::Router;\n")]),
        &edges(&[(TARGET, FORBIDDEN)]),
    );
    assert_eq!(v.len(), 1, "got {v:?}");
}

/// 形态 4：**带属性的 `use`**（属性在上一行）—— 旧扫描器逐行看前缀，抓不到。
#[test]
fn grd004_attributed_use_is_caught() {
    let v = edge_violations(
        &files(&[(TARGET, "#[cfg(test)]\nuse crate::proxy::Router;\n")]),
        &edges(&[(TARGET, FORBIDDEN)]),
    );
    assert_eq!(v.len(), 1, "got {v:?}");
}

/// 形态 5：**宏体内的 `use`** —— 旧扫描器（行首前缀）抓不到。
#[test]
fn grd004_macro_internal_use_is_caught() {
    let v = edge_violations(
        &files(&[(TARGET, "some_macro! { use crate::proxy::Router; }\n")]),
        &edges(&[(TARGET, FORBIDDEN)]),
    );
    assert_eq!(v.len(), 1, "got {v:?}");
}

/// 形态 6：**跨行 `use`** —— 路径被换行拆开，旧扫描器只看单行，抓不到。
#[test]
fn grd004_multiline_use_is_caught() {
    let v = edge_violations(
        &files(&[(TARGET, "use crate::\n    proxy::Router;\n")]),
        &edges(&[(TARGET, FORBIDDEN)]),
    );
    assert_eq!(v.len(), 1, "got {v:?}");
}

/// 形态 7：**花括号多行 use** —— `use crate::proxy::{A,\n B};`。
#[test]
fn grd004_braced_multiline_use_is_caught() {
    let v = edge_violations(
        &files(&[(
            TARGET,
            "use crate::proxy::{\n    Router,\n    Server,\n};\n",
        )]),
        &edges(&[(TARGET, FORBIDDEN)]),
    );
    assert_eq!(v.len(), 1, "got {v:?}");
}

/// 反例（**假阳性**）：注释掉的 import **不得**被判违规。
///
/// 旧扫描器会把 `/* use crate::proxy::Router; */` 这类内容当成边（因为它只看行首）。
#[test]
fn grd004_commented_out_use_is_not_flagged() {
    let src = "// use crate::proxy::Router;\n\
               /* use crate::proxy::Server; */\n\
               /*\n use crate::proxy::Inner;\n */\n";
    let v = edge_violations(&files(&[(TARGET, src)]), &edges(&[(TARGET, FORBIDDEN)]));
    assert!(
        v.is_empty(),
        "commented-out imports must not be flagged; got {v:?}"
    );
}

/// 反例（**假阳性**）：把禁用串写进字符串字面量/普通标识符里不得误伤。
#[test]
fn grd004_unrelated_mentions_are_not_flagged() {
    let src = "let note = \"crate::proxy is forbidden\";\n\
               fn crate_proxy_helper() {}\n";
    let v = edge_violations(&files(&[(TARGET, src)]), &edges(&[(TARGET, FORBIDDEN)]));
    assert!(
        v.is_empty(),
        "non-import mentions must not be flagged; got {v:?}"
    );
}

/// 边界：不同文件的边**不得**互串（edge 表按文件匹配）。
#[test]
fn grd004_edges_are_file_scoped() {
    let v = edge_violations(
        &files(&[("src/router.rs", "use crate::proxy::Router;\n")]),
        &edges(&[(TARGET, FORBIDDEN)]),
    );
    assert!(
        v.is_empty(),
        "an edge for {TARGET} must not fire on another file; got {v:?}"
    );
}
