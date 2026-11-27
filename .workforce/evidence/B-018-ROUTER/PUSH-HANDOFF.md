# B-018-ROUTER 推送 / PR 更新交接包

> 生成时间：2026-11-25（Asia/Shanghai）
> 状态：本地 `accepted`，远端 push 被 GitHub 403 阻断；**不是 CI 失败，也不是代码失败**。

## 1. 已完成的本地事实

- Workforce spec：`f068a5a470bf219776b1a4d0a95aed64cd18f86ffbd8346004135257e967dcf7`
- Batch / Card：`B-018-ROUTER` / `C-018-ROUTER-COST`
- Handoff：`f0ccaa47803ffa8b43084e96`
- 业务提交：`29f535cf8a57b5de900c30fc48d8f5d8bd55accc`
- 证据提交 / 本地 HEAD：`5807ab32bcdf4f1bb65fcc5b976c5b2f49007254`
- QA / Gate / Ledger / 五层验收：全部 PASS
- 本地测试：Router/Pricing 命名测试 5 passed；门禁 13/13 exit 0；semver 0.1.6 默认/全特性各 196 pass
- 当前账号：`dongyanyuxuana`，仓库权限 `pull=true, push=false`
- 远端分支 `task/PH-0.1.8-B-018-ROUTER`：不存在
- 现有累计 PR：[#253](https://github.com/llmrust/llmrust/pull/253)，head 分支 `task/PH-0.1.8-B-018-HTTP@18bdb3dcf5a6e8e59ab8a26592d6ca41250f2d9c`

## 2. 推荐推送方式：更新现有累计 PR #253

Router 本地提交链直接基于 HTTP 累计分支，因此不需要新开 PR。由具备写权限的维护者在本地执行：

```bash
git push origin \
  5807ab32bcdf4f1bb65fcc5b976c5b2f49007254:refs/heads/task/PH-0.1.8-B-018-HTTP
```

这是普通快进推送，**不要 force push**。推送后：

- PR #253 的 head 应从 `18bdb3d` 更新到 `5807ab3`。
- CI 与 Security Gates 会自动在 `5807ab3` 上运行。
- 维护者确认两个 workflow 的 7/7 jobs 全绿后，可合并 PR #253。
- PR #252 已被 #253 累计包含，可在 #253 合并后关闭。

## 3. 备选方式：新开 Router PR

如果维护者希望保留独立 PR：

```bash
git push origin \
  5807ab32bcdf4f1bb65fcc5b976c5b2f49007254:refs/heads/task/PH-0.1.8-B-018-ROUTER

gh pr create \
  --repo llmrust/llmrust \
  --base main \
  --head task/PH-0.1.8-B-018-ROUTER \
  --title "PH-0.1.8 B-018-ROUTER：路由冷却回退与缓存计费" \
  --body "累计包含已交付的 #252/#253 链；本地 QA/Gate/Ledger/五层验收 PASS。"
```

此 PR 会包含 #253 的全部累计提交，合并时只能选择其中一个累计 PR，避免重复合并。

## 4. 推送后的 Workforce 动作

远端双绿后，由 CI 角色：

1. 用 `gh` 拉取 `5807ab3` 的 CI / Security Gates run 与 job 明细。
2. 覆盖 `.workforce/evidence/B-018-ROUTER/ci.json`，将 `verdict` 改为 `PASS`、`conclusion` 改为 `success`，绑定新 HEAD 与真实 run URL。
3. `ci.json` 保持未提交的运行时证据。
4. Lead 执行：
   ```bash
   node .agents/skills/agent-workforce-loop/scripts/loop.mjs release --batch B-018-ROUTER --repo .
   node .agents/skills/agent-workforce-loop/scripts/loop.mjs deliver --batch B-018-ROUTER --repo .
   ```
5. 下一可开工批次应为 `B-018-STREAM`。

## 5. 不得做的事

- 不得伪造 `workflow_runs` 或使用旧 HEAD 的绿灯。
- 不得手工修改 `.workforce/loop-state.json`。
- 不得在 push 403 时把批次标成 delivered。
- 不得删除阻断回执 [ci.json](<ci.json>)；它记录了真实的权限阻断。
