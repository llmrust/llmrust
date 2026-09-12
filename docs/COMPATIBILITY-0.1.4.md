# Compatibility & Upgrade Notes — llmrust 0.1.4

**一句话**：**0.1.4 不要求源码迁移。** 已按 0.1.2 基线跑过 `cargo-semver-checks`（CI 的
`API-002 semver gate`，每次 PR 与 main 推送均通过），未发现破坏性变更。

---

## 1. 为什么有 0.1.4

0.1.3 之后的主题是**"让门禁可信 + 让能力与成本说真话 + 清掉已知静默失败"**：

| 里程碑 | 内容 | 状态 |
|---|---|---|
| `N0` 门禁完整性 | 双向化、fail-closed、扫描器加固、治理文档行数地板 | **4/4 DONE** |
| `N1` 能力真相 | 能力声明进入代码（`Capabilities`）、统一入口裁决、三处一致门、缓存断点与价格目录 | **6/6 DONE** |
| `N2` 静默失败 | 工具参数静默改写、错误分类失实、终结原因失真、忽略 `Retry-After`、代理认证 trim 不一致 | **5/5 DONE** |
| `N3` 验证深度 | 真端点证据（`live-endpoint`） | **移出本版 → 0.1.5** |
| `N4` 治理伸缩 | 状态区小文件化、一致性 CI 化 | **移出本版 → 0.1.5** |

**范围收缩是 Owner 于 2026-09-11 的决定（读法 B）**：主项目 `yesagent` 有交期，
`N0`+`N1`+`N2` 已足以用上新能力且有门保护。见 `docs/SPCC-0.1.4.md` §11.1.2 决策记录。

---

## 2. 行为变更清单（**升级前请读**）

### 2.1 wire 可见的行为变更

| 变更 | 之前 | 现在 | 触发条件 |
|---|---|---|---|
| 代理拒收 `cache` 键 | **200 成功、断点一个没设**（serde 静默忽略） | **400**，错误体指明用库内 API 设置 `ChatRequest.cache` | 客户端向代理发 `cache` |
| Anthropic `stop_reason`：未知上游原因 | **原样回显上游串**（含 Gemini `FINISH_REASON_UNSPECIFIED`；实测任意串都会被回显） | **`null`**（+ 截断的 `tracing::warn` 痕迹） | 上游给出未映射的终结原因 |
| Anthropic `stop_reason`：内容过滤 | `"content_filter"`（**非** Anthropic 取值） | **`"refusal"`**（Anthropic 真值） | 上游返回内容过滤 |
| `with_retry()` 下的能力裁决 | 装饰器不转发声明 ⇒ 判为"未声明" ⇒ **只 warn、不 Reject**（`tools` 会被发往上游） | **转发** ⇒ 对已声明 `unsupported` 的能力**响亮拒绝** | 使用 `LlmrsClient::with_retry()` |
| 代理认证：配置键带前后空白 | **任何客户端都 401**（校验 trim、存储不 trim） | **规范化（trim）后生效** | 部署配置键带空白 |

### 2.2 新增公开能力（纯附加）

- `Capabilities` / `Capability` / `CapabilityLevel` / `EvidenceKind` / `Verified` / `Verdict` /
  `RequestDemands`：能力**声明**与**裁决**（`Provider::capabilities()` 有默认实现，
  **下游最小 Provider 不实现它仍可编译**）；
- `Provider::protocol_name()`、`Provider::last_retry_after()`（均带默认实现）；
- `ChatRequest.cache` + `CachePolicy`/`CacheRetention`：**在稳定前缀末尾打缓存断点**（Anthropic 系）；
- `CachePricing` + `ModelPricing::estimate_cost_with_cache`：缓存读写分档计价；
- `catalog`：官方服务商/接入路径目录；`capability_table`：机读能力表 + 由表生成 `CAPABILITIES.md`。

### 2.3 行为修复（无 wire 形状变更）

- 工具参数解析失败**不再静默替换成 `{}`**（改为留痕降级，痕迹不含参数原文）；
- HTTP 客户端构建失败**不再静默丢弃全部连接配置**；
- 错误分类不再硬编码 `api_error`（9 类映射，限流/连接/5xx/未注册可区分）；
- 代理错误体 ≤200 字符上界**对全部路径生效**（此前 `UnknownProvider`/`Unsupported` 可无界）；
- 上游 `Retry-After` **被消费**（两种格式；上限 60 秒；畸形值回落本地退避，不猜、不 panic）；
- `Retry-After` 的 HTTP-date 年份越界**不再 panic**（4 位年边界 + 饱和算术）。

---

## 3. 本版**未**包含什么（**如实披露，不得省略**）

1. **无 `live-endpoint` 证据**：本版所有 `verified` 格子只到 `local-fixture`。
   **`CAP-005` 只证明"缓存断点发得出"，未证明真端点 `cached_tokens > 0` ⇒「省钱是否为正」未被实测。**
   （依据规格 §1.4：真实端点证据到位前，任何"修好了"的结论仍然是自证。）
2. **门禁真实性有已知缺口**：外部审计点名的 **9 项门禁问题未修**，经裁定整块顺延 0.1.5
   （其中一项是**设计决定**：`CAPABILITIES.md` 矩阵行 ↔ `llmrust.models.json` face 的逐行定性）。
   **这不影响本版 API 可用性**，但意味着"将来改坏东西时门会不会叫"在本版**未被完全保证**。
3. **执行侧自己的假门（已披露）**：`ERR-003` 曾报"含负例"，实则该测试把结果**自己包成 `Some(..)`**
   ⇒ **断言恒真**。已在审计中更正；未修的同类项计入上面第 2 条。
4. `ERR-004` 的 `Retry-After` 消费**未覆盖** Anthropic / Gemini / Ollama 三条独立错误构造路径
   （与 0.1.3 行为一致；已在 `docs/CONTRACTS.md` 记录）。
5. **热点巨石未拆且本版净增**（`proxy/mod.rs`、`router.rs`、`google.rs` 等）；
   拆分属 0.2.0（§1.6 明文排除）。

---

## 4. 升级指引

- **不需要改代码**：直接在 `Cargo.toml` 把 `llmrust` 提到 `0.1.4`（若使用 `yesagent`，
  注意其 pin 由 `=0.1.3` 改为 `=0.1.4` 是**另一个仓库的独立 PR**）；
- **若你依赖 `with_retry()`**：本版起，被明确声明为 `unsupported` 的能力会**被拒绝**而不是静默放行——
  这是**恢复** 0.1.3 既有的拒绝承诺，若你的代码此前依赖"发出去上游自己忽略"，请改为不发送该能力；
- **若你向代理发 `cache`**：本版起会得到 **400**（此前是"200 但无效"）。请在库内设置
  `ChatRequest.cache`，或不要发送该键；
- **若你解析 Anthropic 代理的 `stop_reason`**：请容忍 **`null`**（未知原因）与 **`refusal`**。

---

## 5. 验证基线（本版）

| 项 | 值 |
|---|---|
| 全量测试 | **19 组 / 580 passed / 0 failed**（`cargo test --all-features`） |
| 格式 / Lint | `cargo fmt --all --check` 通过；`cargo clippy --all-targets --all-features -- -D warnings` **零告警** |
| semver | `API-002 semver gate`（对 0.1.2 基线）通过 |
| 供应链 | `cargo-deny` / RustSec / gitleaks 通过 |
| 审计 | `docs/release/RC-0.1.4-AUDIT.md`（`RC-002`，结论 `GO`） |

---

## 6. 勘误与致谢

- 本版发布前经**外部独立审计**，其发现（含执行侧自身的假门、失实数字、目标替换）
  已逐条记录在 `docs/release/RC-0.1.4-AUDIT.md` 与相关 PR 的更正评论中；
- **commit message 中的历史数字失实不改写历史**，以 PR 更正评论留档（`#225`/`#231`/`#232`）。
