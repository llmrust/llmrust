# config.json 字段说明

位置：`<workforce 目录>/config.json`（默认 `.workforce/config.json`）。模板：`assets/config.template.json`。

**用户同意的佐证**：默认 `trust.anchor: "local_review"`。执行侧按项目填好、提交，交用户本地审核。用户看过后，本人在自己的终端跑：

```bash
node "$SKILL/scripts/approve.mjs" setup  --repo "$ROOT"   # 只第一次：设审批密码，屏幕显示你的指纹，记下来
node "$SKILL/scripts/approve.mjs" config --repo "$ROOT"   # 列出规则表要点 → 输入「同意」→ 输入审批密码
```

会生成 `approver.pub.json`（你的公钥，只能验签、不能签）和 `approvals/config-<内容指纹>.json`（签了名的同意记录：哪份文件、什么内容、什么时间、谁）。私钥用你的密码加密，放在你电脑主目录 `~/.agent-workforce/`，不进仓库。执行侧把这两个文件提交即可，脚本只认签名，不认谁提交。

- 没有同意记录 → `USER_APPROVAL_MISSING`；记录被改 → `USER_APPROVAL_SIGNATURE_INVALID`；用的不是登记的那把钥匙 → `USER_APPROVAL_WRONG_APPROVER`；同意后规则表又改 → 记录对不上新内容，等于没同意。
- `spec-confirm` 锁定规则表和审批人公钥；之后改规则表 → `CONFIG_DRIFT`，换审批人 → `APPROVER_KEY_CHANGED`，一切迁移拒绝。
- `approve.mjs` 必须在真实终端运行（隐藏输入密码），agent 的非交互 shell 跑不了；agent 也不许让用户把密码发给它。
- 剩下一个口子：第一次 setup 之前，agent 抢先登记它自己的钥匙。所以 `config-check` 会显示审批指纹，用户核对一次；用户跑 `approve.mjs` 时，若仓库里登记的不是自己，它会直接报警并拒签。

需要机器核对 git 提交者时改用 `remote`（受保护分支）或 `signed`（git 签名提交），这时 `acceptors.json` 列用户邮箱，由用户本人提交。

字段、默认值、说明的完整表：`generated/config-fields.md`（生成，禁止手改）。

### 机器生成文件（generated）

```json
"generated": [
  { "path": "docs/tech-stack.md", "command": "node orchestration/tech-stack/gen.mjs" },
  { "path": ".workforce/spec.json", "command": "python tools/build_workforce_spec.py" }
]
```

`accept` 与 `release` 逐条重跑命令，把结果和仓里的文件逐字节比（换行统一为 LF），不同即 `GENERATED_HAND_EDITED`，比完恢复原文件（不替你偷偷修）。要改内容：改源头，再跑生成命令。

### 台账自洽的约定

台账 JSON 的 `self_sha256` = `sha256(JSON.stringify(去掉 self_sha256 键后的对象))`（Node 的 `JSON.stringify`，键顺序按文件里的顺序，无缩进）。生成脚本按这个算，脚本侧才能复核。

### 为什么需要信任锚

git 作者邮箱谁都能填。更隐蔽的是**首次占位**：在用户动手之前，agent 自己写一份 `acceptors.json`（填自己的邮箱）再自己提交，就成了「验收者」。仓内任何检查都分辨不出这种情况，所以必须有一个 agent 够不着的锚：远端受保护分支，或只有用户持有私钥的签名。

## 示例：React 前端 + Node 后端（原 BeeLi 配置）

```json
{
  "schema": "agent-workforce-config/v1",
  "business_dirs": ["web/src/", "host/src/", "contract/"],
  "architect_required_inputs": ["AGENTS.md", "orchestration/tech-stack/spec.json"],
  "gates": { "required_commands": ["verify-ui-slice"], "require_no_mock": true },
  "rule_id_pattern": "^[A-Z]+-[A-Z]-\\d+$",
  "release": { "overall_file": "docs/ui-ledger.json", "overall_pointer": "/summary/overall" },
  "ci": { "provider": "github" },
  "frontend": { "dirs": ["web/"], "web_guidance": true },
  "ledger": { "command": "node scripts/gen-ui-ledger.mjs" },
  "generated": [{ "path": "docs/tech-stack.md", "command": "node orchestration/tech-stack/gen.mjs" }],
  "trust": { "anchor": "local_review" }
}
```

沿用旧目录时，每条命令带 `--dir orchestration/workforce`，或设 `WORKFORCE_DIR=orchestration/workforce`。

## 示例：Python 库（无 UI）

```json
{
  "schema": "agent-workforce-config/v1",
  "business_dirs": ["lib/"],
  "roles": { "design": false },
  "gates": { "required_commands": ["pytest", "ruff check"] },
  "ci": { "provider": "github", "repo": "your-org/your-lib" },
  "ledger": { "none_reason": "库项目无台账生成器" },
  "trust": { "anchor": "signed", "allowed_signers": "../.ssh/allowed_signers" }
}
```

## CI 是强制项（没有后门）

- `ci.provider` **只能是 `github`**。`command`（只在本地跑一条命令）与 `none` 都是绕开 CI，config 校验直接判 `CONFIG_CI_BYPASS_FORBIDDEN`，**任何迁移都跑不起来**。
- `ci.repo` 必填（`owner/name`）：CI 跑在哪个远端仓库。
- `release` 前必须同时满足：
  1. 存在 **ci 工种回执** `evidence/<batch>/ci.json`，且 `role:"ci"`、`verdict:"PASS"`、`provider:"github"`；
  2. 回执里 `pushed.head_sha` **等于当前 HEAD**；
  3. 当前 HEAD **真的在某个远端分支上**（`git branch -r --contains <head>` 非空）——没推过的提交，远端 CI 必然没跑过，直接 `CI_NOT_PUSHED`；
  4. `gh run list --repo <ci.repo> --commit <head>` 拉到的运行**全部** `conclusion=success`；
  5. 任一不满足都 `RELEASE_BLOCKED`，没有开关能跳过。
- 规矩：**改完就 push 分支 + 开 PR，等远端 CI 跑完再谈发布。**绕过 CI 视为违规。
