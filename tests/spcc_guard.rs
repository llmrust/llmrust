//! `GRD-001` 第 ④ 项：**治理文档行数地板守卫**。
//!
//! 数值型门禁在大文件（`src/**`）上早已双向（`GRD-001` 主体，`architecture_guard.rs`）；
//! 但**治理文档本身**没有守卫——规格/能力表可以被静默截断而无人发现。
//! REL-003 的截断事故正是这一类（`E-101` 亦为其产物）。
//!
//! 本守卫的口径：
//!
//! - 地板值登记在 `tests/spcc_line_baseline.json`（**`wc -l`，即换行符数**，依 `E-101`）；
//! - 文档**可以自由增长**（我们天天在加行），但**不得低于其登记地板**；
//! - 低于地板 = **红**，且失败信息必须给出 **truth correction 指引**（下述 [`TRUTH_CORRECTION_HINT`]）；
//! - **降低地板是"经授权的事实更正"**，须在该 JSON 的 `description`/提交里写明理由——
//!   原文纪律："Do not inflate a number to turn a guard green."（本守卫为其镜像：**也不许悄悄调低**）。
//!
//! 本地跑：`cargo test --test spcc_guard`

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// 失败信息里的授权指引（`GRD-001` DoD ③ 要求）。
pub const TRUTH_CORRECTION_HINT: &str = "If the reduction is intentional, it is an authorized truth correction: \
record the reason and get architect authorization, then lower the floor in tests/spcc_line_baseline.json. \
Do not lower it merely to turn this guard green.";

/// `wc -l` 口径：换行符个数。
pub fn wc_l(text: &str) -> u64 {
    text.bytes().filter(|b| *b == b'\n').count() as u64
}

/// 纯函数：给定地板表与"路径 → 实际行数"，返回违规项。
///
/// 抽成纯函数是为了让负例可以直接驱动它（不触碰真实文件）。
pub fn floor_violations(
    floors: &BTreeMap<String, u64>,
    actual: &BTreeMap<String, u64>,
) -> Vec<String> {
    let mut out = Vec::new();
    for (path, floor) in floors {
        match actual.get(path) {
            None => out.push(format!(
                "{path}: recorded floor {floor} but the file is MISSING (evidence evaporated)"
            )),
            Some(&n) if n < *floor => out.push(format!(
                "{path}: {n} lines < recorded floor {floor} (possible silent truncation). {TRUTH_CORRECTION_HINT}"
            )),
            Some(_) => {}
        }
    }
    out
}

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn load_floors() -> BTreeMap<String, u64> {
    let text = fs::read_to_string(repo_root().join("tests").join("spcc_line_baseline.json"))
        .expect("tests/spcc_line_baseline.json must exist");
    let parsed: serde_json::Value =
        serde_json::from_str(&text).expect("tests/spcc_line_baseline.json must be valid JSON");
    let obj = parsed["floors"]
        .as_object()
        .expect("spcc_line_baseline.json must contain a `floors` object");
    obj.iter()
        .map(|(k, v)| {
            (
                k.clone(),
                v.as_u64()
                    .expect("each floor must be an integer line count"),
            )
        })
        .collect()
}

/// **守卫**：每份治理文档不得低于其登记地板。
#[test]
fn governance_docs_do_not_fall_below_their_floors() {
    let floors = load_floors();
    assert!(!floors.is_empty(), "the floor ledger must not be empty");

    let mut actual = BTreeMap::new();
    for path in floors.keys() {
        let text = fs::read_to_string(repo_root().join(path))
            .unwrap_or_else(|e| panic!("{path}: cannot read ({e})"));
        actual.insert(path.clone(), wc_l(&text));
    }

    let violations = floor_violations(&floors, &actual);
    assert!(
        violations.is_empty(),
        "Governance-document floor violations (GRD-001 §4):\n{}",
        violations.join("\n")
    );
}

/// 地板表非空且口径自洽：每条地板 > 0（0 地板等于没有守卫）。
#[test]
fn floors_are_positive() {
    for (path, floor) in load_floors() {
        assert!(
            floor > 0,
            "{path}: floor must be > 0 (a zero floor is no guard)"
        );
    }
}

// ── 负例：门必须会红（SPCC §3："没有 negative test 的 guard 不算门禁"） ──

fn floors(pairs: &[(&str, u64)]) -> BTreeMap<String, u64> {
    pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

/// 负例一：**人为删若干行**（模拟截断）→ 必须报违规。
#[test]
fn negative_shrunk_doc_is_reported() {
    let f = floors(&[("docs/SPCC-0.1.4.md", 900)]);
    let a: BTreeMap<String, u64> = [("docs/SPCC-0.1.4.md".to_string(), 850u64)]
        .into_iter()
        .collect();
    let v = floor_violations(&f, &a);
    assert_eq!(v.len(), 1, "got {v:?}");
    assert!(v[0].contains("possible silent truncation"), "{v:?}");
    assert!(
        v[0].contains("truth correction"),
        "failure text must carry the truth-correction instruction: {v:?}"
    );
}

/// 负例二：**文件被整个删掉**（证据蒸发）→ 必须报违规。
#[test]
fn negative_missing_doc_is_reported() {
    let f = floors(&[("docs/SPCC-0.1.4.md", 900)]);
    let a: BTreeMap<String, u64> = BTreeMap::new();
    let v = floor_violations(&f, &a);
    assert_eq!(v.len(), 1, "got {v:?}");
    assert!(v[0].contains("MISSING"), "{v:?}");
}

/// 正向对照：增长与持平**都不得**报违规（否则门是"无人能过"的假门）。
#[test]
fn growth_and_equality_pass() {
    let f = floors(&[("docs/SPCC-0.1.4.md", 900)]);
    let grew: BTreeMap<String, u64> = [("docs/SPCC-0.1.4.md".to_string(), 1000u64)]
        .into_iter()
        .collect();
    let equal: BTreeMap<String, u64> = [("docs/SPCC-0.1.4.md".to_string(), 900u64)]
        .into_iter()
        .collect();
    assert!(floor_violations(&f, &grew).is_empty());
    assert!(floor_violations(&f, &equal).is_empty());
}

/// `wc -l` 口径钉死：**只数换行符**，末行无换行不计。
#[test]
fn wc_l_counts_newlines_only() {
    assert_eq!(wc_l("a\nb\n"), 2);
    assert_eq!(wc_l("a\nb"), 1); // 末行无换行 → wc -l 不计
    assert_eq!(wc_l(""), 0);
    assert_eq!(wc_l("\n"), 1);
}
