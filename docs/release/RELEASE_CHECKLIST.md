# Release Checklist

Operator checklists, newest first. Each release's checklist is the companion to **its own release commit**
and is **kept verbatim as history** after execution (不得回改已执行过的清单)。

> **预期 crate hash 见实现 PR body 与状态回证账本（§11.1.4）** —— 本清单不内嵌 hash 值
> （写入即改变包内容，hash 立即失效，防自引用悖论；MUST-1(c)）。

---

## `REL-005` execution checklist（**0.1.4**，待 Owner 授权后执行）

- [ ] **Owner 授权**：`REL-005` 的前置为「`REL-004` DONE **+ Owner 授权**」——**无授权不打 tag**；
- [ ] **版本四处一致**：`Cargo.toml` `0.1.4` / `llmrust.capabilities.json` `0.1.4` /
      `llmrust.models.json` `0.1.4` / `CHANGELOG.md` `## [0.1.4] - 2026-09-12` / `docs/COMPATIBILITY-0.1.4.md` 版本与日期；
- [ ] **三证一致**：`git rev-parse main` == `REL-004` 的 merge SHA == 待打 tag 指向的 commit；
- [ ] **CI 全绿**：主干 head 的七项 check runs 全 `success`（semver / Arch guards / MSRV / Test / cargo-deny / gitleaks / RustSec）；
- [ ] **tag 形态**：`v0.1.4`（**打 tag 是唯一触发 `release.yml` 的动作**；该 workflow 无 `workflow_dispatch`）；
- [ ] **publish job 观察**：`rust-lang/crates-io-auth-action`（OIDC 短期身份）交换 → `cargo publish`（**无 `--token`**）；
      **`environment: release` 需 Owner 在 GitHub 上批准**；
- [ ] **三方验证**：crates.io 页面 / docs.rs / GitHub Release 与仓库版本元数据一致；`yanked=false`；
- [ ] **crate hash 对账**：crates.io 上传产物的 `sha256` == §11.1.4 账本记录的预期值；
- [ ] **发布说明核对**：`docs/COMPATIBILITY-0.1.4.md` §3 的四条披露**必须**出现在 GitHub Release 说明中
      （无 `live-endpoint` 证据 / `CAP-005` 省钱未实测 / 9 项门禁顺延 0.1.5 / 执行侧自身的假门）；
- [ ] **异常即停**：任何一步失败 → 立即停手，开 Incident（**不得继续、不得手工补发**）。

---

## REL-003 execution checklist（**0.1.3**，已执行 · 保留为历史）

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
