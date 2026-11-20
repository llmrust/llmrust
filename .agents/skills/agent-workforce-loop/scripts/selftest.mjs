#!/usr/bin/env node
/**
 * 自测：在临时 git 仓库里把 loop 从 IDLE 跑到交付→issue→回流→下一批，
 * 同时逐条验证「该拦的都拦住」（v2 的漏洞全部作为反例用例）。
 * 远端 CI 用临时目录里的假 gh 模拟，仅用于本自测。
 */
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync, chmodSync, cpSync } from 'node:fs';
import { join, dirname, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { execFileSync, spawnSync } from 'node:child_process';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { createHash } from 'node:crypto';

const S = resolve(dirname(fileURLToPath(import.meta.url)));
const SKILL_DIR = resolve(S, '..');
const R = mkdtempSync(join(tmpdir(), 'awl-'));
const BIN = mkdtempSync(join(tmpdir(), 'awl-bin-'));
const ORIGIN = mkdtempSync(join(tmpdir(), 'awl-origin-'));
const W = join(R, '.workforce');
const sha = (p) => createHash('sha256').update(readFileSync(join(R, p))).digest('hex');
const wj = (p, o) => { mkdirSync(dirname(join(R, p)), { recursive: true }); writeFileSync(join(R, p), JSON.stringify(o, null, 2)); };
const wt = (p, t) => { mkdirSync(dirname(join(R, p)), { recursive: true }); writeFileSync(join(R, p), t); };
const g = (...a) => execFileSync('git', a, { cwd: R, stdio: 'pipe' }).toString().trim();
const commitAs = (email, msg) => { g('add', '-A'); g('-c', `user.email=${email}`, '-c', 'user.name=x', 'commit', '-q', '--allow-empty', '-m', msg); };
// 模拟「用户推到受保护分支」：真实环境里 agent 凭据推不上该分支
const pushUser = () => g('push', '-q', 'origin', 'HEAD:main');
const selfSha = (o) => { const c = { ...o }; delete c.self_sha256; return { ...o, self_sha256: createHash('sha256').update(JSON.stringify(c)).digest('hex') }; };
const env = { ...process.env, PATH: BIN + (process.platform === 'win32' ? ';' : ':') + process.env.PATH };
let fails = 0, n = 0;
function run(script, args, repo = R, extraEnv = {}) {
  const r = spawnSync(process.execPath, [join(S, script), ...args, '--repo', repo], { env: { ...env, ...extraEnv }, encoding: 'utf8' });
  let j = {}; try { j = JSON.parse(r.stdout); } catch {}
  return { code: r.status, j, raw: r.stdout };
}
function expect(name, script, args, code, check) {
  n++;
  const r = run(script, args);
  const ok = r.code === code && (!check || check(r.j));
  if (!ok) { fails++; console.log('✗', name, '→ exit', r.code, JSON.stringify(r.j).slice(0, 400)); }
  else console.log('✓', name);
  return r.j;
}
const has = (k) => (j) => JSON.stringify(j).includes(k);

try {
  execFileSync('git', ['init', '-q', '--bare', ORIGIN]);
  g('init', '-q');
  g('remote', 'add', 'origin', ORIGIN);
  const USER = 'boss@example.com', AGENT = 'agent@bot.local';
  wj('.workforce/acceptors.json', { emails: [USER] });
  wt('README.md', 'x');
  commitAs(USER, 'acceptors'); pushUser();
  expect('无 config.json → 一切迁移拒绝', 'loop.mjs', ['architect-spec', '--as', 'architect-a'], 1, has('CONFIG_MISSING'));
  expect('scaffold 生成 config 模板', 'loop.mjs', ['scaffold'], 0, has('config.json'));
  wj('.workforce/config.json', { schema: 'agent-workforce-config/v1', business_dirs: ['web/src/', 'host/src/'], gates: { required_commands: [] } });
  expect('config 没有门禁命令 → 拒绝', 'loop.mjs', ['architect-spec', '--as', 'architect-a'], 1, has('CONFIG_NO_GATE_COMMANDS'));
  const CONFIG = { schema: 'agent-workforce-config/v1', business_dirs: ['web/src/', 'host/src/', 'contract/'],
    acceptance: { user_confirm: true }, trust: { anchor: 'remote', remote: 'origin', branch: 'main' },
    architect_required_inputs: ['AGENTS.md'], gates: { required_commands: ['verify-ui-slice'], require_no_mock: true },
    release: { overall_file: 'docs/ui-ledger.json', overall_pointer: '/summary/overall' }, ci: { provider: 'github', repo: 'x/y' },
    frontend: { dirs: ['web/'], web_guidance: true }, ledger: { command: 'node gen-ledger.mjs' } };
  wt('gen-ledger.mjs', `import{writeFileSync,mkdirSync}from'node:fs';import{createHash}from'node:crypto';mkdirSync('out',{recursive:true});` +
    `const o={schema:'ui-ledger/v1',generatedAt:new Date().toISOString(),rows:2};o.self_sha256=createHash('sha256').update(JSON.stringify(o)).digest('hex');` +
    `writeFileSync('out/ledger.json',JSON.stringify(o,null,2));`);
  wj('.workforce/config.json', { ...CONFIG, ledger: {} });
  expect('config 没有台账生成命令也没写理由 → 拒绝', 'loop.mjs', ['config-check'], 1, has('CONFIG_LEDGER_UNVERIFIED'));
  wj('.workforce/config.json', CONFIG);
  commitAs(AGENT, 'agent writes config');
  expect('config 由 agent 提交 → config-check 拒绝', 'loop.mjs', ['config-check'], 1, has('CONFIG_NOT_COMMITTED_BY_ACCEPTOR'));
  wj('.workforce/config.json', { ...CONFIG, _by: 'user' });
  commitAs(USER, 'user config');
  expect('config 由用户提交但未进受保护分支 → 拒绝', 'loop.mjs', ['config-check'], 1, has('TRUST_NOT_ON_PROTECTED_BRANCH'));
  pushUser();
  expect('config 由用户提交并进入远端受保护分支 → config-check PASS', 'loop.mjs', ['config-check'], 0, (j) => !(j.warnings || []).length);
  // ----- spec + cards -----
  const spec = {
    produced_by: 'architect-a', inputs_read: ['AGENTS.md'],
    constitution: { priority: 'highest', articles: [{ id: 'K-01', text: 't', enforced_by: 'e' }] },
    total_vertical_slice: { id: 'VS-0', goal: 'g', acceptance: 'a', serves_all_modules: true },
    phases: [{ id: 'PG-1', phase_vertical_slice: { id: 'VS-1', goal: 'g', serves: 'VS-0' }, batches: [
      { id: 'B-1', goal: 'g', tasks: [{ id: 'T1', role: 'dev', card: 'C1' }, { id: 'T2', role: 'dev', card: 'C2' }] },
      { id: 'B-2', goal: 'g', tasks: [{ id: 'T3', role: 'dev', card: 'C3' }] }] }],
  };
  const card = (id, scope, blocked = []) => ({ card_id: id, phase: 'PG-1', owner_agent: 'dev-' + id, qa_agent: 'qa-' + id,
    write_scopes: [scope], acceptance_criteria: ['GIVEN WHEN THEN'], independent_test: 'node t', blocked_by: blocked, checkpoint: 'cp' });
  wj('.workforce/cards/C1.json', card('C1', 'web/src/account/'));
  wj('.workforce/cards/C2.json', card('C2', 'web/src/plan/'));
  wj('.workforce/cards/C3.json', card('C3', 'web/src/chat/', ['C1']));
  wj('.workforce/spec.json', spec);
  commitAs(AGENT, 'spec');
  const specSha = () => sha('.workforce/spec.json');

  // ----- 审批链 -----
  expect('Lead 不能写 spec（guard）', 'loop.mjs', ['guard', '--action', 'architect-spec'], 1, has('ARCHITECT_ONLY'));
  expect('Lead 不能执行 architect-spec', 'loop.mjs', ['architect-spec'], 1, has('ARCHITECT_ONLY'));
  expect('工种动作一律拒绝', 'loop.mjs', ['guard', '--action', 'implement'], 1, has('ROLE_WORK'));
  expect('produced_by 与执行者不符拒绝', 'loop.mjs', ['architect-spec', '--as', 'architect-b'], 1, has('SPEC_PRODUCER_MISMATCH'));
  expect('架构师产出 spec', 'loop.mjs', ['architect-spec', '--as', 'architect-a'], 0);
  expect('无签署不能 review', 'loop.mjs', ['spec-review'], 1, has('SIGNOFF_MISSING'));
  for (const r of ['architect', 'dev', 'qa', 'gate', 'ci', 'ledger']) wj(`.workforce/signoffs/${r}.json`, { role: r, agent: r + '-x', spec_sha256: specSha(), agreed: true });
  wj('.workforce/signoffs/design.json', { role: 'design', agent: 'dev-x', spec_sha256: specSha(), agreed: true });
  expect('签署 agent 角色不符 / 复用拒绝', 'loop.mjs', ['spec-review'], 1, has('SIGNOFF_AGENT_NOT_ROLE'));
  wj('.workforce/signoffs/design.json', { role: 'design', agent: 'design-x', spec_sha256: 'old', agreed: true });
  expect('签署绑定旧 spec 拒绝', 'loop.mjs', ['spec-review'], 1, has('SIGNOFF_STALE_SPEC'));
  wj('.workforce/signoffs/design.json', { role: 'design', agent: 'design-x', spec_sha256: specSha(), agreed: true });
  expect('六工种全部同意 → SPEC_REVIEWED', 'loop.mjs', ['spec-review'], 0);
  expect('无 Lead 签署不能 approve', 'loop.mjs', ['spec-approve'], 1, has('LEAD_SIGNOFF_MISSING'));
  wj('.workforce/signoffs/lead.json', { role: 'lead', agent: 'lead', spec_sha256: specSha(), approved: true });
  expect('Lead 审批 → SPEC_APPROVED', 'loop.mjs', ['spec-approve'], 0);
  wj('.workforce/signoffs/user.json', { role: 'user', agent: 'boss', spec_sha256: specSha(), confirmed: true });
  expect('用户签署未提交拒绝', 'loop.mjs', ['spec-confirm'], 1, has('NOT_COMMITTED'));
  commitAs(AGENT, 'agent forges user signoff');
  expect('agent 代提交用户签署拒绝', 'loop.mjs', ['spec-confirm'], 1, has('NOT_COMMITTED_BY_ACCEPTOR'));
  wj('.workforce/signoffs/user.json', { role: 'user', agent: 'boss', spec_sha256: specSha(), confirmed: true, signedAt: 'now' });
  commitAs(USER, 'user confirms'); pushUser();
  expect('用户亲自提交 → SPEC_CONFIRMED', 'loop.mjs', ['spec-confirm'], 0);

  // ----- config 漂移 -----
  const cfgBak = readFileSync(join(W, 'config.json'));
  wj('.workforce/config.json', { ...CONFIG, gates: { required_commands: ['true'] } });
  expect('确认后放宽 config → CONFIG_DRIFT', 'loop.mjs', ['init'], 1, has('CONFIG_DRIFT'));
  writeFileSync(join(W, 'config.json'), cfgBak);

  // ----- 状态文件防篡改 -----
  const st = JSON.parse(readFileSync(join(W, 'loop-state.json'), 'utf8'));
  const bak = readFileSync(join(W, 'loop-state.json'));
  st.state = 'ACCEPTED'; writeFileSync(join(W, 'loop-state.json'), JSON.stringify(st));
  expect('手改状态文件拒绝', 'loop.mjs', ['status'], 1, has('STATE_TAMPERED'));
  writeFileSync(join(W, 'loop-state.json'), bak);

  // ----- 执行 -----
  expect('init 从已确认 spec 展开批次', 'loop.mjs', ['init'], 0, has('B-1'));
  commitAs(AGENT, 'state');
  expect('dispatch B-1（只唤醒 dev，测试等交接）', 'loop.mjs', ['dispatch'], 0, (j) => has('dev-C1')(j) && j.wake.every(w => w.role === 'dev') && j.qa_waits_for_handoff.length === 2);
  expect('开发还没交卷 → 不能交接给测试', 'loop.mjs', ['handoff', '--card', 'C1'], 1, has('DEV_RECEIPT_MISSING'));
  expect('v2 漏洞：空回执直接 evidence → 拒绝', 'loop.mjs', ['evidence'], 1, has('EVIDENCE_MISSING'));
  expect('v2 漏洞：跳过 evidence 直接 accept → 拒绝', 'loop.mjs', ['accept'], 1, has('STATE_FORBIDS'));

  // Lead 偷偷写业务代码
  wt('web/src/account/Login.tsx', 'export {}');
  wt('web/src/frame/Hack.tsx', 'lead wrote this');
  expect('业务改动未被认领 → check-drift FAIL', 'loop.mjs', ['check-drift'], 1, has('UNCLAIMED_BUSINESS_CHANGE web/src/frame/Hack.tsx'));
  rmSync(join(R, 'web/src/frame'), { recursive: true });
  wt('web/src/plan/Plan.tsx', 'export {}');

  const E = '.workforce/evidence/B-1';
  wt('out/c1.txt', 'ok'); wt('out/c2.txt', 'ok'); wt('out/shot.png', 'png');
  wj('out/ledger.json', { schema: 'ui-ledger/v1', generatedAt: 'now', self_sha256: 'x' });
  const ev = (role, agent, card, extra = {}) => ({ schema: 'agent-workforce-evidence/v1', role, agent, card_id: card, batch_id: 'B-1',
    spec_sha256: specSha(), artifacts: [{ path: 'out/c1.txt', sha256: sha('out/c1.txt') }],
    command_exit_codes: [{ cmd: 'node t', exit_code: 0 }], verdict: 'PASS', independent: true, declared_by_lead: false, written_by: 'qa-fixture', written_by: (extra && extra.written_by) || String(extra && extra.role || 'dev') + '-fixture', ...extra });
  wt('out/mwg-c1.md', '# light-dismiss-dialog\nguide text');
  const devC1 = { files_changed: ['web/src/account/Login.tsx'] };
  wj(`${E}/dev-C1.json`, ev('dev', 'dev-C1', 'C1', devC1));
  const mwg = {
    files_changed: ['web/src/plan/Plan.tsx'],
    web_guidance: { queries: ['light dismiss dialog'], guide_ids: ['light-dismiss-dialog'], retrieved_artifact: 'out/mwg-c1.md',
      deviations: [{ guide_id: 'light-dismiss-dialog', overridden_by: 'tech-stack R-UI-02 Radix headless', note: '用 Radix Dialog 但保留 closedby 语义' }] },
    artifacts: [{ path: 'out/mwg-c1.md', sha256: sha('out/mwg-c1.md') }],
    command_exit_codes: [{ cmd: 'npx -y modern-web-guidance@latest search "light dismiss dialog"', exit_code: 0 },
      { cmd: 'npx -y modern-web-guidance@latest retrieve "light-dismiss-dialog"', exit_code: 0 }, { cmd: 'node t', exit_code: 0 }],
  };
  wj(`${E}/dev-C2.json`, ev('dev', 'dev-C2', 'C2', mwg));
  const qa = { layers_covered: ['事件', '逻辑', '接口'], rule_ids: ['AUTH-L-08'], guidance_verified: ['light-dismiss-dialog'] };
  const hand = (c) => run('loop.mjs', ['handoff', '--card', c]).j.handoff_id;
  const reQa = () => { for (const c of ['C1', 'C2']) wj(`${E}/qa-${c}.json`, ev('qa', 'qa-' + c, c, { ...qa, handoff_id: hand(c) })); };
  wj(`${E}/qa-C1.json`, ev('qa', 'dev-C1', 'C1', qa));
  wj(`${E}/qa-C2.json`, ev('qa', 'qa-C2', 'C2', qa));
  wj(`${E}/gate.json`, ev('gate', 'gate-1', null, { mock_used: false, command_exit_codes: [{ cmd: 'node scripts/verify-ui-slice.mjs', exit_code: 0 }] }));
  wj(`${E}/ledger.json`, ev('ledger', 'ledger-1', null, { artifacts: [{ path: 'out/ledger.json', sha256: sha('out/ledger.json') }] }));
  wj(`${E}/design.json`, ev('design', 'design-1', null, { prototype_refs: ['#/login'], artifacts: [{ path: 'out/shot.png', sha256: 'deadbeef' }] }));
  expect('开发自测冒充 QA、sha 造假 → 拒绝', 'loop.mjs', ['evidence'], 1, (j) => has('QA_AGENT_NOT_CARD_QA')(j) && has('ARTIFACT_SHA_MISMATCH')(j) && has('AGENT_REUSE')(j));
  expect('前端卡没跑 modern-web-guidance → 拒绝', 'loop.mjs', ['evidence'], 1, has('WEB_GUIDANCE_MISSING dev-C1.json'));
  wj(`${E}/dev-C1.json`, ev('dev', 'dev-C1', 'C1', { ...devC1, web_guidance: { queries: ['email input validation'], guide_ids: ['form-validation'], retrieved_artifact: 'out/mwg-c1.md' },
    artifacts: [{ path: 'out/mwg-c1.md', sha256: sha('out/mwg-c1.md') }],
    command_exit_codes: [{ cmd: 'npx -y modern-web-guidance@latest search "email input validation"', exit_code: 0 }, { cmd: 'npx -y modern-web-guidance@latest retrieve "form-validation"', exit_code: 0 }] }));
  expect('声称取了指南但存档里没有该指南 → 拒绝', 'loop.mjs', ['evidence'], 1, has('WEB_GUIDANCE_OUTPUT_LACKS_IDS dev-C1.json'));
  wj(`${E}/dev-C1.json`, ev('dev', 'dev-C1', 'C1', { ...devC1, web_guidance: { queries: ['email input validation'], guide_ids: [], none_applicable_reason: '查了 form validation，指南针对原生表单提交，本卡只做显示态' },
    command_exit_codes: [{ cmd: 'npx -y modern-web-guidance@latest search "email input validation"', exit_code: 0 }, { cmd: 'node t', exit_code: 0 }] }));
  wj(`${E}/qa-C2.json`, ev('qa', 'qa-C2', 'C2', { ...qa, guidance_verified: [] }));
  wj(`${E}/qa-C1.json`, ev('qa', 'qa-C1', 'C1', qa));
  expect('QA 没核对 dev 用到的指南 → 拒绝', 'loop.mjs', ['evidence'], 1, has('QA_DID_NOT_VERIFY_GUIDANCE C2'));
  wj(`${E}/qa-C2.json`, ev('qa', 'qa-C2', 'C2', qa));
  wj(`${E}/design.json`, ev('design', 'design-1', null, { prototype_refs: ['#/login'], artifacts: [{ path: 'out/shot.png', sha256: sha('out/shot.png') }] }));
  expect('测试回执没走交接（与开发同时进行）→ 拒绝', 'loop.mjs', ['evidence'], 1, has('QA_WITHOUT_HANDOFF C1'));
  reQa();
  wt('web/src/account/Login.tsx', 'dev keeps editing while qa tests');
  expect('交接后开发又改代码 → 测试作废', 'loop.mjs', ['evidence'], 1, has('DEV_CHANGED_AFTER_HANDOFF C1'));
  wt('web/src/account/Login.tsx', 'export {}');
  hand('C1');
  expect('重新交接后，旧测试回执不算数', 'loop.mjs', ['evidence'], 1, has('QA_NOT_FOR_CURRENT_HANDOFF C1'));
  reQa();
  expect('check-roles.mjs 独立运行 PASS', 'check-roles.mjs', [], 0);
  expect('回执齐全 → EVIDENCE_COLLECTED', 'loop.mjs', ['evidence'], 0);
  expect('overall 未达标 + 手写台账（self_sha256 不自洽）→ accept FAIL 并自动 REOPEN', 'loop.mjs', ['accept'], 1, (j) => has('OVERALL_FALSE')(j) && has('LEDGER_SELF_SHA_INVALID')(j));
  wj('docs/ui-ledger.json', { summary: { overall: true } });
  expect('REOPEN 后重新 dispatch（工作区仍有未提交改动）→ 拒绝', 'loop.mjs', ['dispatch'], 1, has('DIRTY_BUSINESS_TREE'));
  commitAs(AGENT, 'batch 1 work');
  // v1.0.0 漏洞：回流重派会重置基线，夹在中间提交的未认领改动掉出比对窗口
  wt('web/src/frame/Sneak.tsx', 'committed between rounds'); commitAs(AGENT, 'sneak');
  expect('提交后重新 dispatch', 'loop.mjs', ['dispatch'], 0);
  expect('回流期间提交的未认领业务改动 → 仍被 evidence 拦下（批次基线不重置）', 'loop.mjs', ['evidence'], 1, has('UNCLAIMED_BUSINESS_CHANGE web/src/frame/Sneak.tsx'));
  rmSync(join(R, 'web/src/frame'), { recursive: true }); commitAs(AGENT, 'revert sneak');
  reQa();
  // v1.0.0 漏洞：台账只验字段存在。现在自洽也不够，accept 重跑 ledger.command 比对
  wj('out/ledger.json', selfSha({ schema: 'ui-ledger/v1', generatedAt: 'now', rows: 999 }));
  wj(`${E}/ledger.json`, ev('ledger', 'ledger-1', null, { artifacts: [{ path: 'out/ledger.json', sha256: sha('out/ledger.json') }] }));
  expect('evidence', 'loop.mjs', ['evidence'], 0);
  expect('自洽但手写的台账 → 重跑生成命令不一致 → accept FAIL', 'loop.mjs', ['accept'], 1, has('LEDGER_REGEN_MISMATCH'));
  execFileSync(process.execPath, ['gen-ledger.mjs'], { cwd: R });
  wj(`${E}/ledger.json`, ev('ledger', 'ledger-1', null, { artifacts: [{ path: 'out/ledger.json', sha256: sha('out/ledger.json') }] }));
  expect('回流 dispatch', 'loop.mjs', ['dispatch'], 0);
  reQa();
  expect('evidence', 'loop.mjs', ['evidence'], 0);
  expect('五层 PASS（台账由生成命令重跑一致）→ ACCEPTED', 'loop.mjs', ['accept'], 0);
  expect('无 gh / 远端 CI → release 拒绝', 'loop.mjs', ['release'], 1, (j) => has('CI_')(j) || has('WORKTREE_DIRTY')(j));
  commitAs(AGENT, 'state');
  const head = g('rev-parse', 'HEAD');
  // CI 强制门：先推送 HEAD，再由 ci 工种写回执（release 会核对 head_sha 与远端运行结论）
  g('push', '-q', '--force', 'origin', 'HEAD:refs/heads/main');
  const writeCi = (concl) => wj(E + '/ci.json', { schema: 'agent-workforce-evidence/v1', role: 'ci', agent: 'ci-1', card_id: null, batch_id: 'B-1', spec_sha256: 'selftest', provider: 'github', verdict: 'PASS', pushed: { remote: 'origin', branch: 'main', head_sha: head }, workflow_runs: [{ name: 'ci', run_id: '1', head_sha: head, conclusion: concl, url: 'https://x/run/1' }], command_exit_codes: [{ cmd: 'gh run list', exit_code: 0 }], mock_used: false, independent: true, declared_by_lead: false, notes: 'selftest' });
  const fakeGh = (concl) => {
    const js = `#!/usr/bin/env node\nconsole.log(JSON.stringify([{conclusion:${JSON.stringify(concl)},status:'completed',workflowName:'ci',headSha:${JSON.stringify(head)}}]))\n`;
    writeFileSync(join(BIN, 'gh'), js); chmodSync(join(BIN, 'gh'), 0o755);
    writeFileSync(join(BIN, 'gh.cmd'), `@node "%~dp0gh" %*\r\n`);
  };
  fakeGh('failure'); writeCi('failure');
  expect('远端 CI failure → release 拒绝', 'loop.mjs', ['release'], 1, has('CI_NOT_SUCCESS'));
  fakeGh('success'); writeCi('success');
  expect('CI success → RELEASED', 'loop.mjs', ['release'], 0);
  expect('deliver → DELIVERED', 'loop.mjs', ['deliver'], 0);
  expect('交付后 ready 列出 B-2（C1 已交付，C3 解锁）', 'loop.mjs', ['ready'], 0, (j) => j.ready_to_dispatch.includes('B-2'));
  expect('B-2 的 C3 依赖已交付的 C1 → dispatch 可行（guard）', 'loop.mjs', ['guard', '--action', 'dispatch', '--batch', 'B-2'], 0);

  // ----- 用户 issue -----
  const issue = { issue_id: 'ISSUE-001', raised_by: 'user', artifact: '#/login', layer: '视图', expected: 'a', observed: 'b', severity: 'major', trace_to_card: 'C2', used_as_pass_evidence: false };
  wj('.workforce/issues/ISSUE-001.json', issue);
  commitAs(AGENT, 'agent issue');
  expect('agent 冒充用户提 issue → 拒绝', 'loop.mjs', ['user-issue', '--issue', join(R, '.workforce/issues/ISSUE-001.json')], 1, has('NOT_COMMITTED_BY_ACCEPTOR'));
  wj('.workforce/issues/ISSUE-001.json', { ...issue, note: 'by user' });
  commitAs(USER, 'user issue'); pushUser();
  expect('用户 issue → REOPEN 到 C2 所在批次', 'loop.mjs', ['user-issue', '--issue', join(R, '.workforce/issues/ISSUE-001.json')], 0, has('B-1'));
  expect('REOPEN 允许 dispatch', 'loop.mjs', ['guard', '--action', 'dispatch', '--batch', 'B-1'], 0);

  // ----- spec 漂移 -----
  const sp2 = JSON.parse(readFileSync(join(W, 'spec.json'), 'utf8')); sp2.total_vertical_slice.goal = 'changed';
  wj('.workforce/spec.json', sp2);
  expect('确认后改 spec → dispatch 拒绝', 'loop.mjs', ['dispatch'], 1, has('SPEC_DRIFT'));
  expect('架构师 spec-revise → 回 SPEC_DRAFT，旧签署失效', 'loop.mjs', ['spec-revise', '--as', 'architect-a'], 0);
  expect('旧签署不能复用', 'loop.mjs', ['spec-review'], 1, has('SIGNOFF_STALE_SPEC'));

  // ----- 下一批 / blocked_by -----
  // 恢复原 spec，用新状态验证 blocked_by
  rmSync(join(W, 'loop-state.json'));
  wj('.workforce/spec.json', spec);
  for (const r of ['architect', 'dev', 'qa', 'gate', 'ledger', 'design']) wj(`.workforce/signoffs/${r}.json`, { role: r, agent: r + '-x', spec_sha256: specSha(), agreed: true });
  wj('.workforce/signoffs/lead.json', { role: 'lead', agent: 'lead', spec_sha256: specSha(), approved: true });
  commitAs(AGENT, 'reset');
  wj('.workforce/signoffs/user.json', { role: 'user', agent: 'boss', spec_sha256: specSha(), confirmed: true, signedAt: 'again' });
  commitAs(USER, 'user confirms again'); pushUser();
  for (const c of ['architect-spec --as architect-a', 'spec-review', 'spec-approve', 'spec-confirm', 'init']) run('loop.mjs', c.split(' '));
  expect('B-1 未交付时 B-2 不能开工（blocked_by）', 'loop.mjs', ['dispatch', '--batch', 'B-2'], 1, has('BLOCKED C3 等待 C1'));
  expect('init 后只有 B-1 可开工', 'loop.mjs', ['ready'], 0, (j) => JSON.stringify(j.ready_to_dispatch) === '["B-1"]');

  // ----- 场景 2：非 UI 项目（design 关闭、CI 用命令、英文层名、--dir 自定义目录） -----
  const R2 = mkdtempSync(join(tmpdir(), 'awl2-'));
  try {
    const g2 = (...a) => execFileSync('git', a, { cwd: R2, stdio: 'pipe' }).toString().trim();
    const c2 = (email, msg) => { g2('add', '-A'); g2('-c', `user.email=${email}`, '-c', 'user.name=x', 'commit', '-q', '--allow-empty', '-m', msg); };
    const D = 'ops/wf';
    const w2 = (p, o) => { mkdirSync(dirname(join(R2, p)), { recursive: true }); writeFileSync(join(R2, p), typeof o === 'string' ? o : JSON.stringify(o, null, 2)); };
    const sha2 = (p) => createHash('sha256').update(readFileSync(join(R2, p))).digest('hex');
    const r2 = (args) => run('loop.mjs', [...args, '--dir', D], R2);
    const e2 = (name, args, code, check) => { n++; const r = r2(args); const ok = r.code === code && (!check || check(r.j)); if (!ok) { fails++; console.log('✗', name, '→ exit', r.code, JSON.stringify(r.j).slice(0, 400)); } else console.log('✓', name); return r.j; };
    g2('init', '-q');
    w2(`${D}/acceptors.json`, { emails: [USER] });
    w2(`${D}/config.json`, { schema: 'agent-workforce-config/v1', business_dirs: ['lib/'], roles: { design: false },
      gates: { required_commands: ['pytest'] }, ledger: { none_reason: '无台账生成器' }, trust: { anchor: 'none', none_reason: '单机演示' }, ci: { provider: 'github', repo: 'x/y' } });
    const sp = { produced_by: 'architect-a', inputs_read: ['README.md'],
      constitution: { priority: 'highest', articles: [{ id: 'K-01', text: 't', enforced_by: 'e' }] },
      total_vertical_slice: { id: 'VS-0', goal: 'g', acceptance: 'a', serves_all_modules: true },
      phases: [{ id: 'PH-1', phase_vertical_slice: { id: 'VS-1', goal: 'g', serves: 'VS-0' }, batches: [{ id: 'B-1', goal: 'g', tasks: [{ id: 'T1', role: 'dev', card: 'C1' }] }] }] };
    w2(`${D}/spec.json`, sp);
    w2(`${D}/cards/C1.json`, { card_id: 'C1', phase: 'PH-1', owner_agent: 'dev-c1', qa_agent: 'qa-c1', write_scopes: ['lib/core/'], acceptance_criteria: ['x'], independent_test: 'pytest', blocked_by: [], checkpoint: 'cp' });
    c2(USER, 'setup');
    e2('[2] trust=none / 无台账命令 → status 永久提示', ['status'], 0, (j) => has('TRUST_NONE')(j) && has('LEDGER_UNVERIFIED')(j));
    e2('[2] 自定义目录 + architect-spec', ['architect-spec', '--as', 'architect-a'], 0);
    const ss = () => sha2(`${D}/spec.json`);
    for (const r of ['architect', 'dev', 'qa', 'gate', 'ci', 'ledger']) w2(`${D}/signoffs/${r}.json`, { role: r, agent: r + '-x', spec_sha256: ss(), agreed: true });
    e2('[2] design 关闭时六工种（含 ci）签署即可 review', ['spec-review'], 0);
    w2(`${D}/signoffs/lead.json`, { role: 'lead', agent: 'lead', spec_sha256: ss(), approved: true });
    e2('[2] approve', ['spec-approve'], 0);
    w2(`${D}/signoffs/user.json`, { role: 'user', agent: 'u', spec_sha256: ss(), confirmed: true });
    c2(USER, 'confirm');
    e2('[2] confirm', ['spec-confirm'], 0);
    e2('[2] init', ['init'], 0);
    c2(AGENT, 'state');
    const dj = e2('[2] dispatch 给出与 harness 无关的 brief，且无 design', ['dispatch'], 0, (j) => j.wake?.[0]?.brief?.includes('dev-c1') && !JSON.stringify(j.required_evidence).includes('design'));
    w2('lib/core/a.py', 'x'); w2('out/t.txt', 'ok'); w2('out/l.json', selfSha({ schema: 's', generatedAt: 'n' }));
    const E2 = `${D}/evidence/B-1`;
    const ev2 = (role, agent, card, extra = {}) => ({ schema: 'agent-workforce-evidence/v1', role, agent, card_id: card, batch_id: 'B-1', spec_sha256: ss(),
      artifacts: [{ path: 'out/t.txt', sha256: sha2('out/t.txt') }], command_exit_codes: [{ cmd: 'pytest', exit_code: 0 }], verdict: 'PASS', independent: true, declared_by_lead: false, written_by: 'qa-fixture', written_by: (extra && extra.written_by) || String(extra && extra.role || 'dev') + '-fixture', ...extra });
    w2(`${E2}/dev-C1.json`, ev2('dev', 'dev-c1', 'C1', { files_changed: ['lib/core/a.py'] }));
    const qa2 = () => w2(`${E2}/qa-C1.json`, ev2('qa', 'qa-c1', 'C1', { layers_covered: ['event', 'logic', 'interface'], rule_ids: ['R-1'], handoff_id: r2(['handoff', '--card', 'C1']).j.handoff_id }));
    qa2();
    w2(`${E2}/gate.json`, ev2('gate', 'gate-1', null, { mock_used: false, command_exit_codes: [{ cmd: 'npm run lint', exit_code: 0 }] }));
    w2(`${E2}/ledger.json`, ev2('ledger', 'ledger-1', null, { artifacts: [{ path: 'out/l.json', sha256: sha2('out/l.json') }] }));
    e2('[2] evidence（非前端卡不要求 web_guidance）', ['evidence'], 0);
    e2('[2] gate 没跑 config 要求的命令 → accept FAIL', ['accept'], 1, has('pytest'));
    c2(AGENT, 'work');
    e2('[2] 回流后 dispatch', ['dispatch'], 0);
    qa2();
    w2(`${E2}/gate.json`, ev2('gate', 'gate-1', null, { mock_used: false, command_exit_codes: [{ cmd: 'pytest -q', exit_code: 0 }] }));
    e2('[2] evidence', ['evidence'], 0);
    e2('[2] 英文层名 + view 跳过 → ACCEPTED', ['accept'], 0, (j) => JSON.stringify(j.layers).includes('skipped'));
    c2(AGENT, 'state');
    e2('[2] 有 CI 强制门（无远端/无回执）→ release 拒绝', ['release'], 1, (j) => JSON.stringify(j).includes('CI_'));
  } finally { rmSync(R2, { recursive: true, force: true }); }

  // ----- 场景 4：并行批次（前端批 + 后端批同时做）、一卡一人、用户不做开工确认 -----
  const R4 = mkdtempSync(join(tmpdir(), 'awl4-'));
  try {
    const g4 = (...a) => execFileSync('git', a, { cwd: R4, stdio: 'pipe' }).toString().trim();
    const c4 = (email, msg) => { g4('add', '-A'); g4('-c', `user.email=${email}`, '-c', 'user.name=x', 'commit', '-q', '--allow-empty', '-m', msg); };
    const w4 = (p, o) => { mkdirSync(dirname(join(R4, p)), { recursive: true }); writeFileSync(join(R4, p), typeof o === 'string' ? o : JSON.stringify(o, null, 2)); };
    const sha4 = (p) => createHash('sha256').update(readFileSync(join(R4, p))).digest('hex');
    const e4 = (name, args, code, check) => { n++; const r = run('loop.mjs', args, R4); const ok = r.code === code && (!check || check(r.j)); if (!ok) { fails++; console.log('✗', name, '→ exit', r.code, JSON.stringify(r.j).slice(0, 400)); } else console.log('✓', name); return r.j; };
    const D = '.workforce';
    g4('init', '-q');
    w4(`${D}/acceptors.json`, { emails: [USER] });
    // 默认：用户不做技术验收（user_confirm 默认 false）、规则表本地审核后由执行侧提交（trust 默认 local_review）
    w4(`${D}/config.json`, { schema: 'agent-workforce-config/v1', business_dirs: ['web/', 'host/'], roles: { design: false },
      gates: { required_commands: ['t'] }, ledger: { none_reason: 'demo' },
      generated: [{ path: 'docs/gen.md', command: 'node gen.mjs' }], ci: { provider: 'github', repo: 'x/y' } });
    w4('gen.mjs', "import{writeFileSync,mkdirSync}from'node:fs';mkdirSync('docs',{recursive:true});writeFileSync('docs/gen.md','# 机器生成\\n');");
    execFileSync(process.execPath, ['gen.mjs'], { cwd: R4 });
    const task = (id, card) => ({ id, role: 'dev', card });
    const sp4 = { produced_by: 'architect-a', inputs_read: ['README.md'],
      constitution: { priority: 'highest', articles: [{ id: 'K-01', text: 't', enforced_by: 'e' }] },
      total_vertical_slice: { id: 'VS-0', goal: 'g', acceptance: 'a', serves_all_modules: true },
      phases: [{ id: 'PH-1', phase_vertical_slice: { id: 'VS-1', goal: 'g', serves: 'VS-0' }, batches: [
        { id: 'B-F', goal: '前端', tasks: [task('T1', 'F1')] }, { id: 'B-B', goal: '后端', tasks: [task('T2', 'K1'), task('T9', 'F1')] }, { id: 'B-X', goal: 'x', tasks: [task('T3', 'X1')] }] }] };
    const cd = (id, scope, owner, qa) => ({ card_id: id, phase: 'PH-1', owner_agent: owner, qa_agent: qa, write_scopes: [scope], acceptance_criteria: ['x'], independent_test: 't', blocked_by: [], checkpoint: 'cp' });
    w4(`${D}/cards/F1.json`, cd('F1', 'web/src/', 'dev-f', 'qa-f'));
    w4(`${D}/cards/K1.json`, cd('K1', 'host/src/', 'dev-b', 'qa-b'));
    w4(`${D}/cards/X1.json`, cd('X1', 'web/src/x/', 'dev-x', 'dev-x'));
    w4(`${D}/spec.json`, sp4);
    c4('exec@local', 'setup by executor');
    // ---- 用户同意的佐证 ----
    const LIB = await import(pathToFileURL(join(S, 'lib.mjs')).href);
    const P4 = LIB.paths(R4, {});
    e4('[4] 没登记审批人 → config-check 拒绝', ['config-check'], 1, has('APPROVER_NOT_SET'));
    const user = LIB.makeApprover('user-secret-pw');
    w4(`${D}/approver.pub.json`, user.pub);
    e4('[4] 有审批人但用户没签同意 → 拒绝（没有佐证就不可信）', ['config-check'], 1, has('USER_APPROVAL_MISSING config'));
    const cfgSha = () => sha4(`${D}/config.json`);
    const approve = (store, pw, kind, target) => { const r = LIB.signApproval(LIB.unlockApprover(store, pw), store.fingerprint, kind, target, sha4(target)); w4(LIB.relP(P4, LIB.approvalFile(P4, kind, sha4(target))), r); return r; };
    // agent 自己造一把钥匙签 → 指纹不是用户的
    const fake = LIB.makeApprover('agent-made-pw');
    approve(fake.store, 'agent-made-pw', 'config', `${D}/config.json`);
    e4('[4] agent 用自己的钥匙伪造同意 → 拒绝', ['config-check'], 1, has('USER_APPROVAL_WRONG_APPROVER'));
    // agent 篡改同意记录里的内容（签名不再匹配）
    const rec = approve(user.store, 'user-secret-pw', 'config', `${D}/config.json`);
    w4(LIB.relP(P4, LIB.approvalFile(P4, 'config', cfgSha())), { ...rec, approved_at: '2020-01-01T00:00:00Z' });
    e4('[4] 同意记录被改 → 签名不符，拒绝', ['config-check'], 1, has('USER_APPROVAL_SIGNATURE_INVALID'));
    let threw = false; try { LIB.unlockApprover(user.store, 'guess-123'); } catch { threw = true; }
    n++; if (threw) console.log('✓ [4] 不知道密码打不开用户的钥匙'); else { fails++; console.log('✗ [4] 错密码竟能解锁'); }
    approve(user.store, 'user-secret-pw', 'config', `${D}/config.json`);
    e4('[4] 用户本人签名同意 → config-check PASS，并给出指纹供核对', ['config-check'], 0, (j) => j.user_approval?.approver_fingerprint === user.pub.fingerprint);
    const sa = run('approve.mjs', ['config', '--repo', R4], R4);
    n++; if (sa.code === 1 && /APPROVE_NEEDS_TERMINAL/.test(sa.raw || JSON.stringify(sa))) console.log('✓ [4] approve.mjs 不在真实终端 → 拒绝（agent 不能代跑）'); else { fails++; console.log('✗ [4] approve.mjs 非终端应拒绝', sa.code); }
    c4('exec@local', 'approvals');
    e4('[4] 同一张卡被两个任务认领 → spec 拒绝', ['architect-spec', '--as', 'architect-a'], 1, has('CARD_ASSIGNED_TWICE F1'));
    sp4.phases[0].batches[1].tasks.pop(); w4(`${D}/spec.json`, sp4);
    e4('[4] architect-spec', ['architect-spec', '--as', 'architect-a'], 0);
    for (const r of ['architect', 'dev', 'qa', 'gate', 'ledger']) w4(`${D}/signoffs/${r}.json`, { role: r, agent: r + '-x', spec_sha256: sha4(`${D}/spec.json`), agreed: true });
    w4(`${D}/signoffs/lead.json`, { role: 'lead', agent: 'lead', spec_sha256: sha4(`${D}/spec.json`), approved: true });
    run('loop.mjs', ['spec-review'], R4); run('loop.mjs', ['spec-approve'], R4);
    const cfgBak4 = readFileSync(join(R4, D, 'config.json'));
    w4(`${D}/config.json`, { ...JSON.parse(cfgBak4), parallel: { max_batches: 9 } });
    e4('[4] 同意后又改规则表 → 同意失效，不能开工', ['spec-confirm'], 1, has('CONFIG_USER_APPROVAL_MISSING'));
    writeFileSync(join(R4, D, 'config.json'), cfgBak4);
    e4('[4] acceptance.user_confirm=false → 不需要用户签署即可确认', ['spec-confirm'], 0);
    e4('[4] 开发和测试是同一个 agent → init 拒绝', ['init'], 1, has('CARD_OWNER_IS_QA X1'));
    w4(`${D}/cards/X1.json`, cd('X1', 'web/src/x/', 'dev-x', 'qa-x'));
    const pubBak = readFileSync(join(R4, D, 'approver.pub.json'));
    w4(`${D}/approver.pub.json`, fake.pub);
    e4('[4] 开工后审批人被换 → 一切迁移拒绝', ['init'], 1, has('APPROVER_KEY_CHANGED'));
    writeFileSync(join(R4, D, 'approver.pub.json'), pubBak);
    e4('[4] init', ['init'], 0, (j) => JSON.stringify(j.ready_to_dispatch) === '["B-F","B-B","B-X"]');
    c4('agent@bot.local', 'state');
    e4('[4] 派前端批 B-F', ['dispatch'], 0, has('B-F'));
    e4('[4] 并行派后端批 B-B', ['dispatch', '--batch', 'B-B'], 0, (j) => j.active_batches.length === 2);
    e4('[4] status 显示两批同时在做', ['status'], 0, (j) => j.state === 'EXECUTING' && j.active_batches.length === 2);
    e4('[4] 第三批：超并行上限且写入范围与前端批重叠 → 拒绝', ['dispatch', '--batch', 'B-X'], 1, (j) => has('PARALLEL_LIMIT')(j) && has('PARALLEL_SCOPE_CONFLICT B-X:web/src/x/')(j));
    w4('web/src/a.js', 'f'); w4('host/src/b.js', 'b'); w4('out/t.txt', 'ok'); w4('out/l.json', selfSha({ schema: 's', generatedAt: 'n' }));
    const ev4 = (b, role, agent, card, extra = {}) => ({ schema: 'agent-workforce-evidence/v1', role, agent, card_id: card, batch_id: b, spec_sha256: sha4(`${D}/spec.json`),
      artifacts: [{ path: 'out/t.txt', sha256: sha4('out/t.txt') }], command_exit_codes: [{ cmd: 't', exit_code: 0 }], verdict: 'PASS', independent: true, declared_by_lead: false, written_by: 'qa-fixture', written_by: (extra && extra.written_by) || String(extra && extra.role || 'dev') + '-fixture', ...extra });
    w4(`${D}/evidence/B-F/dev-F1.json`, ev4('B-F', 'dev', 'dev-f', 'F1', { files_changed: ['web/src/a.js'] }));
    w4(`${D}/evidence/B-B/dev-K1.json`, ev4('B-B', 'dev', 'dev-b', 'K1', { files_changed: ['host/src/b.js'] }));
    e4('[4] 两批都在做时不指定批次 → 拒绝（要求 --batch）', ['evidence'], 1, has('BATCH_AMBIGUOUS'));
    for (const [b, c, q] of [['B-F', 'F1', 'qa-f'], ['B-B', 'K1', 'qa-b']]) {
      const id = run('loop.mjs', ['handoff', '--card', c], R4).j.handoff_id;
      w4(`${D}/evidence/${b}/qa-${c}.json`, ev4(b, 'qa', q, c, { layers_covered: ['event', 'logic', 'interface'], rule_ids: ['R-1'], handoff_id: id }));
      w4(`${D}/evidence/${b}/gate.json`, ev4(b, 'gate', 'gate-' + b, null, { mock_used: false }));
      w4(`${D}/evidence/${b}/ledger.json`, ev4(b, 'ledger', 'ledger-' + b, null, { artifacts: [{ path: 'out/l.json', sha256: sha4('out/l.json') }] }));
    }
    e4('[4] 后端批回执：前端批的改动不算后端批的未认领改动', ['evidence', '--batch', 'B-B'], 0);
    w4('host/lib/z.js', 'lead sneaks');
    e4('[4] 两批范围都不覆盖的业务改动 → 仍被拦下', ['evidence', '--batch', 'B-F'], 1, has('UNCLAIMED_BUSINESS_CHANGE host/lib/z.js'));
    rmSync(join(R4, 'host/lib'), { recursive: true });
    e4('[4] 前端批回执', ['evidence', '--batch', 'B-F'], 0);
    e4('[4] 后端批独立验收', ['accept', '--batch', 'B-B'], 0);
    w4('docs/gen.md', '# 有人手改了生成文件\n');
    e4('[4] 机器生成文件被手改 → 验收拒绝', ['accept', '--batch', 'B-F'], 1, has('GENERATED_HAND_EDITED docs/gen.md'));
    if (readFileSync(join(R4, 'docs/gen.md'), 'utf8') !== '# 有人手改了生成文件\n') { fails++; console.log('✗ [4] 检查后应恢复原文件'); } else console.log('✓ [4] 检查后恢复原文件，不偷偷替你修');
    n++;
  } finally { rmSync(R4, { recursive: true, force: true }); }

  // ----- 场景 5：skill 自己的机器生成文件不许手改 -----
  {
    const T5 = mkdtempSync(join(tmpdir(), 'awl5-')); const S5 = join(T5, 'agent-workforce-loop');
    try {
      cpSync(SKILL_DIR, S5, { recursive: true });
      const v = () => { try { execFileSync(process.execPath, [join(S5, 'scripts', 'validate-skill.mjs')], { stdio: 'pipe' }); return { code: 0, out: '' }; } catch (e) { return { code: e.status, out: e.stdout.toString() }; } };
      const chk = (name, ok) => { n++; if (ok) console.log('✓', name); else { fails++; console.log('✗', name); } };
      chk('[5] 原样 skill → validate PASS', v().code === 0);
      const sm = join(S5, 'references/generated/state-machine.md');
      writeFileSync(sm, readFileSync(sm, 'utf8').replace('ALL_DELIVERED', 'ALL_DONE'));
      chk('[5] 手改生成的状态表 → validate FAIL', v().out.includes('GENERATED_DOC_HAND_EDITED'));
      execFileSync(process.execPath, [join(S5, 'scripts', 'gen-docs.mjs')], { stdio: 'pipe' });
      const rm = join(S5, 'references/roles.md');
      writeFileSync(rm, readFileSync(rm, 'utf8') + '\n跑 loop.mjs finish-all 宣布完成\n');
      chk('[5] 文档写了不存在的命令 → validate FAIL', v().out.includes('DOC_UNKNOWN_COMMAND'));
    } finally { rmSync(T5, { recursive: true, force: true }); }
  }

  // ----- 场景 3：首次占位（agent 抢先自写自提交 acceptors.json + config.json） -----
  const R3 = mkdtempSync(join(tmpdir(), 'awl3-')); const O3 = mkdtempSync(join(tmpdir(), 'awl3o-'));
  try {
    const g3 = (...a) => execFileSync('git', a, { cwd: R3, stdio: 'pipe' }).toString().trim();
    const c3 = (email, msg) => { g3('add', '-A'); g3('-c', `user.email=${email}`, '-c', 'user.name=x', 'commit', '-q', '--allow-empty', '-m', msg); };
    const w3 = (p, o) => { mkdirSync(dirname(join(R3, p)), { recursive: true }); writeFileSync(join(R3, p), JSON.stringify(o, null, 2)); };
    execFileSync('git', ['init', '-q', '--bare', O3]); g3('init', '-q'); g3('remote', 'add', 'origin', O3);
    c3(USER, 'init'); g3('push', '-q', 'origin', 'HEAD:main');
    w3('.workforce/acceptors.json', { emails: [AGENT] });
    w3('.workforce/config.json', { schema: 'agent-workforce-config/v1', business_dirs: ['src/'], gates: { required_commands: ['npm test'] }, ledger: { command: 'node gen.mjs' }, trust: { anchor: 'remote' } });
    c3(AGENT, 'agent self-seals');
    n++;
    const r = run('loop.mjs', ['config-check'], R3);
    if (r.code === 1 && JSON.stringify(r.j).includes('TRUST_NOT_ON_PROTECTED_BRANCH')) console.log('✓ [3] agent 自写自提交 acceptors（首次占位）→ 未进受保护分支，拒绝');
    else { fails++; console.log('✗ [3] 首次占位应被拒', r.code, JSON.stringify(r.j).slice(0, 300)); }
  } finally { rmSync(R3, { recursive: true, force: true }); rmSync(O3, { recursive: true, force: true }); }

  // ----- 场景 6：交卷方角色泛化（role=ci 的卡能 handoff；role=qa 的卡仍被拒） -----
  const R6 = mkdtempSync(join(tmpdir(), 'awl6-'));
  try {
    const g6 = (...a) => execFileSync('git', a, { cwd: R6, stdio: 'pipe' }).toString().trim();
    const c6 = (email, msg) => { g6('add', '-A'); g6('-c', `user.email=${email}`, '-c', 'user.name=x', 'commit', '-q', '--allow-empty', '-m', msg); };
    const D = '.workforce';
    const w6 = (p, o) => { mkdirSync(dirname(join(R6, p)), { recursive: true }); writeFileSync(join(R6, p), typeof o === 'string' ? o : JSON.stringify(o, null, 2)); };
    const sha6 = (p) => createHash('sha256').update(readFileSync(join(R6, p))).digest('hex');
    const e6 = (name, args, code, check) => { n++; const r = run('loop.mjs', args, R6); const ok = r.code === code && (!check || check(r.j)); if (!ok) { fails++; console.log('✗', name, '→ exit', r.code, JSON.stringify(r.j).slice(0, 400)); } else console.log('✓', name); return r.j; };
    g6('init', '-q');
    w6(`${D}/acceptors.json`, { emails: [USER] });
    w6(`${D}/config.json`, { schema: 'agent-workforce-config/v1', business_dirs: ['lib/'], roles: { design: false },
      gates: { required_commands: ['t'] }, ledger: { none_reason: 'demo' }, trust: { anchor: 'none', none_reason: '单机演示' }, ci: { provider: 'github', repo: 'x/y' } });
    w6(`${D}/spec.json`, { produced_by: 'architect-a', inputs_read: ['README.md'],
      constitution: { priority: 'highest', articles: [{ id: 'K-01', text: 't', enforced_by: 'e' }] },
      total_vertical_slice: { id: 'VS-0', goal: 'g', acceptance: 'a', serves_all_modules: true },
      phases: [{ id: 'PH-1', phase_vertical_slice: { id: 'VS-1', goal: 'g', serves: 'VS-0' }, batches: [
        { id: 'B-CI', goal: 'skill 面由 ci 工种交卷', tasks: [{ id: 'T1', role: 'ci', card: 'CCI' }, { id: 'T2', role: 'qa', card: 'CQA' }] }] }] });
    w6(`${D}/cards/CCI.json`, { card_id: 'CCI', phase: 'PH-1', owner_agent: 'ci', qa_agent: 'qa', write_scopes: ['lib/core/'], acceptance_criteria: ['x'], independent_test: 't', blocked_by: [], checkpoint: 'cp' });
    w6(`${D}/cards/CQA.json`, { card_id: 'CQA', phase: 'PH-1', owner_agent: 'qa-x', qa_agent: 'qa-witness', write_scopes: ['lib/qa/'], acceptance_criteria: ['x'], independent_test: 't', blocked_by: [], checkpoint: 'cp' });
    c6(USER, 'setup');
    for (const a of [['architect-spec', '--as', 'architect-a']]) run('loop.mjs', a, R6);
    const ss6 = () => sha6(`${D}/spec.json`);
    for (const r of ['architect', 'dev', 'qa', 'gate', 'ci', 'ledger']) w6(`${D}/signoffs/${r}.json`, { role: r, agent: r + '-x', spec_sha256: ss6(), agreed: true });
    for (const a of [['spec-review']]) run('loop.mjs', a, R6);
    w6(`${D}/signoffs/lead.json`, { role: 'lead', agent: 'lead', spec_sha256: ss6(), approved: true });
    for (const a of [['spec-approve']]) run('loop.mjs', a, R6);
    w6(`${D}/signoffs/user.json`, { role: 'user', agent: 'u', spec_sha256: ss6(), confirmed: true });
    c6(USER, 'confirm');
    for (const a of [['spec-confirm'], ['init']]) run('loop.mjs', a, R6);
    c6(AGENT, 'state');
    e6('[6] dispatch：wake 含 role=ci 的卡（交卷方不再写死 dev）', ['dispatch', '--batch', 'B-CI'], 0,
      (j) => (j.wake || []).some(w => w.role === 'ci' && w.card === 'CCI') && (j.qa_waits_for_handoff || []).some(w => w.card === 'CCI'));
    w6('lib/core/a.txt', 'x'); w6('out/t.txt', 'ok');
    w6(`${D}/evidence/B-CI/ci-CCI.json`, { schema: 'agent-workforce-evidence/v1', role: 'ci', agent: 'ci-seat', card_id: 'CCI', batch_id: 'B-CI',
      spec_sha256: ss6(), artifacts: [{ path: 'out/t.txt', sha256: sha6('out/t.txt') }], command_exit_codes: [{ cmd: 't', exit_code: 0 }],
      verdict: 'PASS', independent: true, declared_by_lead: false, written_by: 'ci-seat', files_changed: ['lib/core/a.txt'] });
    e6('[6] role=ci 的卡可 handoff：冻结文件 + handoff_id + 唤醒 qa', ['handoff', '--card', 'CCI'], 0,
      (j) => !!j.handoff_id && j.role === 'ci' && (j.frozen_files || []).length === 1 && !!j.wake && j.wake.role === 'qa' && j.wake.agent === 'qa');
    e6('[6] role=qa 的卡仍被拒（验证方不能自交自测）', ['handoff', '--card', 'CQA'], 1, has('CARD_NOT_WORKER_TASK CQA'));
  } finally { rmSync(R6, { recursive: true, force: true }); }

  // ----- 场景 7：release 脏检查的路径解析（真 bug 回归：git() trim 掉首条记录前导状态空格） -----
  // 事故形态：porcelain 原始行 " M .workforce/…/ci.json" 被 git() 整体 trim 成 "M .workforce/…/ci.json"，
  // 旧实现固定 slice(3) ⇒ "workforce/…"（开头点被吃掉）⇒ ci.json / loop-state.json 的豁免与
  // otherOk 的范围判断对「已跟踪且被修改」的文件全部失配（release 卡 WORKTREE_DIRTY）。
  const R7 = mkdtempSync(join(tmpdir(), 'awl7-'));
  try {
    const g7 = (...a) => execFileSync('git', a, { cwd: R7, stdio: 'pipe' }).toString().trim();
    const c7 = (email, msg) => { g7('add', '-A'); g7('-c', `user.email=${email}`, '-c', 'user.name=x', 'commit', '-q', '--allow-empty', '-m', msg); };
    const w7 = (p, t) => { mkdirSync(dirname(join(R7, p)), { recursive: true }); writeFileSync(join(R7, p), t); };
    g7('init', '-q');
    const STATE = '.workforce/loop-state.json';
    const CIF = '.workforce/evidence/B-SKILL-MERGE-GATE/ci.json';
    w7(STATE, '{"state":"EXECUTING"}');
    w7(CIF, '{"verdict":"PASS","bound":"HEAD"}');
    c7(USER, 'setup7');
    const LIB7 = await import(pathToFileURL(join(S, 'lib.mjs')).href);
    const P7 = LIB7.paths(R7, {});
    const b7 = { id: 'B-SKILL-MERGE-GATE', status: 'accepted', tasks: [] };
    const st7 = { batches: [b7] };
    // 用例 1：仅「已跟踪且被修改」的 ci.json / loop-state.json（本次事故形态）必须被豁免。
    w7(CIF, '{"verdict":"PASS","bound":"HEAD-2"}');
    w7(STATE, '{"state":"EXECUTING","rev":2}');
    const d1 = LIB7.releaseDirtyFiles(P7, st7, b7);
    n++;
    if (Array.isArray(d1) && d1.length === 0) console.log('✓ [7] release 脏检查：已跟踪被修改的 ci.json / loop-state.json 正确豁免（旧 slice(3) 会误报 WORKTREE_DIRTY）');
    else { fails++; console.log('✗ [7] ci.json/state 豁免失效（旧实现 slice(3) 的症状）', JSON.stringify(d1), 'old_slice3=' + JSON.stringify([' M ' + CIF, ' M ' + STATE].map(l => l.slice(3)))); }
    // 用例 2：真实脏文件仍被拦下 ⇒ 「必须干净」的语义没有被放宽；解析保留路径首字符（含前导点）。
    w7('other/junk.txt', 'x');
    const d2 = LIB7.releaseDirtyFiles(P7, st7, b7);
    n++;
    const junkKept = Array.isArray(d2) && d2.includes('other/junk.txt');
    if (junkKept && d2.length === 1) console.log('✓ [7] release 脏检查：未被豁免的真实脏文件仍被拦下，判据未放宽');
    else { fails++; console.log('✗ [7] 真实脏文件未被拦下或豁免被放宽', JSON.stringify(d2)); }
  } finally { rmSync(R7, { recursive: true, force: true }); }

  // ----- 场景 8：回执 artifacts 里的「同批兄弟回执」（结构性 sha 不稳定） -----
  // 真缺陷：回执链顺序产生（dev→qa→gate→ledger→ci），dev 交卷时后续回执还不存在/返工后必变，
  // 让回执钉兄弟回执 sha = 要求预见未来 sha ⇒ 每次返工必 ARTIFACT_SHA_MISMATCH。
  // 修法：同批回执 artifact 跳过存在性/sha 比对（NOTE RECEIPT_ARTIFACT_SKIPPED），其它判据一条不放宽。
  const R8 = mkdtempSync(join(tmpdir(), 'awl8-'));
  try {
    const g8 = (...a) => execFileSync('git', a, { cwd: R8, stdio: 'pipe' }).toString().trim();
    const c8 = (email, msg) => { g8('add', '-A'); g8('-c', `user.email=${email}`, '-c', 'user.name=x', 'commit', '-q', '--allow-empty', '-m', msg); };
    const w8 = (p, t) => { mkdirSync(dirname(join(R8, p)), { recursive: true }); writeFileSync(join(R8, p), t); };
    const L8 = await import(pathToFileURL(join(S, 'lib.mjs')).href);
    g8('init', '-q');
    w8('.workforce/config.json', JSON.stringify({ schema: 'agent-workforce-config/v1', business_dirs: ['lib/'], roles: { design: false },
      gates: { required_commands: ['t'], require_no_mock: true }, ledger: { none_reason: 'demo' },
      trust: { anchor: 'none', none_reason: 'demo' }, ci: { provider: 'github', repo: 'x/y' } }));
    w8('lib/a.txt', 'a'); w8('out/t.txt', 'ok');
    c8(USER, 'base8');
    w8('.workforce/signoffs/architect.json', JSON.stringify({ role: 'architect', agent: 'architect-a', spec_sha256: 'spec-8', agreed: true }));
    const P8 = L8.paths(R8, {});
    const card8 = { card_id: 'C1', phase: 'PH', owner_agent: 'dev-c1', qa_agent: 'qa-c1', write_scopes: ['lib/'], acceptance_criteria: ['x'], independent_test: 't', blocked_by: [], checkpoint: 'cp' };
    const batch8 = { id: 'B-8', status: 'dispatched', tasks: [{ id: 'T1', role: 'dev', card_id: 'C1', card: card8 }], handoffs: {} };
    const st8 = { spec_sha256: 'spec-8', batches: [batch8] };
    const sha8 = (p) => L8.shaFile(join(R8, p));
    const ev8 = (role, agent, extra = {}) => ({ schema: 'agent-workforce-evidence/v1', role, agent, card_id: 'C1', batch_id: 'B-8', spec_sha256: 'spec-8',
      artifacts: [{ path: 'out/t.txt', sha256: sha8('out/t.txt') }], command_exit_codes: [{ cmd: 't', exit_code: 0 }], verdict: 'PASS',
      independent: true, declared_by_lead: false, written_by: role + '-fixture', ...extra });
    // 兄弟回执先落盘（模拟 qa 已存在、且后续会被返工重写）
    const qaRel = '.workforce/evidence/B-8/qa-C1.json';
    w8(qaRel, JSON.stringify(ev8('qa', 'qa-c1', { layers_covered: ['event', 'logic', 'interface'], rule_ids: ['R-1'], handoff_id: 'h1' }), null, 2));
    // dev 回执 artifacts：正常产物 + 同批兄弟回执（qa 回执、自钉、尚不存在的 ci.json），sha 全部故意写错
    const wrong = 'deadbeef'.repeat(8);
    const devBase = ev8('dev', 'dev-c1', { files_changed: ['lib/a.txt'], artifacts: [
      { path: 'out/t.txt', sha256: sha8('out/t.txt') },
      { path: qaRel, sha256: wrong },
      { path: '.workforce/evidence/B-8/dev-C1.json', sha256: wrong },
      { path: '.workforce/evidence/B-8/ci.json', sha256: wrong },
    ] });
    w8('.workforce/evidence/B-8/dev-C1.json', JSON.stringify(devBase, null, 2));
    w8('.workforce/evidence/B-8/gate.json', JSON.stringify(ev8('gate', 'gate-1', { card_id: null, mock_used: false }), null, 2));
    w8('.workforce/evidence/B-8/ledger.json', JSON.stringify(ev8('ledger', 'ledger-1', { card_id: null }), null, 2));
    const refreshHandoff = () => { batch8.handoffs = { C1: { handoff_id: 'h1', receipt_sha256: sha8('.workforce/evidence/B-8/dev-C1.json'), files: { 'lib/a.txt': sha8('lib/a.txt') } } }; };
    refreshHandoff();
    const r8a = L8.checkEvidence(P8, st8, batch8);
    n++;
    const skipNotes = (r8a.notes || []).filter(x => x.includes('RECEIPT_ARTIFACT_SKIPPED'));
    const noteOk = skipNotes.some(x => x.includes('qa-C1.json')) && skipNotes.some(x => x.includes('ci.json'));
    if (r8a.reasons.length === 0 && noteOk) console.log('✓ [8] 回执 artifacts 钉同批兄弟回执（含 sha 故意写错/尚不存在）→ PASS + NOTE RECEIPT_ARTIFACT_SKIPPED（旧实现会 ARTIFACT_SHA_MISMATCH/MISSING）');
    else { fails++; console.log('✗ [8] 同批兄弟回执应被跳过并给 NOTE', JSON.stringify({ reasons: r8a.reasons, notes: r8a.notes })); }
    // 反向：非回执文件的 sha 写错 ⇒ 必须 FAIL（证明判据没被放宽）
    const devBad = JSON.parse(JSON.stringify(devBase));
    devBad.artifacts = [{ path: 'out/t.txt', sha256: wrong }];
    w8('.workforce/evidence/B-8/dev-C1.json', JSON.stringify(devBad, null, 2));
    refreshHandoff();
    const r8b = L8.checkEvidence(P8, st8, batch8);
    n++;
    const shaBad = r8b.reasons.some(x => x.includes('ARTIFACT_SHA_MISMATCH') && x.includes('out/t.txt'));
    if (shaBad) console.log('✓ [8] 非回执文件 sha 写错 → FAIL ARTIFACT_SHA_MISMATCH（未放宽；skip 仅限同批回执命名）');
    else { fails++; console.log('✗ [8] 非回执文件的 sha 错误未被拦下', JSON.stringify(r8b.reasons)); }
  } finally { rmSync(R8, { recursive: true, force: true }); }

  // ----- 场景 9：判据可判性（INC-005/006 的守卫；check-criteria.mjs 三条规则） -----
  const R9 = mkdtempSync(join(tmpdir(), 'awl9-'));
  try {
    const w9 = (p, t) => { mkdirSync(dirname(join(R9, p)), { recursive: true }); writeFileSync(join(R9, p), t); };
    const runCrit = (card) => {
      w9('.workforce/cards/C-T1.json', JSON.stringify({ schema: 'agent-workforce-card/v1', card_id: 'C-T1', acceptance_criteria: card }));
      try { return { code: 0, j: JSON.parse(execFileSync(process.execPath, [join(SKILL_DIR, 'scripts', 'check-criteria.mjs'), '--repo', R9], { encoding: 'utf8', stdio: 'pipe' })) }; }
      catch (e) { let j = {}; try { j = JSON.parse(e.stdout.toString()); } catch {} return { code: e.status, j }; }
    };
    const hasReason = (r, c) => (r.j.reasons || []).some(x => String(x).includes(c));
    // 用例 1：抽样歧义
    const r1a = runCrit(['P95 ≤ 20ms（取最快一次）']);
    const r1b = runCrit(['P95 ≤ 20ms（min-of-5，N ≥ 5 轮）']);
    n++;
    if (r1a.code === 1 && hasReason(r1a, 'CRITERIA_SAMPLING_AMBIGUOUS') && r1b.code === 0) console.log('✓ [9] 判据抽样歧义：P95 无取样口径 → 红；min-of-N+轮次 → 绿');
    else { fails++; console.log('✗ [9] 抽样歧义规则未按预期验红/验绿', JSON.stringify({ red: r1a, green: r1b }).slice(0, 500)); }
    // 用例 2：绝对数量断言
    const r2a = runCrit(['必须恰好 3 个 job 全部 success']);
    const r2b = runCrit(['恰好 3 个 job 全部 success（docs-only policy 结构性跳过，跳过项逐项说明）']);
    n++;
    if (r2a.code === 1 && hasReason(r2a, 'CRITERIA_ABSOLUTE_COUNT_UNVERIFIABLE') && r2b.code === 0) console.log('✓ [9] 绝对数量断言：无结构性跳过说明 → 红；带跳过说明 → 绿');
    else { fails++; console.log('✗ [9] 绝对数量规则未按预期验红/验绿', JSON.stringify({ red: r2a, green: r2b }).slice(0, 500)); }
    // 用例 3：基线未钉
    const r3a = runCrit(['失败数不劣于基线']);
    const r3b = runCrit(['失败数不劣于基线（median-of-5 聚合） .workforce/evidence/B-THINK-LOSS/skill-selftest.txt（sha256=eb0375299390f1798fcdcbd7ad08b21e7a994d9b2b13ae107fb6c9f7393c6312）']);
    n++;
    if (r3a.code === 1 && hasReason(r3a, 'CRITERIA_NO_BASELINE_PIN') && r3b.code === 0) console.log('✓ [9] 基线未钉：不劣于基线无路径/sha → 红；带基线文件+sha → 绿');
    else { fails++; console.log('✗ [9] 基线钉规则未按预期验红/验绿', JSON.stringify({ red: r3a, green: r3b }).slice(0, 500)); }
    // 用例 4：INC-013 —— 判据点名调用点/入口，但 write_scopes 未覆盖该 .rs ⇒ 红；覆盖 ⇒ 绿
    const runCritCard = (card) => {
      w9('.workforce/cards/C-T2.json', JSON.stringify(card));
      try { return { code: 0, j: JSON.parse(execFileSync(process.execPath, [join(SKILL_DIR, 'scripts', 'check-criteria.mjs'), '--repo', R9], { encoding: 'utf8', stdio: 'pipe' })) }; }
      catch (e) { let j = {}; try { j = JSON.parse(e.stdout.toString()); } catch {} return { code: e.status, j }; }
    };
    const CS_TEXT = '按判据必须改调用方 crates/a/mod.rs（timeBeginPeriod 的 RAII guard 由 mod.rs 持有）';
    const r4a = runCritCard({ card_id: 'C-T2', write_scopes: ['crates/a/other/'], acceptance_criteria: [CS_TEXT] });
    const r4b = runCritCard({ card_id: 'C-T2', write_scopes: ['crates/a/'], acceptance_criteria: [CS_TEXT] });
    n++;
    if (r4a.code === 1 && hasReason(r4a, 'CRITERIA_SCOPE_GAP') && JSON.stringify(r4a.j.reasons).includes('crates/a/mod.rs') && r4b.code === 0) console.log('✓ [9] 判据点名调用点但 scope 未覆盖 → 红；scope 覆盖 → 绿');
    else { fails++; console.log('✗ [9] scope 覆盖规则未按预期验红/验绿', JSON.stringify({ red: r4a, green: r4b }).slice(0, 500)); }
    // 用例 5：INC-014 —— 相对比值判据未明示聚合 ⇒ 红；明示 median-of-5 双侧 ⇒ 绿
    const r5a = runCritCard({ card_id: 'C-T3', write_scopes: ['crates/a/'], acceptance_criteria: ['单帧 CPU 相对基线 ≤ 1.5X'] });
    const r5b = runCritCard({ card_id: 'C-T3', write_scopes: ['crates/a/'], acceptance_criteria: ['单帧 CPU 相对基线 ≤ 1.5X（median-of-5，双侧比较）'] });
    n++;
    if (r5a.code === 1 && hasReason(r5a, 'CRITERIA_AGGREGATION_AMBIGUOUS') && JSON.stringify(r5a.j.reasons).includes('C-T3') && r5b.code === 0) console.log('✓ [9] 相对比值判据未明示聚合 → 红；明示 median-of-5 双侧 → 绿');
    else { fails++; console.log('✗ [9] 聚合口径规则未按预期验红/验绿', JSON.stringify({ red: r5a, green: r5b }).slice(0, 500)); }
  } finally { rmSync(R9, { recursive: true, force: true }); }

  // ----- 场景 10：失误台账守卫（INC-001..009 的 check-incidents.mjs 规则逐条验红） -----
  const T10 = mkdtempSync(join(tmpdir(), 'awl10-')); const S10 = join(T10, 'agent-workforce-loop');
  try {
    cpSync(SKILL_DIR, S10, { recursive: true });
    const incPath = join(S10, 'incidents.json');
    const genPath = join(S10, 'references/generated/incidents.md');
    const origInc = readFileSync(incPath, 'utf8');
    const origGen = readFileSync(genPath, 'utf8');
    const runInc = () => {
      try { return { code: 0, j: JSON.parse(execFileSync(process.execPath, [join(S10, 'scripts', 'check-incidents.mjs')], { encoding: 'utf8', stdio: 'pipe' })) }; }
      catch (e) { let j = {}; try { j = JSON.parse(e.stdout.toString()); } catch {} return { code: e.status, j }; }
    };
    const withInc = (mutate) => {
      const j = JSON.parse(origInc);
      mutate(j);
      writeFileSync(incPath, JSON.stringify(j, null, 2) + '\n');
      const r = runInc();
      writeFileSync(incPath, origInc);
      return r;
    };
    const incHas = (r, c) => (r.j.reasons || []).some(x => String(x).includes(c));
    const incCase = (name, cond, detail) => { n++; if (cond) console.log('✓ ' + name); else { fails++; console.log('✗ ' + name, JSON.stringify(detail || '').slice(0, 500)); } };
    const base = runInc();
    incCase('[10] incidents 基线：14 条 / 18 guard 全验通过 → PASS', base.code === 0 && base.j.incidents === 14 && base.j.guards === 18, base);
    incCase('[10] 守卫 code 在 scripts 里不存在 → INCIDENT_GUARD_CODE_UNKNOWN',
      incHas(withInc(j => { j.incidents[0].guard[0].code = 'BOGUS_CODE_XYZ'; }), 'INCIDENT_GUARD_CODE_UNKNOWN'));
    incCase('[10] 守卫 selftest 用例名找不到 → INCIDENT_SELFTEST_MISSING',
      incHas(withInc(j => { j.incidents[0].guard[0].selftest = ['NOT', 'A', 'REAL', 'CASE', Math.random().toString(36).slice(2)].join('_'); }), 'INCIDENT_SELFTEST_MISSING'));
    incCase('[10] 只记录不设防（无 guard）→ INCIDENT_WITHOUT_ENFORCEMENT',
      incHas(withInc(j => { j.incidents[0].guard = []; }), 'INCIDENT_WITHOUT_ENFORCEMENT'));
    incCase('[10] accepted_risk 的 review_by 过期 → INCIDENT_RISK_REVIEW_EXPIRED',
      incHas(withInc(j => { j.incidents[0].status = 'accepted_risk'; j.incidents[0].accepted_risk = { rationale: 'demo', review_by: '2000-01-01' }; }), 'INCIDENT_RISK_REVIEW_EXPIRED'));
    incCase('[10] status 不在白名单 → INCIDENT_STATUS_INVALID',
      incHas(withInc(j => { j.incidents[0].status = 'bogus_status'; }), 'INCIDENT_STATUS_INVALID'));
    incCase('[10] kind 不在白名单 → INCIDENT_KIND_UNKNOWN',
      incHas(withInc(j => { j.incidents[0].kind = 'BOGUS_KIND'; }), 'INCIDENT_KIND_UNKNOWN'));
    writeFileSync(genPath, origGen + '\n手改一行\n');
    let drift;
    try { drift = runInc(); } finally { writeFileSync(genPath, origGen); }
    incCase('[10] 生成物被手改 → INCIDENTS_DOC_DRIFT', incHas(drift, 'INCIDENTS_DOC_DRIFT'), drift);
  } finally { rmSync(T10, { recursive: true, force: true }); }

  // ----- 场景 11：合并门的真实时机可用 + 不可绕过（INC-010 / INC-011） -----
  const R11 = mkdtempSync(join(tmpdir(), 'awl11-'));
  try {
    const g11 = (...a) => execFileSync('git', a, { cwd: R11, stdio: 'pipe' }).toString().trim();
    const w11 = (p, t) => { mkdirSync(dirname(join(R11, p)), { recursive: true }); writeFileSync(join(R11, p), t); };
    const L11 = await import(pathToFileURL(join(S, 'lib.mjs')).href);
    g11('init', '-q');
    w11('.workforce/config.json', JSON.stringify({ schema: 'agent-workforce-config/v1', business_dirs: ['lib/'], roles: { design: false },
      gates: { required_commands: ['t'], require_no_mock: true }, ledger: { none_reason: 'demo' },
      trust: { anchor: 'none', none_reason: 'demo' }, ci: { provider: 'github', repo: 'x/y' } }));
    const P11 = L11.paths(R11, {});
    L11.saveState(P11, { state: 'EXECUTING', batches: [{ id: 'B-DEL', phase: 'PH', status: 'delivered', tasks: [], handoffs: {} }] });
    // 用例 1：delivered 批次跑 premerge 必须进入校验逻辑（不再被 BATCH_STATE_FORBIDS 拦在门外）
    const pre = run('loop.mjs', ['premerge', '--batch', 'B-DEL'], R11);
    n++;
    const preJson = JSON.stringify(pre.j);
    if (pre.code === 1 && !preJson.includes('BATCH_STATE_FORBIDS') && /CI_RECEIPT|CI_|MERGE_/.test(preJson)) console.log('✓ [11] delivered 批次跑 premerge 应进入校验逻辑而非 BATCH_STATE_FORBIDS');
    else { fails++; console.log('✗ [11] delivered 批次 premerge 仍未进入校验逻辑', pre.code, preJson.slice(0, 400)); }
    // 用例 2：premerge FAIL ⇒ merge-pr.mjs exit 1 且不调用任何合并 API（merge_called=false）
    let mp;
    try {
      const out = execFileSync(process.execPath, [join(SKILL_DIR, 'scripts', 'merge-pr.mjs'), '--batch', 'B-DEL', '--pr', '99', '--repo', 'x/y', '--method', 'squash', '--root', R11, '--allow-bootstrap'], { encoding: 'utf8', stdio: 'pipe' });
      mp = { code: 0, j: JSON.parse(out) };
    } catch (e) { let j = {}; try { j = JSON.parse(e.stdout.toString()); } catch {} mp = { code: e.status, j }; }
    n++;
    if (mp.code === 1 && mp.j.reason === 'MERGE_PR_REFUSED' && mp.j.merge_called === false && mp.j.premerge && mp.j.premerge.ok === false) console.log('✓ [11] merge-pr：premerge FAIL 时 exit 1 且不调用任何合并 API（merge_called=false）');
    else { fails++; console.log('✗ [11] merge-pr FAIL 分支未按预期拒绝/仍可能合并', mp.code, JSON.stringify(mp.j).slice(0, 400)); }
  } finally { rmSync(R11, { recursive: true, force: true }); }

  // ----- 场景 12：门自身引导合并（INC-012）+ 合并机器记录 -----
  const R12 = mkdtempSync(join(tmpdir(), 'awl12-'));
  try {
    const g12 = (...a) => execFileSync('git', a, { cwd: R12, stdio: 'pipe' }).toString().trim();
    const w12 = (p, t) => { mkdirSync(dirname(join(R12, p)), { recursive: true }); writeFileSync(join(R12, p), t); };
    const L12 = await import(pathToFileURL(join(S, 'lib.mjs')).href);
    g12('init', '-q');
    w12('.workforce/config.json', JSON.stringify({ schema: 'agent-workforce-config/v1', business_dirs: ['lib/'], roles: { design: false },
      gates: { required_commands: ['t'], require_no_mock: true }, ledger: { none_reason: 'demo' },
      trust: { anchor: 'none', none_reason: 'demo' }, ci: { provider: 'github', repo: 'x/y' } }));
    const P12 = L12.paths(R12, {});
    L12.saveState(P12, { state: 'EXECUTING', batches: [{ id: 'B-BS', phase: 'PH', status: 'delivered', tasks: [], handoffs: {} }] });
    const runMP = (extra = []) => {
      try {
        const out = execFileSync(process.execPath, [join(SKILL_DIR, 'scripts', 'merge-pr.mjs'), '--batch', 'B-BS', '--pr', '7', '--repo', 'x/y', '--method', 'squash', '--root', R12, ...extra], { encoding: 'utf8', stdio: 'pipe' });
        return { code: 0, out, err: '' };
      } catch (e) { return { code: e.status, out: e.stdout ? e.stdout.toString() : '', err: e.stderr ? e.stderr.toString() : '' }; }
    };
    // 用例 1：目标 worktree 无 merge-pr.mjs 且未给 --allow-bootstrap ⇒ exit 1（GATE_BOOTSTRAP_REQUIRED）
    const bs1 = runMP();
    n++;
    const j1 = JSON.parse(bs1.out || '{}');
    if (bs1.code === 1 && j1.reason === 'GATE_BOOTSTRAP_REQUIRED' && j1.merge_called === false && /GATE_BOOTSTRAP_MODE/.test(bs1.err)) console.log('✓ [12] merge-pr 引导自检：无 merge-pr.mjs 且未给 --allow-bootstrap ⇒ exit 1（GATE_BOOTSTRAP_REQUIRED）');
    else { fails++; console.log('✗ [12] 引导自检未按预期拒绝', bs1.code, bs1.err.slice(0, 200), bs1.out.slice(0, 300)); }
    // 用例 2：给了 --allow-bootstrap + --dry-run ⇒ 打 GATE_BOOTSTRAP_MODE 并继续到 premerge（FAIL 不合并）
    const bs2 = runMP(['--allow-bootstrap', '--dry-run']);
    n++;
    const j2 = JSON.parse(bs2.out || '{}');
    if (bs2.code === 1 && /GATE_BOOTSTRAP_MODE/.test(bs2.err) && j2.reason === 'MERGE_PR_REFUSED' && j2.bootstrap_used === true && j2.merge_called === false && j2.premerge && j2.premerge.ok === false) console.log('✓ [12] --allow-bootstrap：打印 GATE_BOOTSTRAP_MODE 后继续到门校验（premerge FAIL 仍不合并）');
    else { fails++; console.log('✗ [12] 引导模式未按预期「留痕并继续」', bs2.code, bs2.err.slice(0, 200), bs2.out.slice(0, 300)); }
    // 用例 3：合并机器记录 merge.json 字段（writeMergeRecord）
    const recPath = L12.writeMergeRecord(P12, 'B-BS', { pr: 7, merge_commit: 'abc123', merge_called: true, head_sha: 'deadbeef', checks: [{ name: 'ci-gate', conclusion: 'success' }], bootstrap_used: true });
    n++;
    let rec = null; try { rec = JSON.parse(readFileSync(recPath, 'utf8')); } catch {}
    if (rec && typeof rec.at === 'string' && rec.batch === 'B-BS' && rec.pr === 7 && rec.merge_commit === 'abc123' && rec.merge_called === true && rec.head_sha === 'deadbeef' && Array.isArray(rec.checks) && rec.checks.length === 1 && rec.bootstrap_used === true) console.log('✓ [12] 合并后自动写 evidence/<batch>/merge.json（{at,pr,merge_commit,merge_called:true,head_sha,checks,bootstrap_used}）');
    else { fails++; console.log('✗ [12] merge.json 记录字段不对', JSON.stringify(rec)); }
  } finally { rmSync(R12, { recursive: true, force: true }); }
} finally {
  rmSync(R, { recursive: true, force: true });
  rmSync(BIN, { recursive: true, force: true });
  rmSync(ORIGIN, { recursive: true, force: true });
}
console.log(JSON.stringify({ verdict: fails ? 'FAIL' : 'PASS', cases: n, failed: fails }));
process.exit(fails ? 1 : 0);




