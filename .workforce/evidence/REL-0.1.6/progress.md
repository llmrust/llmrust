# REL-0.1.6 发版进度（dev-llmrust）

- repo: D:\llmrust-0.1.5, branch `main`, 起点 HEAD `3ed46b0`（= origin/main）
- 起始版本 0.1.5，已有 tag v0.1.0 / v0.1.1 / v0.1.3 / v0.1.4 / v0.1.5

## 1. 查到的发版惯例（已核实）

来源：`bf7dee9`（0.1.5 release 提交，squash 自 `764bc2f`，tag `v0.1.5`=a9fe0c0）、
`RELEASE_CHECKLIST.md`、`.github/scripts/release-validate.sh`、`tests/agent_docs_validation.rs`。

### "四处一致"（release-validate.sh Gate 2 原文）
tag == `Cargo.toml` version == `llmrust.capabilities.json` version == `CHANGELOG.md` section：

1. git tag `vX.Y.Z`
2. `Cargo.toml` `version = "X.Y.Z"`
3. `llmrust.capabilities.json` 的 `"version"`（机器门 `capabilities_json_version_matches_crate_version`）
4. `CHANGELOG.md` 首标题 `## [X.Y.Z] - YYYY-MM-DD`（机器门 `changelog_first_heading_matches_cargo_version`；
   日期必须真实，占位符/`[Unreleased]` 一律红；0.1.3/0.1.1/0.1.0 历史标题必须保留）

### 0.1.5 实际动的 7 个文件（零源码）
`Cargo.toml`、`Cargo.lock`（恰 1 行 llmrust 自身版本）、`CHANGELOG.md`、
`llmrust.capabilities.json`、`llmrust.models.json`、`README.md`、`README.zh-CN.md`。
README 双语：当前版本段升新版、旧版降为 `<details>` 折叠保留原文。

### 发布通道
`.github/workflows/release.yml` **只有** `on: push: tags: v[0-9]+.[0-9]+.[0-9]+`，
**无 workflow_dispatch**（无旁路条款）⇒ 推 tag 自动触发 validate → dry-run → publish；
dry-run 与 publish 两 job 均挂 `environment: release`（required_reviewers=bishuan）
⇒ Owner 需在 GitHub 上点 **2 次**环境审批。不手动触发 workflow、不本地 `cargo publish`。

### 其它硬门
- Gate 3：`git status --porcelain -uall` 必须完全为空（故未跟踪的
  `.workforce/evidence/B-DELTA-NULL/ci.json` 随本次提交入库，Lead 已批准）。
- Gate 4：默认特性不得拉入 `axum`（实测 `cargo tree -e no-dev -i axum` 无匹配 ⇒ PASS）。
- `core.autocrlf=false` 保持不动。

## 2. 改动清单（已完成）

- [x] Cargo.toml 0.1.5 → 0.1.6
- [x] Cargo.lock（llmrust 包版本行，恰 1 行）
- [x] CHANGELOG.md 新增 `## [0.1.6] - 2026-11-20`（Fixed + Known issues）
- [x] llmrust.capabilities.json version
- [x] llmrust.models.json version
- [x] README.md（当前段升 0.1.6，0.1.5 降 details，附 Known issue）
- [x] README.zh-CN.md（同上中文）
- [x] `.workforce/evidence/B-DELTA-NULL/ci.json` 入库（满足 Gate 3）

## 3. 行尾事故与修复（已解决）

编辑工具把 7 个文件物化为 CRLF（HEAD blob 为 LF，`core.autocrlf=false`）⇒ `git diff --stat`
显示整文件漂移（如 CHANGELOG 1015 行）。已按 HEAD 的 LF 原始字节重写回 LF，diff 收敛为
2/2/49/33/13/2/2 行。`core.autocrlf` 未改动（仍 false）。

## 4. 本地四门（全 exit 0）

- cargo build --workspace
- cargo test --workspace（19 个测试二进制，0 failed）
- cargo fmt --all --check
- cargo clippy --workspace --all-targets -- -D warnings

## 5. 待办

- [ ] commit（消息含 task-ids: REL-0.1.6）
- [ ] push main
- [ ] tag v0.1.6 + push tag
- [ ] 等 CI（main push run + tag release 流水线）
- [ ] release-result.json
