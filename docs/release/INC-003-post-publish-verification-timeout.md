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
| 1 | **`curl` 未发送 `User-Agent`** —— crates.io 的 API **要求** UA，缺失时返回 **403** | **真因（已复现，非推断）**：审核官 2026-09-12 亲跑 3/3 → 不带描述性 UA = **403**，带 UA = **200** |
| 2 | 预算 **30×10s = 300s** | **不是本次事故之因**（crate `10:17:09` 已发布，轮询窗口 `10:18:10→10:23:11` 内**早已可见**，只是每次都被 403）。改 600s 是**偿还 `REL-005` 步骤③ 的规格欠账**，非止血 |
| 3 | 失败时**不打印状态码** | 无法区分 403（UA 被拒）/ 404（未发布）/ 其他 ⇒ 诊断信息为零 |

## 3. 为什么没在推 tag 前拦住

该项**早已在册**：`docs/SPCC-0.1.4.md` §14 发现→任务映射表

> `发布可见性轮询 300s`｜`0.1.3 结转`｜P3｜`R-B`｜处置 `GRD-003` / **`REL-005`**｜**待开工**

即：**这是一个被登记、被指派、但未被执行的项**；而 `REL-005` 的执行步骤③ 正是修它。
**它并不是第一次表现**：`0.1.3` 的发布 run `30801472635` 结论**同样是 `failure`**（同因），SPCC-0.1.3 档案原文即已记为"列 0.1.4 改进"。⇒ **这笔债在 0.1.3 就已经发作过一次，0.1.4 是第二次**；"已登记的债不还，就会在最贵的时候还"这个教训成立，**但发作时点更早**。

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

## 6. 本起之外、但同批发现的后续项（登记，不在本 PR 修）

1. **`release.yml` 不创建 GitHub Release**（`grep` 无 `gh release`/`softprops`）⇒ `v0.1.3` 与 `v0.1.4` 的 Release **均为人工创建**。
   本次已按清单要求人工补建 `v0.1.4`（含四条披露，置 `Latest`）；**是否把 Release 创建纳入流水线，另立卡裁定**。
2. **文档口径不一**：SPCC §11.8/§11.1.2 的 N5 出口写"GitHub **tag** 三方一致"，`RELEASE_CHECKLIST` 写"GitHub **Release**"。
   本次以**更严的清单口径**执行（tag + Release 双满足）；建议后续统一措辞。