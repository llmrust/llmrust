# `INC-003` — 0.1.4 发布后核验步骤误报失败（上传本身成功）

- **发生时间**：2026-09-12 10:17–10:23 UTC
- **客体**：`v0.1.4` 发布（`REL-005`）
- **流水线 run**：`34687710549`（workflow `release.yml`）
- **严重度**：**低（发布结果无损）**，但**信号被污染**：成功的发布被标成 failure
- **处置状态**：已修（轮询步）+ 已留痕（本文件）+ 已回证（SPCC §11.1.2/§11.1.3/§11.1.4）

---

## 1. 事实（全部机器取证，无推断）

| 项 | 实测 | 证据 |
|---|---|---|
| **上传是否成功** | **成功** | `crates.io/api/v1/crates/llmrust/0.1.4` → `crate_size=514567`、`created=2026-09-12T10:17:09Z`、`yanked=false`、`license=MIT OR Apache-2.0` |
| **docs.rs** | **已生成** | `https://docs.rs/llmrust/0.1.4/llmrust/` 返回 39 KB 且含 `llmrust` |
| **tag** | 正确 | `v0.1.4`（annotated）→ `4ee2b67`；与 `origin/main` 一致；该 SHA 的 8/8 check runs 全 success |
| **失败在哪一步** | **仅"发布后核验"** | run 内四个 job：`Pre-flight validation` ✅、`fmt/clippy gate` ✅、`Package + dry-run` ✅、**`Publish to crates.io` ❌**；其失败步骤为 `Verify crate visible on crates.io (bounded poll)`，其余步骤（含 `cargo publish`）成功 |
| **失败原话** | — | `##[error]crate 0.1.4 not visible on crates.io after 300s`（10:18:10 起轮询 → 10:23:11 放弃，恰 300 秒） |

**⇒ 结论：发布成功，误报来自核验步骤。**

## 2. 根因

核验步骤（`release.yml`）原文：

```bash
for i in $(seq 1 30); do
  STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
    "https://crates.io/api/v1/crates/llmrust/${VER}" || true)
  [ "$STATUS" = "200" ] && { … exit 0; }
  sleep 10
done
echo "::error::crate ${VER} not visible on crates.io after 300s"
```

| # | 缺陷 | 性质 |
|---|---|---|
| 1 | **`curl` 未发送 `User-Agent`** —— crates.io 的 API **要求** UA，缺失时返回非 200 | **本次最可能的直接原因**（带 UA 的同一端点，事后查询立即 200） |
| 2 | 预算 **30×10s = 300s** | 与 `REL-005` 执行步骤③「改为 60×10s」不符，**本卡本该修而未修** |
| 3 | 失败时**不打印状态码** | 无法区分 403（UA 被拒）/ 404（未发布）/ 其他 ⇒ 诊断信息为零 |

## 3. 为什么没在推 tag 前拦住

该项**早已在册**：`docs/SPCC-0.1.4.md` §14 发现→任务映射表

> `发布可见性轮询 300s`｜`0.1.3 结转`｜P3｜`R-B`｜处置 `GRD-003` / **`REL-005`**｜**待开工**

即：**这是一个被登记、被指派、但未被执行的项**；而 `REL-005` 的执行步骤③ 正是修它。
**它第一次表现出来，就是 0.1.4 的真发布。** —— 这是"已登记的债不还，就会在最贵的时候还"的实例。

## 4. 处置

| 动作 | 状态 |
|---|---|
| 修核验步：`60×10s` + 带 `User-Agent` + **失败时打印状态码** | **已修**（本 Incident 的配套 PR） |
| **不手工补发** | 遵守 `RELEASE_CHECKLIST`「异常即停…不得手工补发」；0.1.4 已在册，任何重发都会失败且破坏痕迹 |
| **不重跑该 run** | 同上（会重复进入 publish 路径） |
| 状态回证 | SPCC §11.1.2 `N5` → `3/3 DONE`；§11.1.3 `REL-005` → `DONE`；§11.1.4 补回证行 |

## 5. 教训（可被后续卡复用的）

1. **"发布后核验"必须有诊断信息**：只说"不可见"而不说状态码，等于把失败变成不可查；
2. **对外部 API 的调用一律带 `User-Agent`**（crates.io 尤其）；
3. **已登记的 P3 债在发版窗口前要清**：本项登记的处置任务就是本卡，**"待开工"三个字在发版日等价于"必炸"**；
4. 失败步在 `cargo publish` **之后** ⇒ 判 failure 时**必须区分"上传失败"与"核验失败"**，否则会把成功的发布当成事故（本次即如此）。
