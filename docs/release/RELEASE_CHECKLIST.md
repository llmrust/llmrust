# 0.1.3 Release Checklist

Operator checklist for the 0.1.3 release (REL-003). Created by REL-002
(`docs/release/RELEASE_CHECKLIST.md`, 2026-08-03) as the release-commit companion.
Publishing channel enabled by REL-003A (`docs/release/RELEASE_CHECKLIST.md` update,
2026-08-03).

> **预期 crate hash 见实现 PR body 与状态回证账本（§11.1.4）** —— 本清单不内嵌 hash 值
> （写入即改变包内容，hash 立即失效，防自引用悖论；MUST-1(c)）。

## REL-003 execution checklist

- [ ] **三证确认**：`git rev-parse main`、实现 merge SHA（`$MERGESHA`）、待打 tag 三者一致；
- [ ] **版本四方一致**：Cargo.toml `0.1.3` / capabilities.json `0.1.3` / CHANGELOG `[0.1.3] - 2026-08-03` / COMPATIBILITY 版本+日期；
- [ ] **CI 全绿**：最新主干 head 七项 check runs 全 success（semver / Arch guards / MSRV / Test / cargo-deny / gitleaks / RustSec）；
- [ ] **release workflow 观察**：push `v0.1.3` tag → `Release (tag-only, dry-run)` workflow 触发；`validate` / `dry-run` / `guards-fmt-clippy` 三闸全绿后 `publish` job 执行（无 `workflow_dispatch` 旁路，`environment: release` required reviewers 门禁）；
- [ ] **publish job 观察**：`rust-lang/crates-io-auth-action`（OIDC 短期身份）交换 → `cargo publish`（无 `--token`）→ crates.io 可见性有界轮询通过（≤300s）；
- [ ] **三方验证**：crates.io 页面 / docs.rs / GitHub Release 与仓库版本元数据一致；
- [ ] **crate hash 对账**：crates.io 上传产物 sha256 == 状态回证账本记录的预期值；
- [ ] **异常即停**：任何一步失败 → 立即停手，开 Incident（不得继续、不得手工补发）。

## Publishing identity

- **crates.io Trusted Publishing (OIDC)**：`rust-lang/crates-io-auth-action` 交换短期身份，
  无长期 API token 存储（0.1.2 token 泄漏事故教训）。
  crates.io 侧配置：`llmrust/llmrust` · workflow `release.yml` · environment `release`
  （REL-003A 时点已配置，2026-08-03）。
- **"Trusted Publishing Only" 模式**：REL-003 发布成功验证通过后启用（API token 发布拒绝；
  先证新通道可用再关旧通道——架构师裁定 2026-08-03）。
- **本地 `cargo publish` 绝对禁止**（任何环境、任何理由）；token 零出现（`--token` /
  密钥形态全仓零命中）。

## Rollback policy

- **不手工补发**：发布失败/异常后不通过手工命令补发 crate（0.1.2 事故教训的结构化避免）。
  一律停下 → 开 Incident → 经治理流程裁定后再动作。
- 若已发布且发现严重问题：按 SPCC 事故流程处理，另行裁定（不删已发布产物）。

## Governance notes

- REL-003 由 Owner 明确授权打 tag 后方可执行（§11.8：REL-002 状态闭合后架构师向 Owner
  呈报，Owner 授权打标）。
- 本清单不构成发布授权——它只是 REL-003 执行时的核对表。
