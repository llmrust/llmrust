#!/usr/bin/env node
/**
 * 合并 PR 的**唯一入口**（INC-011 / INC-012 的守卫：合并动作不得绕过 premerge 门）。
 *
 * 用法：
 *   node scripts/merge-pr.mjs --batch <id> --pr <n> --repo <owner/name> --method squash \
 *        [--root <repo 根>] [--dry-run] [--allow-bootstrap]
 *
 * 行为（顺序固定，不可绕过）：
 *   0. **引导自检（INC-012）**：校验目标 worktree（--root，默认 cwd）里的
 *      `.agents/skills/agent-workforce-loop/scripts/merge-pr.mjs` 是否存在且与本脚本 sha256 一致。
 *      找不到/版本落后 ⇒ stderr 打印
 *      `GATE_BOOTSTRAP_MODE：当前 worktree 无 merge-pr.mjs，本合并未经门校验`，
 *      且**必须显式 `--allow-bootstrap`** 才继续；否则 exit 1（GATE_BOOTSTRAP_REQUIRED）。
 *   1. 读 loop-state + 目标批次，跑 premergeCheck()；
 *      FAIL ⇒ {"verdict":"REJECT","reason":"MERGE_PR_REFUSED",...,"merge_called":false} 并 exit 1，
 *      **不调用任何合并 API**（合并调用点在 FAIL 分支之后，物理不可达）。
 *   2. PASS 且 --dry-run ⇒ would_merge=true / merge_called=false（供回归断言）。
 *   3. PASS 且非 dry-run ⇒ `gh api -X PUT repos/<repo>/pulls/<n>/merge -f merge_method=… -f sha=<HEAD>`；
 *      合并后打印 merge_commit + 逐 job 结论，并**自动写 evidence/<batch>/merge.json**：
 *      {at, batch, pr, merge_commit, merge_called:true, head_sha, checks:[…], bootstrap_used}。
 */
import { execFileSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import * as L from './lib.mjs';

const SELF = fileURLToPath(import.meta.url);

const { flags } = L.parseArgs(process.argv.slice(2));
const batchId = typeof flags.batch === 'string' ? flags.batch : '';
const pr = typeof flags.pr === 'string' ? flags.pr : '';
const repoSlug = typeof flags.repo === 'string' ? flags.repo : '';
const method = typeof flags.method === 'string' ? flags.method : 'squash';
const dryRun = flags['dry-run'] === true;
const allowBootstrap = flags['allow-bootstrap'] === true;
const root = resolve(typeof flags.root === 'string' ? flags.root : process.cwd());
let bootstrapUsed = false;
const fail = (reason, extra = {}) => { console.log(JSON.stringify({ verdict: 'REJECT', reason, batch: batchId, pr, merge_called: false, bootstrap_used: bootstrapUsed, ...extra }, null, 2)); process.exit(1); };

if (!batchId) fail('GATE_BOOTSTRAP_REQUIRED', { detail: '缺少 --batch <id>' });
if (!pr) fail('GATE_BOOTSTRAP_REQUIRED', { detail: '缺少 --pr <n>' });
if (!/^[\w.-]+\/[\w.-]+$/.test(repoSlug)) fail('GATE_BOOTSTRAP_REQUIRED', { detail: '缺少/非法 --repo <owner/name>' });
if (!['merge', 'squash', 'rebase'].includes(method)) fail('GATE_BOOTSTRAP_REQUIRED', { detail: '非法 --method ' + method });

// ---- 0. 引导自检（INC-012）：目标 worktree 是否真的带着本门 ----
const targetGate = join(root, '.agents/skills/agent-workforce-loop/scripts/merge-pr.mjs');
let bootstrapWhy = '';
try {
  if (!existsSync(targetGate)) bootstrapWhy = '目标 worktree 缺 merge-pr.mjs';
  else if (L.shaFile(SELF) !== L.shaFile(targetGate)) bootstrapWhy = '目标 worktree 的 merge-pr.mjs 与本脚本版本不一致';
} catch { bootstrapWhy = '引导自检异常（无法读目标 merge-pr.mjs）'; }
if (bootstrapWhy) {
  bootstrapUsed = true;
  process.stderr.write('GATE_BOOTSTRAP_MODE：当前 worktree 无 merge-pr.mjs，本合并未经门校验\n');
  process.stderr.write('  原因：' + bootstrapWhy + '（目标 ' + targetGate + '）\n');
  if (!allowBootstrap) fail('GATE_BOOTSTRAP_REQUIRED', { bootstrap_reason: bootstrapWhy, hint: '门自身首次合并：确需引导合并时显式加 --allow-bootstrap（会留痕 bootstrap_used=true 并写入 merge.json）' });
}

const P = L.paths(root, {});
const s = L.readState(P);
if (!s) fail('MERGE_PR_REFUSED', { detail: 'loop-state 不存在：' + P.state });
const b = (s.batches || []).find(x => x.id === batchId);
if (!b) fail('MERGE_PR_REFUSED', { detail: 'BATCH_UNKNOWN ' + batchId });

// ---- 1. 门：premergeCheck（只读） ----
const r = L.premergeCheck(P, s, b);
if (!r.ok) fail('MERGE_PR_REFUSED', { premerge: r, hint: '合并被拒：先修 premerge 的 reasons（批次状态 / ci 回执绑当前 HEAD / 该 head 真 PR 通道全绿）；FAIL 分支不会调用任何合并 API' });
const head = r.head;

// ---- 2. dry-run：证明「PASS 也还没合并」 ----
if (dryRun) {
  console.log(JSON.stringify({ verdict: 'PASS', dry_run: true, merge_called: false, would_merge: true, bootstrap_used: bootstrapUsed, batch: b.id, pr: Number(pr), head, premerge: r, next: '去掉 --dry-run 才会调用 gh api -X PUT .../merge' }, null, 2));
  process.exit(0);
}

// ---- 3. 只有到这里才可能调用合并 API ----
const gh = (args) => execFileSync('gh', args, { cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim();
let merge = {};
const mergeCalled = true;
try {
  const raw = gh(['api', '-X', 'PUT', `repos/${repoSlug}/pulls/${pr}/merge`, '-f', 'merge_method=' + method, '-f', 'sha=' + head]);
  merge = JSON.parse(raw);
} catch (e) {
  console.log(JSON.stringify({ verdict: 'FAIL', reason: 'MERGE_PR_API_FAILED', batch: b.id, pr: Number(pr), merge_called: mergeCalled, bootstrap_used: bootstrapUsed, stderr: String((e.stderr || e.message || e)).slice(0, 800) }, null, 2));
  process.exit(1);
}
// ---- 合并后：逐 job 结论 + 机器记录 evidence/<batch>/merge.json ----
let jobs = [];
try {
  const out = gh(['pr', 'checks', pr, '--repo', repoSlug, '--json', 'name,state,conclusion,link']);
  jobs = JSON.parse(out).map(x => ({ name: x.name, state: x.state, conclusion: x.conclusion, link: x.link }));
} catch { /* checks 读取失败不改变已发生的合并事实 */ }
const merged = !!merge.merged;
const mergeCommit = merge.sha || null;
let mergeRecordPath = null;
try {
  mergeRecordPath = L.writeMergeRecord(P, b.id, {
    pr: Number(pr), merge_commit: mergeCommit, merge_called: mergeCalled, head_sha: head,
    checks: jobs, bootstrap_used: bootstrapUsed, merged, method, repo: repoSlug,
  });
} catch (e) { process.stderr.write('MERGE_RECORD_WRITE_FAILED：' + String(e.message || e) + '\n'); }
console.log(JSON.stringify({ verdict: merged ? 'PASS' : 'FAIL', batch: b.id, pr: Number(pr), method, repo: repoSlug, merge_called: true, merged, merge_commit: mergeCommit, message: merge.message || '', head, checks: jobs, bootstrap_used: bootstrapUsed, merge_record: mergeRecordPath }, null, 2));
process.exit(merged ? 0 : 1);
