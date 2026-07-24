# API-001 — 公开 API 双基线清单与差异裁定 (Design / Inventory)

- **Issue**: #100
- **Branch**: `task/API-001-public-api-design`
- **Baseline HEAD**: `ceccd9b`
- **Status**: 纯观测任务，**零 `src/**` 改动**。本文件是 M1 API 冻结的测量基线 SSOT。
- **Machine artifact**: `docs/api-inventory.json` (schema `llmrust-api-inventory/1.0`)，供 API-002 消费。

---

## 0. 红线与基线获取（方法学）

按 SPCC §868 任务卡与 Issue #100 要求，取**三个**基线，三方对照：

| Baseline | 来源 | SHA-256（证据） | 说明 |
|---|---|---|---|
| v0.1.1 干净源码 | `git archive v0.1.1` | `0B6BA5304CF67E517F03C7866E0022988D71DA857F87A41B50999548D3AA90B0` | git tag，干净 |
| v0.1.2 发布物 | crates.io `.crate` (static.crates.io) | `1DFB0E25B02AF20AD562FDF4C8C4492B71BDB37D9A66B6DC181A951B4879C481` | dirty/yanked 发布；含 `.cargo_vcs_info.json` + `publish.log`（152034 字节），证明是**真实发布物**，非本地工作树 |
| current main | `git rev ceccd9b` | — | API-001 基线 HEAD |

**红线（§868 / Issue #100）**：v0.1.2 基线**只**取自 crates.io 发布的 `.crate`，严禁用本地 0.1.2 历史工作树冒充。上方证据（`.cargo_vcs_info.json`、`publish.log`、152034 字节 crate）确认用的是真实发布物。

---

## 1. 公开 API 面构成

公开 API = 以下并集：
- **(A)** 根再导出（`lib.rs` `pub use`，行 79–88）— 24 个类型
- **(B)** `pub mod types` 可达但未根再导出的类型 — `ThinkingConfig`
- **(C)** `LmrsClient` facade（23 个 `pub` 方法，`lib.rs:110–516`）
- **(D)** `Provider` trait（`chat`/`stream`/`embed`，`providers/mod.rs`）+ `RetryProvider`
- **(E)** 再导出的 `Router` / `RoutingStrategy` / `ModelPricing`
- **(F)** `proxy` 模块 DTO（feature 门控；`proxy/mod.rs` 20 个 + `anthropic_proxy.rs` 14 个 + `AppState`）

---

## 2. 三方事实表（存在性 + 分类）

每行带 `file:line` + `non_exhaustive` + 分类 + 理由的完整数据见 `docs/api-inventory.json`。汇总：

| Symbol | kind | module | root-reexported | non_exhaustive | v0.1.1 | v0.1.2 | main | classification |
|---|---|---|---|---|---|---|---|---|
| 24 个根类型 | struct/enum | types | yes | 见 JSON | ✓ | ✓ | ✓ | STABLE / STABLE-ADDITIVE |
| `ThinkingConfig` | enum | types | **no** | **no** | ✗ | ✓ | ✓ | STABLE（建议）— 已标红 |
| `LmrsClient` | facade | lib | n/a | n/a | ✓ | ✓ | ✓ | STABLE |
| `Provider` trait | trait | providers | yes | n/a | ✓ | ✓ | ✓ | STABLE |
| `RetryProvider` | struct | providers::retry | yes | n/a | ✓ | ✓ | ✓ | STABLE |
| `Router`/`RoutingStrategy`/`ModelPricing` | — | router/pricing | yes | n/a | ✓ | ✓ | ✓ | STABLE |
| proxy DTOs（35） | struct/enum | proxy | yes(feat) | no | ✓ | ✓ | ✓ | UNSTABLE（建议） |

---

## 3. 差异分析

### 3.1 v0.1.1 → v0.1.2（"继承"的变更）
- **新增** `ThinkingConfig` enum — `src/types.rs:556`（0.1.1 无）
- **新增** `ChatRequest.thinking: Option<ThinkingConfig>` 字段 — `src/types.rs:652`
- **新增** `ChatRequest::with_thinking(...)` builder — `src/types.rs:740`
- 无任何公开符号的删除 / 重命名 / 签名变更。
- Semver 影响：**非破坏性**。`ChatRequest` 为 `#[non_exhaustive]`（加字段允许）；加方法非破坏性；`ThinkingConfig` 为新公开 enum（加法）。
- 其余（根再导出、LmrsClient、Provider、proxy DTOs）在 0.1.1 与 0.1.2 间逐字节相同。

### 3.2 v0.1.2 → current main（"新增"的变更）
- **空集。** 每个公开面逐字节相同（已核验：types.rs 行号 556/592/833 一致；lib.rs 23 方法一致；proxy 38 处匹配一致）。
- ⇒ DoD「相对 0.1.2 的 allowed-change-set 为空」**满足**。

### 3.3 解读
0.1.2 带入 main 的唯一内容是加法的 `ThinkingConfig` 引入。**main 相对 0.1.2 无新增破坏性变更**。但因 0.1.2 是 dirty/yanked、从未正式批准发布，架构师须裁定（§5）是否将 `ThinkingConfig` + `ChatRequest.thinking` 采纳为冻结的 0.1.x 基线，或回退。

---

## 4. 分类方案（依 Issue #100 裁定）

- **STABLE**：0.1.x 内形状冻结，无破坏性变更。
- **STABLE-ADDITIVE**：`#[non_exhaustive]`；现有字段/变体冻结，允许新增可选项。
- **INTERNAL-PUB**：仅因可见性而 `pub`，不在 0.1.x semver 承诺内。
- **UNSTABLE**：0.1.x 内可能变更（如 proxy wire DTO）。

**每行硬要求**：来源 `file:line` + `non_exhaustive` 标志 + 一行理由（见 JSON）。

---

## 5. 漂移发现（需架构师逐项裁定）

**D1（最高优先，Issue #100 指定首条）`FinishReason`** — `src/types.rs:404`，`pub enum`，**非** `#[non_exhaustive]`。
- AGENTS.md 声称：*"FinishReason variants are cross-provider"*（暗示可扩展）。
- 代码：非 `#[non_exhaustive]` ⇒ 0.1.x 内加变体 = **破坏性**变更。
- 处置待架构师：改文档（删"可扩展"声称）vs 改代码（加 `#[non_exhaustive]`）。路由：`E-*`（代码）或 `SPEC-*`（规范）——架构师另给建议。

**D2：`ChatResponse`** — `src/types.rs:473`，`pub struct`，非 `#[non_exhaustive]`。
- 响应体未冻结；加字段会破坏 0.1.x 消费者。与 `ChatRequest`（non_exhaustive）不一致。建议：标 `#[non_exhaustive]`。

**D3：`ThinkingConfig`** — `src/types.rs:556`，`pub enum`，非 `#[non_exhaustive]`，未根再导出。
- 两处不一致：(a) 不在根 `pub use`（所有同侪类型都在）；(b) 作为可扩展 enum 却非 `#[non_exhaustive]`。建议：根再导出 + 加 `#[non_exhaustive]`。

**D4：`ThinkingConfig` 未文档化** — AGENTS.md / docs/CAPABILITIES.md 对 `thinking`/`ThinkingConfig` 只字未提，尽管其自 0.1.2 起已公开。文档缺口。

**D5：AGENTS.md 声称准确性** — "ChatRequest is #[non_exhaustive]" ✓；"EmbeddingRequest fields are cross-provider; provider-specific knobs belong in `extra`" ✓（EmbeddingRequest 确为 non_exhaustive）。仅 D1（`FinishReason`）为真实矛盾。

---

## 6. Proxy DTO 稳定性（显式裁定请求）

proxy DTO 为 wire-facing（`proxy/mod.rs`）：`ProxyChatRequest`、`ProxyMessage`、`ProxyChatResponse`、`ProxyStreamChunk`、`ProxyEmbeddingRequest/Response`、`ProxyError*`、加服务器内部的 `AppState`，以及 `anthropic_proxy.rs` 请求/响应 DTO。
- 三个基线间**逐字节相同**（无漂移）。
- 建议分类 **UNSTABLE**：proxy 是 HTTP 兼容垫片，wire 格式可能在 0.1.x 内演进；消费者不应依赖 DTO 精确形状。`AppState` 为 INTERNAL-PUB（服务器状态，非 wire 契约）。
- 请架构师确认 UNSTABLE vs STABLE。

---

## 7. DoD 自检

- [x] 覆盖 default + `proxy` feature 公开面。
- [x] 每个差异可追溯到基线制品内的 `file:line`。
- [x] 相对 0.1.2 的 allowed-change-set 为空（核验逐字节相同）。
- [x] 0.1.2 基线取自 crates.io，非本地树（红线遵守，checksum 在案）。
- [x] 纯观测：未修改任何 `src/**`。
- [ ] 架构师对差异表逐项裁定（交付后，依 Issue #100）。

---

## 8. 机器产物

`docs/api-inventory.json` — schema `llmrust-api-inventory/1.0`，由 API-002（semver 机器化）消费。
