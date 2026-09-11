//! `H-4` 门禁（`test_hygiene_guard`）：**负例的判定必须来自逻辑，而不是时钟或进程全局状态**。
//!
//! 外部审计第 4 项（"门禁负例补真"）。实测到的两类缺陷（本仓各一处/一批）：
//!
//! 1. **计时型判定**：`serve_rejects_blank_or_empty_key` 原先用「2 秒内是否返回」判定
//!    `serve()` 是否拒绝空白 key —— **红/绿取决于机器负载**（实测在负载下误红，
//!    见 PR：注入验红那次它被连带判红，而干净树连跑 3 次全绿）。这类"负例"不驱动逻辑，
//!    它驱动的是**调度器**。
//! 2. **进程全局状态**：`std::env::set_var` / `remove_var` 影响**同进程内并发运行的所有测试**。
//!    改动它而不与同类测试串行 ⇒ 结果取决于**测试调度顺序**（与"共享 `static` 捕获槽"
//!    同类的隐患；本卡在 H-1/H-3 期间已在真实测试里踩到过一次同族问题）。
//!
//! 本门把这两条**机器化**，使它们不再依赖人记得：
//!
//! - ① 任何 `env::set_var` / `env::remove_var` 所在的**测试函数**必须持有 `ENV_LOCK`；
//! - ② 测试里的 `timeout(...)` 只允许作为**死锁安全网**（≥ 30 秒），不得用作延迟断言。
//!
//! 口径说明：本门**扫描源码文本**（它守护的正是"源码里有没有这类写法"），
//! 故自身也必须是**纯函数 + 负例驱动**的（与仓内其它 `*_guard` 一致）。

use std::fs;
use std::path::{Path, PathBuf};

/// 判定问题清单的**最小超时秒数**：低于此值的 `timeout(...)` 视为延迟断言。
const MIN_SAFETY_TIMEOUT_SECS: u64 = 30;

/// 扫描根：`src/` 与 `tests/` 下的全部 `.rs`。
/// 本门**排除自己**：本文件里的负例夹具是"坏代码**作为数据**"（字面量），
/// 若一并扫描，门会抓到自己（实测过一次）。这是**如实披露的边界**：
/// 排除名单**只允许这一个文件**，由 `negative_exemption_is_only_this_file` 钉住。
const EXEMPT: &str = "test_hygiene_guard.rs";

fn rust_sources() -> Vec<PathBuf> {
    rust_sources_including_self()
        .into_iter()
        .filter(|p| !p.to_string_lossy().ends_with(EXEMPT))
        .collect()
}

/// 含自己的全集（供"排除名单不许扩大"的测试使用）。
fn rust_sources_including_self() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for root in ["src", "tests"] {
        collect(Path::new(root), &mut out);
    }
    out.sort();
    out
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// 把字符串字面量、字符字面量与注释的内容**替换成空格**（等长，索引不变）。
///
/// 为什么必须做：首版直接在原文本上配平大括号，结果 `build_request("{")` 里的
/// **字符串内含花括号**把配平带偏，把下一个测试吞进"函数体"⇒ **假阳性**
/// （实测：`malformed_json_returns_proxy_error_body` 被报成"用了 0s 超时"）。
fn strip_literals_and_comments(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0usize;
    while i < b.len() {
        match b[i] {
            b'/' if i + 1 < b.len() && b[i + 1] == b'/' => {
                while i < b.len() && b[i] != b'\n' {
                    out[i] = b' ';
                    i += 1;
                }
            }
            b'/' if i + 1 < b.len() && b[i + 1] == b'*' => {
                out[i] = b' ';
                out[i + 1] = b' ';
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    if b[i] != b'\n' {
                        out[i] = b' ';
                    }
                    i += 1;
                }
                if i + 1 < b.len() {
                    out[i] = b' ';
                    out[i + 1] = b' ';
                    i += 2;
                }
            }
            b'"' => {
                out[i] = b' ';
                i += 1;
                while i < b.len() {
                    if b[i] == b'\\' {
                        out[i] = b' ';
                        if i + 1 < b.len() {
                            out[i + 1] = b' ';
                        }
                        i += 2;
                        continue;
                    }
                    let done = b[i] == b'"';
                    out[i] = b' ';
                    i += 1;
                    if done {
                        break;
                    }
                }
            }
            _ => i += 1,
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| src.to_string())
}

/// 把源码切成 `(函数名, 函数体)`。只认 `fn` 声明；用大括号配平取体。
fn functions(src: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let stripped = strip_literals_and_comments(src);
    let bytes = stripped.as_bytes();
    let mut i = 0usize;
    while let Some(pos) = src[i..].find("fn ") {
        let start = i + pos;
        // 取函数名
        let name_start = start + 3;
        let name_end = stripped[name_start..]
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .map(|o| name_start + o)
            .unwrap_or(stripped.len());
        let name = stripped[name_start..name_end].to_string();
        // 从名字后第一个 '{' 起配平
        if let Some(brace) = stripped[name_end..].find('{') {
            let body_start = name_end + brace;
            let mut depth = 0i32;
            let mut j = body_start;
            while j < bytes.len() {
                match bytes[j] {
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            if j < bytes.len() {
                out.push((name, src[body_start..=j].to_string()));
                i = j;
                continue;
            }
        }
        i = name_end.max(start + 1);
    }
    out
}

/// ① 改动进程环境变量的函数必须持锁。
fn env_mutation_problems(src: &str, file: &str) -> Vec<String> {
    let mut problems = Vec::new();
    for (name, body) in functions(src) {
        let mutates = body.contains("env::set_var(") || body.contains("env::remove_var(");
        if !mutates {
            continue;
        }
        // `ENV_LOCK` 的定义体本身当然不含持锁语句，跳过定义。
        if name == "new" && body.contains("Mutex::new") {
            continue;
        }
        if !body.contains("ENV_LOCK") {
            problems.push(format!(
                "{file}: `{name}` mutates process env without holding ENV_LOCK \
                 (parallel tests would interfere)"
            ));
        }
    }
    problems
}

/// ② 测试里的 `timeout(...)` 只允许作死锁安全网（≥ `MIN_SAFETY_TIMEOUT_SECS`）。
fn short_timeout_problems(src: &str, file: &str) -> Vec<String> {
    let mut problems = Vec::new();
    for (name, body) in functions(src) {
        if !body.contains("timeout(") {
            continue;
        }
        for line in body.lines() {
            let l = line.trim();
            if !l.contains("from_secs(") && !l.contains("from_millis(") {
                continue;
            }
            if let Some(v) = extract_secs(l) {
                if v < MIN_SAFETY_TIMEOUT_SECS {
                    problems.push(format!(
                        "{file}: `{name}` uses a {v}s timeout — too short for a hang safety net; \
                         a timing-based verdict is load-dependent (use a pure predicate instead)"
                    ));
                }
            }
        }
    }
    problems
}

fn extract_secs(line: &str) -> Option<u64> {
    if let Some(p) = line.find("from_secs(") {
        let rest = &line[p + 10..];
        let end = rest.find(|c: char| !c.is_ascii_digit())?;
        return rest[..end].parse().ok();
    }
    if let Some(p) = line.find("from_millis(") {
        let rest = &line[p + 12..];
        let end = rest.find(|c: char| !c.is_ascii_digit())?;
        return rest[..end].parse::<u64>().ok().map(|ms| ms / 1000);
    }
    None
}

#[test]
fn env_mutating_tests_hold_the_lock() {
    let mut problems = Vec::new();
    for path in rust_sources() {
        let src = fs::read_to_string(&path).expect("read source");
        problems.extend(env_mutation_problems(&src, &path.display().to_string()));
    }
    assert!(
        problems.is_empty(),
        "H-4: process-env mutations must be serialized via ENV_LOCK:\n{}",
        problems.join("\n")
    );
}

#[test]
fn test_timeouts_are_safety_nets_not_latency_assertions() {
    let mut problems = Vec::new();
    for path in rust_sources() {
        let src = fs::read_to_string(&path).expect("read source");
        problems.extend(short_timeout_problems(&src, &path.display().to_string()));
    }
    assert!(
        problems.is_empty(),
        "H-4: a short timeout makes a negative's verdict depend on machine load:\n{}",
        problems.join("\n")
    );
}

// ── 负例：本门自身必须能被"改坏"驱动 ──────────────────────────────────────

#[test]
fn negative_env_mutation_without_lock_is_reported() {
    let src = r#"
        #[test]
        fn bad() {
            std::env::set_var("X", "1");
        }
    "#;
    let p = env_mutation_problems(src, "fake.rs");
    assert_eq!(
        p.len(),
        1,
        "unlocked env mutation must be reported, got {p:?}"
    );
    assert!(p[0].contains("bad"), "the report must name the function");
}

#[test]
fn negative_short_timeout_is_reported() {
    let src = r#"
        #[tokio::test]
        async fn bad() {
            let _ = tokio::time::timeout(Duration::from_secs(2), serve(x, "a")).await;;
        }
    "#;
    let p = short_timeout_problems(src, "fake.rs");
    assert_eq!(p.len(), 1, "a 2s timeout must be reported, got {p:?}");
}

#[test]
fn negative_negative_already_holding_lock_is_not_reported() {
    let src = r#"
        #[test]
        fn good() {
            let _g = ENV_LOCK.lock().unwrap();
            std::env::set_var("X", "1");
            std::env::remove_var("X");
        }
    "#;
    assert!(env_mutation_problems(src, "fake.rs").is_empty());
}

#[test]
fn negative_generous_timeout_is_not_reported() {
    let src = r#"
        #[tokio::test]
        async fn good() {
            let _ = tokio::time::timeout(Duration::from_secs(30), serve(x, "a")).await;;
        }
    "#;
    assert!(short_timeout_problems(src, "fake.rs").is_empty());
}

#[test]
fn negative_exemption_is_only_this_file() {
    let all = rust_sources_including_self();
    let scanned = rust_sources();
    let exempt: Vec<String> = all
        .iter()
        .filter(|p| !scanned.contains(p))
        .map(|p| p.display().to_string())
        .collect();
    assert_eq!(
        exempt.len(),
        1,
        "H-4: the self-exemption must cover EXACTLY one file, got {exempt:?}"
    );
    assert!(exempt[0].ends_with(EXEMPT), "exemption must be {EXEMPT}");
    assert!(
        all.len() >= 10,
        "the scan must find a realistic number of sources, got {}",
        all.len()
    );
}

#[test]
fn negative_real_repo_is_clean_today() {
    // 与真门同一判据，显式钉住"当前仓库是干净的"，防本门被悄悄放宽。
    for path in rust_sources() {
        let src = fs::read_to_string(&path).expect("read source");
        let f = path.display().to_string();
        assert!(env_mutation_problems(&src, &f).is_empty(), "{f}");
        assert!(short_timeout_problems(&src, &f).is_empty(), "{f}");
    }
}
