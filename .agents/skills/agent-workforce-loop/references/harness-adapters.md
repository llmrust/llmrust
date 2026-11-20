# 在不同 agent harness 里使用

脚本只依赖 Node 与 git，判据不随 harness 变。harness 之间不同的只有三件事：skill 放哪、怎么开独立 agent 会话、怎么限制每个工种的工具。

## 1. 安装位置（各家文档，2026-10 核对）

| Harness | 项目级 | 用户级 |
|---|---|---|
| Codex | `.agents/skills/` | `~/.agents/skills/` |
| Gemini CLI | `.gemini/skills/` 或 `.agents/skills/` | `~/.gemini/skills/` 或 `~/.agents/skills/` |
| GitHub Copilot（CLI / cloud agent / VS Code） | `.github/skills/`、`.agents/skills/`、`.claude/skills/` | `~/.copilot/skills/`、`~/.agents/skills/` |
| Cursor | `.agents/skills/`、`.cursor/skills/`（也读 `.claude/skills/`） | `~/.agents/skills/`、`~/.cursor/skills/` |
| OpenCode | `.agents/skills/` | 见其文档 |
| Claude Code | `.claude/skills/`（**不读 `.agents/skills/`**） | `~/.claude/skills/` |

建议放 `.agents/skills/agent-workforce-loop/` 随仓库提交；用 Claude Code 时再建一个指向它的 `.claude/skills/agent-workforce-loop` 软链接（Windows 用目录联接 `mklink /J`）。目录名必须是 `agent-workforce-loop`。

## 2. 独立 agent 会话

`dispatch` 输出里每张卡的 `wake[].brief` 和每个批次工种的 `batch_agents[].brief` 是纯文本任务说明。规则只有一条：**每个 brief 交给一个独立会话，会话名 = brief 里的 agent 名**（如 `dev-c001`、`qa-c001`），会话之间不共享上下文。

| Harness | 做法 |
|---|---|
| 有子 agent 的 harness（如 Claude Code 的 subagent、Copilot / VS Code 的自定义 agent） | 为 architect / dev / qa / gate / ledger / design 各定义一个子 agent，Lead 在一条消息里把 brief 分别交给它们 |
| 只有单会话 CLI（如 `codex exec`、`gemini -p`、`opencode run`） | 每个 brief 起一个独立进程，最好各自一个 `git worktree`，跑完把回执提交回主分支 |
| IDE 内 agent（如 Cursor） | 每个工种开一个新的 agent 标签页 / 会话，粘贴 brief |
| 人工 | brief 也可以直接发给人类同事，回执格式不变 |

Lead 自己所在的会话只跑 `loop.mjs` 命令、写 `signoffs/lead.json`、做验收，不接 brief。

## 3. 工具限制（可选，越严越好）

`allowed-tools` 不是所有 harness 都认，所以本 skill 不写在 frontmatter 里。在支持的 harness 里按工种收紧：

| 工种 | 应允许 | 应禁止 |
|---|---|---|
| Lead | 读文件、`node <skill>/scripts/*`、`git status/log/diff`、`gh run list` | 写业务目录 |
| dev | 读写本卡 write_scopes、跑测试；前端卡加 `npx -y modern-web-guidance@latest *` | 写 signoffs、他人回执 |
| qa | 读代码、写测试目录、跑测试 | 读 dev 回执里的 notes |
| gate / ledger | 跑 required_commands 与台账脚本 | 改判据脚本 |

## 4. git 身份

每个 agent 会话用自己的提交身份（如 `dev-c001@agents.local`），**不得**使用 acceptors.json 里的邮箱。用户确认、config、issue 由用户本人提交。远端开启分支保护（agent 凭据推不上 `trust.branch`）或让用户签名提交，对应 `config.trust.anchor` 的 `remote` / `signed`；否则 agent 可伪造作者邮箱或抢先自写 acceptors.json。
