#!/usr/bin/env node
/**
 * agent-workforce-loop v1.1 状态机（与 agent harness 无关）。exit 0 = 允许并已迁移；exit 1 = 拒绝，状态不变。
 * 每个迁移都在内部跑对应判据（不是只改状态名）：
 *   spec-review  → 各工种签署文件齐、绑定当前 spec_sha256、agent 不复用
 *   spec-approve → lead 签署绑定当前 spec
 *   spec-confirm → 锁定 config.json 的 sha（之后改即 CONFIG_DRIFT）；acceptance.user_confirm=true 时还要用户签署
 *   init         → 从「已确认的 spec」展开批次，逐张校验任务卡
 *   dispatch     → 每个批次独立状态；blocked_by 已交付、本批与同时在做的批次 write_scopes / 卡 / agent 都不交叉，按 parallel.max_batches 并行
 *   handoff      → 开发合格交卷后冻结代码（记录文件 sha），才给出测试的 brief；交接后开发再改，测试作废
 *   evidence     → 回执齐、独立、sha256 对得上实物、业务改动全部被 dev 回执认领且在 write_scopes 内
 *   accept       → 五层验收；FAIL 自动 REOPEN
 *   release      → overall 文件为 true（若配置）、工作区干净、CI（github / command / none，按 config）
 *   user-issue   → issue 文件由验收者亲自提交，回流到该卡所在批次
 * 状态文件带 self_sha256，被手改即拒绝一切迁移。
 */
import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { join, dirname, resolve, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import * as L from './lib.mjs';
import * as M from './machine.mjs';

const SKILL = resolve(join(dirname(fileURLToPath(import.meta.url)), '..'));
const { pos, flags } = L.parseArgs(process.argv.slice(2));
const cmd = pos[0] || 'status';
const repo = resolve(typeof flags.repo === 'string' ? flags.repo : process.cwd());
const P = L.paths(repo, flags);
const CF = L.loadConfig(P);
const as = typeof flags.as === 'string' ? flags.as : 'lead';
const { out } = L;

const { ALLOWED, BATCH_ALLOWED } = M;
const ARCHITECT_ONLY = new Set(M.ARCHITECT_ONLY);
// 工种动作：任何人都不能通过 loop.mjs 代做，Lead 更不行
const ROLE_ACTIONS = new Set(['implement', 'test', 'gate', 'ledger', 'design-verify', 'write-spec', 'sign', 'write-evidence']);
const READONLY = new Set(M.READONLY);

function load() {
  const s = L.readState(P);
  if (s && s.__error) out({ verdict: 'FAIL', reason: 'STATE_INVALID_JSON' }, 1);
  if (s && L.stateTampered(s)) out({ verdict: 'FAIL', reason: 'STATE_TAMPERED：loop-state.json 只能由 loop.mjs 写' }, 1);
  return s;
}
function specDrift(s) {
  if (!s?.spec_sha256) return null;
  const p = s.spec_path ? resolve(repo, s.spec_path) : P.spec;
  if (!existsSync(p)) return 'SPEC_FILE_GONE';
  return L.shaFile(p) !== s.spec_sha256 ? 'SPEC_DRIFT：spec 在确认后被改动，必须走 spec-revise 重新审批' : null;
}
function require(s, action) {
  const st = s?.state || 'IDLE';
  if (!(ALLOWED[st] || []).includes(action)) out({ verdict: 'FAIL', reason: `STATE_FORBIDS state=${st} action=${action} allowed=${(ALLOWED[st] || []).join(',')}` }, 1);
  if (ARCHITECT_ONLY.has(action) && !/^architect-/.test(as)) out({ verdict: 'FAIL', reason: 'ARCHITECT_ONLY：' + action + ' 须由 architect-* 以 --as architect-<name> 执行' }, 1);
  if (CF.problems.length) out({ verdict: 'FAIL', reason: 'CONFIG_INVALID', reasons: CF.problems }, 1);
  if (s?.approver_sha256) { const ap = L.readApproverPub(P); if (!ap.ok || ap.sha !== s.approver_sha256) out({ verdict: 'FAIL', reason: 'APPROVER_KEY_CHANGED：开工后审批人公钥被换或删除，一切迁移拒绝（只有你本人能处理）' }, 1); }
  // **死锁修复（2026-11-19）**：CONFIG_DRIFT 的唯一解药就是 `spec-revise`（改完重走审批）。
  // 旧写法对该动作本身也判红 ⇒ 一旦 config 在确认后被改，所有动作（含修复通道）全被拒 = 死锁。
  // 只放行 `spec-revise` 这一条修复通道：改完仍需 spec-approve/spec-confirm 重新锁 sha，
  // 判据强度不变（其它动作照样被 CONFIG_DRIFT 拒）。
  // 诊断（不改判据）：若纯属 CRLF 物化造成的假漂移，附上可读提示，免得人以为规则表真被改了。
  if (action !== 'spec-revise' && s?.config_sha256 && CF.sha256 !== s.config_sha256) {
    const why = L.crlfNormalizedNote(P.config, s.config_sha256);
    out({ verdict: 'FAIL', reason: 'CONFIG_DRIFT：config.json 在用户确认后被改动，须架构师 spec-revise 并重走审批',
      ...(why ? { crlf_hint: why, crlf_fix: 'git config --local core.autocrlf false 后按 LF 原始字节重写该文件（git cat-file -p HEAD:<path>）' } : {}) }, 1);
  }
  if (!ARCHITECT_ONLY.has(action)) { const d = specDrift(s); if (d) out({ verdict: 'FAIL', reason: d }, 1); }
  return st;
}
const rel = (p) => L.norm(relative(repo, p));

// ================= 只读命令 =================
if (cmd === 'validate-skill' || cmd === 'self-test') {
  const r = await import('node:child_process');
  try { r.execFileSync(process.execPath, [join(SKILL, 'scripts', 'validate-skill.mjs')], { stdio: 'inherit' }); process.exit(0); }
  catch { process.exit(1); }
}

if (cmd === 'scaffold') {
  const { mkdirSync, copyFileSync } = await import('node:fs');
  const made = [];
  mkdirSync(P.W, { recursive: true });
  for (const [src, dst] of [['config.template.json', P.config], ['acceptors.template.json', P.acceptors]]) {
    if (!existsSync(dst)) { copyFileSync(join(SKILL, 'assets', src), dst); made.push(rel(dst)); }
  }
  for (const d of [P.cards, P.signoffs, P.evidence, P.issues]) mkdirSync(d, { recursive: true });
  out({ verdict: 'PASS', created: made, dir: P.dirRel,
    next: '执行侧按项目填好 config.json（规则表）交用户本地审核；用户本人在自己终端跑 approve.mjs setup（第一次）和 approve.mjs config 留下签名同意，执行侧提交。之后 config-check 应 PASS' }, 0);
}

/** 永久提示：降级配置必须对用户可见 */
function warnings() {
  const w = []; const c = CF.c;
  if (c.trust.anchor === 'none') w.push('TRUST_NONE：验收者身份只靠 git 作者邮箱，agent 可抢先自写自提交 acceptors.json（首次占位）。理由：' + c.trust.none_reason);
  if (!c.ledger.command) w.push('LEDGER_UNVERIFIED：台账层只验 self_sha256 自洽，不重跑生成命令，手写台账可过。理由：' + c.ledger.none_reason);
  if (c.ci.provider === 'none') w.push('CI_DISABLED：' + c.ci.none_reason);
  // 只报警不改判据：Windows CRLF 物化会让「按原始字节算的 sha」与 LF 锁不一致，制造 CONFIG_DRIFT 假红
  if (CF.crlfNote) w.push(CF.crlfNote);
  if (CF.autocrlfRisk) w.push(CF.autocrlfRisk);
  return w;
}
if (cmd === 'config-check') {
  const reasons = [...CF.problems];
  let ap = null;
  if (CF.present && CF.c.trust.anchor === 'local_review') { ap = L.verifyApproval(P, 'config', rel(P.config), CF.sha256); if (!ap.ok) reasons.push('CONFIG_' + ap.reason); }
  else if (CF.present) { const c = L.userCommitted(P, P.config); if (!c.ok) reasons.push('CONFIG_' + c.reason); }
  out({ verdict: reasons.length ? 'FAIL' : 'PASS', dir: P.dirRel, config: CF.c, roles: L.workerRoles(P), reasons, warnings: warnings(),
    ...(ap?.ok ? { user_approval: { file: ap.file, approved_at: ap.approved_at, approver_fingerprint: ap.fingerprint, note: '请用户核对指纹，和自己设置审批密码时屏幕上显示的一致' } } : {}) }, reasons.length ? 1 : 0);
}

if (cmd === 'status') {
  const s = load();
  if (!s) out({ verdict: 'PASS', state: 'IDLE', allowed_actions: ALLOWED.IDLE, dir: P.dirRel, config_problems: CF.problems, warnings: warnings(),
    note: CF.problems.length ? '先 scaffold → 用户填写并提交 config.json / acceptors.json → config-check' : '由 architect-* 产出 spec 后执行 architect-spec --as architect-<name>', skill_dir: SKILL }, 0);
  out({ verdict: 'PASS', state: s.state, active_batches: L.activeBatches(s).map(x => `${x.id}:${x.status}`), ready_to_dispatch: readyBatches(s).map(x => x.id), spec_sha256: s.spec_sha256 || null,
    allowed_actions: ALLOWED[s.state] || [], spec_drift: specDrift(s), config_problems: CF.problems,
    config_drift: s.config_sha256 && CF.sha256 !== s.config_sha256 ? 'CONFIG_DRIFT' : null, warnings: warnings(),
    batches: (s.batches || []).map(x => `${x.id}:${x.status}`), handoffs: Object.fromEntries((s.batches || []).filter(x => x.handoffs).map(x => [x.id, Object.keys(x.handoffs)])),
    open_issues: (s.issues || []).filter(x => x.status === 'open').map(x => x.issue_id),
    last_reasons: s.last_reasons || [] }, 0);
}

if (cmd === 'guard') {
  const s = load();
  const act = typeof flags.action === 'string' ? flags.action : '';
  const st = s?.state || 'IDLE';
  if (ROLE_ACTIONS.has(act)) out({ verdict: 'REJECT', reason: `ROLE_WORK：${act} 必须由独立工种 agent 完成，Lead 只调度与验收` }, 1);
  if (ARCHITECT_ONLY.has(act) && !/^architect-/.test(as)) out({ verdict: 'REJECT', reason: `ARCHITECT_ONLY：${as} 不能执行 ${act}（Lead 不写 spec）` }, 1);
  if (!Object.values(ALLOWED).flat().includes(act)) out({ verdict: 'REJECT', reason: 'UNKNOWN_ACTION ' + act }, 1);
  if (!(ALLOWED[st] || []).includes(act)) out({ verdict: 'REJECT', reason: `STATE_FORBIDS state=${st} action=${act} allowed=${(ALLOWED[st] || []).join(',')}` }, 1);
  const d = specDrift(s); if (d && !ARCHITECT_ONLY.has(act)) out({ verdict: 'REJECT', reason: d }, 1);
  if (typeof flags.batch === 'string' && s?.batches) {
    const b = s.batches.find(x => x.id === flags.batch);
    if (!b) out({ verdict: 'REJECT', reason: 'BATCH_UNKNOWN ' + flags.batch }, 1);
    if (BATCH_ALLOWED[b.status] && !['user-issue', 'spec-revise'].includes(act) && !BATCH_ALLOWED[b.status].includes(act)) out({ verdict: 'REJECT', reason: `BATCH_STATE_FORBIDS batch=${b.id} status=${b.status} action=${act}` }, 1);
    if (act === 'dispatch') { const r = [...batchPrereq(s, b), ...L.parallelConflicts(P, s, b)]; if (r.length) out({ verdict: 'REJECT', reason: 'NOT_READY', reasons: r }, 1); }
  }
  out({ verdict: 'PASS', state: st, action: act, as }, 0);
}

if (cmd === 'check-drift') {
  const s = load();
  const act = L.activeBatches(s);
  let r;
  if (!act.length) r = L.checkDrift(P, s, null);
  else {
    // 并行时逐批检查，再查有没有落在所有在做批次范围之外的改动
    r = { reasons: [], changed: [] };
    for (const b of act) { const x = L.checkDrift(P, s, b); r.reasons.push(...x.reasons); r.changed.push(...x.changed); }
    r.reasons = [...new Set(r.reasons)]; r.changed = [...new Set(r.changed)];
  }
  out({ verdict: r.reasons.length ? 'FAIL' : 'PASS', business_files_changed: r.changed.length, files: r.changed.slice(0, 20), reasons: r.reasons }, r.reasons.length ? 1 : 0);
}

// ================= spec 审批链 =================
function readSpec(path) {
  if (!existsSync(path)) out({ verdict: 'FAIL', reason: 'SPEC_MISSING ' + path }, 1);
  const sp = L.readJSON(path);
  const v = L.validateSpecStructure(sp, P);
  if (v.reasons.length) out({ verdict: 'FAIL', reason: 'SPEC_STRUCTURE_INVALID', reasons: v.reasons, layers: v.layers }, 1);
  if (sp.produced_by !== as) out({ verdict: 'FAIL', reason: `SPEC_PRODUCER_MISMATCH：spec.produced_by=${sp.produced_by}，执行者 --as ${as}` }, 1);
  return { sp, sha: L.shaFile(path), layers: v.layers };
}

if (cmd === 'architect-spec' || cmd === 'spec-revise') {
  const s = load();
  require(s, cmd);
  const path = resolve(typeof flags.spec === 'string' ? flags.spec : (s?.spec_path ? resolve(repo, s.spec_path) : P.spec));
  const { sha, layers } = readSpec(path);
  if (cmd === 'spec-revise' && sha === s.spec_sha256) out({ verdict: 'FAIL', reason: 'SPEC_UNCHANGED：修订须实际改动 spec' }, 1);
  const st = { schema: 'agent-workforce-state/v1', state: 'SPEC_DRAFT', spec_path: rel(path), spec_sha256: sha,
    revision: (s?.revision || 0) + (s ? 1 : 0), batches: [], batch: null,
    prior_batches: s?.batches?.length ? s.batches : (s?.prior_batches || []),
    issues: s?.issues || [], deliveries: s?.deliveries || [], history: [...(s?.history || []), { at: new Date().toISOString(), event: cmd, by: as, spec_sha256: sha }] };
  L.saveState(P, st);
  out({ verdict: 'PASS', state: 'SPEC_DRAFT', spec_sha256: sha, layers,
    next: '工种（' + L.workerRoles(P).join('/') + '）各自写 signoffs/<role>.json（含 agent、spec_sha256、agreed:true）后，Lead 跑 spec-review' }, 0);
}

if (cmd === 'spec-review') {
  const s = load(); require(s, cmd);
  const d = specDrift(s); if (d) out({ verdict: 'FAIL', reason: d }, 1);
  const r = L.checkWorkerSignoffs(P, s.spec_sha256);
  if (r.reasons.length) out({ verdict: 'FAIL', reason: 'SPEC_REVIEW_FAILED', reasons: r.reasons, signoffs: r.rows }, 1);
  s.state = 'SPEC_REVIEWED'; s.history.push({ at: new Date().toISOString(), event: 'spec-review' });
  L.saveState(P, s);
  out({ verdict: 'PASS', state: 'SPEC_REVIEWED', next: 'Lead 写 signoffs/lead.json（approved:true，绑定 spec_sha256）后跑 spec-approve' }, 0);
}

if (cmd === 'spec-approve') {
  const s = load(); require(s, cmd);
  const d = specDrift(s); if (d) out({ verdict: 'FAIL', reason: d }, 1);
  const w = L.checkWorkerSignoffs(P, s.spec_sha256);
  if (w.reasons.length) out({ verdict: 'FAIL', reason: 'WORKER_SIGNOFFS_CHANGED', reasons: w.reasons }, 1);
  const l = L.readSignoff(P, 'lead');
  const reasons = [];
  if (!l.present) reasons.push('LEAD_SIGNOFF_MISSING');
  else { if (l.approved !== true) reasons.push('LEAD_NOT_APPROVED'); if (l.spec_sha256 !== s.spec_sha256) reasons.push('LEAD_SIGNOFF_STALE_SPEC'); }
  if (reasons.length) out({ verdict: 'FAIL', reason: 'SPEC_APPROVE_FAILED', reasons }, 1);
  s.state = 'SPEC_APPROVED'; s.history.push({ at: new Date().toISOString(), event: 'spec-approve' });
  L.saveState(P, s);
  out({ verdict: 'PASS', state: 'SPEC_APPROVED', next: CF.c.acceptance.user_confirm && CF.c.trust.anchor === 'local_review' ? '用户本人跑 approve.mjs spec 签名同意开工范围，然后 spec-confirm' : CF.c.acceptance.user_confirm ? '用户写 signoffs/user.json（confirmed:true，绑定 spec_sha256）并提交（过 trust 锚），然后跑 spec-confirm' : 'spec-confirm（acceptance.user_confirm=false：不需要用户确认；锁定 config）' }, 0);
}

if (cmd === 'spec-confirm') {
  const s = load(); require(s, cmd);
  const d = specDrift(s); if (d) out({ verdict: 'FAIL', reason: d }, 1);
  const u = L.readSignoff(P, 'user');
  const reasons = [];
  const needUser = CF.c.acceptance.user_confirm !== false;
  if (!needUser) { /* 用户不做开工确认：spec 由工种签署 + Lead 审批即生效，技术结果全部机器自证 */ }
  else if (CF.c.trust.anchor === 'local_review') { const a = L.verifyApproval(P, 'spec', rel(P.spec), s.spec_sha256); if (!a.ok) reasons.push('SPEC_' + a.reason); }
  else if (!u.present) reasons.push('USER_SIGNOFF_MISSING');
  else {
    if (u.confirmed !== true) reasons.push('USER_NOT_CONFIRMED');
    if (u.spec_sha256 !== s.spec_sha256) reasons.push('USER_SIGNOFF_STALE_SPEC');
    const c = L.userCommitted(P, u.path);
    if (!c.ok) reasons.push('USER_SIGNOFF_' + c.reason);
  }
  if (CF.c.trust.anchor === 'local_review') { const a = L.verifyApproval(P, 'config', rel(P.config), CF.sha256); if (!a.ok) reasons.push('CONFIG_' + a.reason); }
  else { const cc = L.userCommitted(P, P.config); if (!cc.ok) reasons.push('CONFIG_' + cc.reason); }
  if (reasons.length) out({ verdict: 'FAIL', reason: 'SPEC_CONFIRM_FAILED', reasons }, 1);
  s.user_confirm = needUser;
  s.state = 'SPEC_CONFIRMED'; s.acceptors_sha256 = L.acceptorEmails(P).sha; s.config_sha256 = CF.sha256;
  if (CF.c.trust.anchor === 'local_review') { const ap = L.readApproverPub(P); s.approver_sha256 = ap.sha; s.approver_fingerprint = ap.fingerprint; }
  s.history.push({ at: new Date().toISOString(), event: 'spec-confirm' });
  L.saveState(P, s);
  out({ verdict: 'PASS', state: 'SPEC_CONFIRMED', next: 'init：从已确认的 spec 展开批次与任务卡' }, 0);
}

// ================= 执行（批次可并行） =================
function nowISO() { return new Date().toISOString(); }
function deliveredCards(s) { return new Set((s.batches || []).filter(x => x.status === 'delivered').flatMap(x => x.tasks.map(t => t.card_id))); }
/** 批次开工前提：卡未变、blocked_by 已交付、本批内 dev 卡写入范围不交叉 */
function batchPrereq(s, b) {
  const reasons = [];
  const delivered = deliveredCards(s);
  for (const t of b.tasks) {
    const c = L.loadCard(P, t.card_id);
    if (c.error) { reasons.push(c.error); continue; }
    if (c.sha256 !== t.card_sha256) reasons.push('CARD_CHANGED_SINCE_INIT ' + t.card_id);
    for (const x of t.card.blocked_by) if (!delivered.has(x)) reasons.push('BLOCKED ' + t.card_id + ' 等待 ' + x);
  }
  const dev = b.tasks.filter(t => t.role === 'dev');
  for (let i = 0; i < dev.length; i++) for (let j = i + 1; j < dev.length; j++)
    for (const a of dev[i].card.write_scopes) for (const c of dev[j].card.write_scopes)
      if (L.inScopes(a, [c]) || L.inScopes(c, [a])) reasons.push(`WRITE_SCOPE_CONFLICT ${dev[i].card_id}:${a} × ${dev[j].card_id}:${c}`);
  return reasons;
}
/** 现在就能开工（可与在做批次并行）的批次 */
function readyBatches(s) {
  if (!s || s.state !== 'EXECUTING') return [];
  return (s.batches || []).filter(b => ['pending', 'reopened'].includes(b.status) && !batchPrereq(s, b).length && !L.parallelConflicts(P, s, b).length);
}
/** 选批次：--batch 指定；否则在「允许做这个动作」的批次里唯一的那个 */
function pickBatch(s, action) {
  if (typeof flags.batch === 'string') {
    const b = s.batches.find(x => x.id === flags.batch);
    if (!b) out({ verdict: 'FAIL', reason: 'BATCH_UNKNOWN ' + flags.batch }, 1);
    if (!(BATCH_ALLOWED[b.status] || []).includes(action)) out({ verdict: 'FAIL', reason: `BATCH_STATE_FORBIDS batch=${b.id} status=${b.status} action=${action} allowed=${(BATCH_ALLOWED[b.status] || []).join(',')}` }, 1);
    return b;
  }
  let c = s.batches.filter(x => (BATCH_ALLOWED[x.status] || []).includes(action));
  if (action === 'dispatch') {
    // 默认按 spec 顺序取第一个能开工的；要并行开第二批时用 --batch 指定
    const r = readyBatches(s);
    if (r.length) return r[0];
    c = c.slice(0, 1);
  }
  if (c.length === 1) return c[0];
  if (!c.length) out({ verdict: 'FAIL', reason: `STATE_FORBIDS 没有批次处在可 ${action} 的状态`, batches: s.batches.map(x => `${x.id}:${x.status}`) }, 1);
  out({ verdict: 'FAIL', reason: `BATCH_AMBIGUOUS 多个批次都能 ${action}，用 --batch 指定`, candidates: c.map(x => x.id) }, 1);
}
function syncGlobal(s) { s.state = s.batches.every(b => b.status === 'delivered') ? 'ALL_DELIVERED' : 'EXECUTING'; }

if (cmd === 'init') {
  const s = load(); require(s, cmd);
  const d = specDrift(s); if (d) out({ verdict: 'FAIL', reason: d }, 1);
  const sp = L.readJSON(resolve(repo, s.spec_path));
  const reasons = [];
  const prior = new Map((s.prior_batches || []).map(b => [b.id, b]));
  const batches = [];
  for (const p of sp.phases) for (const b of p.batches) {
    const tasks = [];
    for (const t of b.tasks) {
      const c = L.loadCard(P, t.card);
      if (c.error) { reasons.push(c.error); continue; }
      if (c.card.phase !== p.id) reasons.push('CARD_PHASE_MISMATCH ' + c.id + ' card=' + c.card.phase + ' spec=' + p.id);
      tasks.push({ task_id: t.id, role: t.role, card_id: c.id, card_sha256: c.sha256, card: c.card });
    }
    const pb = prior.get(b.id);
    const sameCards = pb && JSON.stringify(pb.tasks.map(x => x.card_id + x.card_sha256)) === JSON.stringify(tasks.map(x => x.card_id + x.card_sha256));
    batches.push({ id: b.id, phase: p.id, phase_slice: p.phase_vertical_slice.id, goal: b.goal, tasks,
      status: sameCards && pb.status === 'delivered' ? 'delivered' : 'pending', ...(sameCards && pb.deliveredAt ? { deliveredAt: pb.deliveredAt } : {}) });
  }
  const allCards = new Set(batches.flatMap(b => b.tasks.map(t => t.card_id)));
  for (const b of batches) for (const t of b.tasks) for (const x of t.card.blocked_by) if (!allCards.has(x)) reasons.push('BLOCKED_BY_UNKNOWN ' + t.card_id + '→' + x);
  if (reasons.length) out({ verdict: 'FAIL', reason: 'INIT_FAILED', reasons }, 1);
  s.batches = batches; s.batch = null;
  syncGlobal(s);
  s.history.push({ at: nowISO(), event: 'init' });
  L.saveState(P, s);
  out({ verdict: 'PASS', state: s.state, ready_to_dispatch: readyBatches(s).map(x => x.id), batches: batches.map(b => `${b.phase}/${b.id}:${b.tasks.length}卡:${b.status}`) }, 0);
}

/** 交给任意 harness 的独立 agent 的任务说明（纯文本，不依赖 harness 的工具名） */
function brief(b, t, batchRole, handoff) {
  const D = P.dirRel; const SS = b.__spec;
  if (!t) return `你是 ${batchRole}-<name>（${batchRole} 工种），只做本工种的事。批次 ${b.id}。读 ${D}/spec.json 与本批卡 ${b.tasks.map(x => `${D}/cards/${x.card_id}.json`).join('、')}，` +
    `按 skill 的 references/roles.md 履行 ${batchRole} 契约，把回执写到 ${D}/evidence/${b.id}/${batchRole}.json（模板 assets/evidence.template.json，spec_sha256=${SS}）。不改业务代码，不替别人下结论。`;
  const c = t.card;
  if (handoff) return `你是 ${c.qa_agent}（qa），只测卡 ${c.card_id}。${t.role} 已交卷，代码已冻结（handoff_id=${handoff.handoff_id}）。读 ${D}/cards/${c.card_id}.json 的验收标准，独立写测试并运行，不读 ${t.role} 的解释，不改业务代码。` +
    `回执写 ${D}/evidence/${b.id}/qa-${c.card_id}.json（spec_sha256=${SS}，handoff_id=${handoff.handoff_id}，layers_covered、rule_ids、command_exit_codes、artifacts 含 sha256）。${t.role} 若在你测试期间改代码，你的结果会被脚本作废。`;
  const fe = t.role === 'dev' && L.isFrontendCard(P, c) ? ' 这是前端卡：写代码前先跑 npx -y modern-web-guidance@latest search/retrieve，回执填 web_guidance。' : '';
  return `你是 ${c.owner_agent}（${t.role}），只实现卡 ${c.card_id}：读 ${D}/cards/${c.card_id}.json，只改 write_scopes（${c.write_scopes.join('、')}）。` +
    `完成后写 ${D}/evidence/${b.id}/${t.role}-${c.card_id}.json（spec_sha256=${SS}；files_changed、command_exit_codes、artifacts 含 sha256）。${fe}` +
    ` 交卷后代码冻结，由 ${c.qa_agent} 独立测试；交卷后不得再改，要改须 Lead 重新 handoff，测试重做。`;
}

if (cmd === 'dispatch') {
  const s = load(); require(s, cmd);
  const b = pickBatch(s, cmd);
  const reasons = [...batchPrereq(s, b), ...L.parallelConflicts(P, s, b)];
  const head = L.gitHead(repo);
  if (!head) reasons.push('GIT_UNAVAILABLE（需要至少一次提交作为基线）');
  const others = L.activeScopes(s, b.id);
  const pre = (L.changedBizFiles(P, null) || []).filter(f => !L.inScopes(f, others));
  if (pre.length) reasons.push('DIRTY_BUSINESS_TREE 派卡前（本批及无人认领的）业务目录必须干净：' + pre.slice(0, 5).join(','));
  if (reasons.length) out({ verdict: 'FAIL', reason: 'DISPATCH_FAILED', batch: b.id, reasons }, 1);
  // 漂移基线：同一轮交付内回流不重置；首次开工或已交付后被 issue 重开才取新基线
  if (!b.batch_base || b.reopened_from_delivered) { b.batch_base = head; b.firstDispatchedAt = nowISO(); delete b.reopened_from_delivered; }
  b.base_head = head; b.status = 'dispatched'; b.dispatchedAt = nowISO(); b.handoffs = {};
  syncGlobal(s); s.last_reasons = [];
  s.history.push({ at: b.dispatchedAt, event: 'dispatch', batch: b.id, base_head: head, batch_base: b.batch_base });
  L.saveState(P, s);
  b.__spec = s.spec_sha256;
  out({ verdict: 'PASS', state: s.state, batch: b.id, active_batches: L.activeBatches(s).map(x => x.id),
    wake: b.tasks.filter(t => L.isCardWorker(t.role)).map(t => ({ card: t.card_id, role: t.role, owner: t.card.owner_agent, brief: brief(b, t),
      ...(t.role === 'dev' && L.isFrontendCard(P, t.card) ? { must_first: 'npx -y modern-web-guidance@latest search "<2–5 词动作短语>" → retrieve "<id>"；输出存档进 artifacts，回执填 web_guidance', query_hints: t.card.web_guidance_queries || [] } : {}) })),
    other_tasks: b.tasks.filter(t => ![...L.CARD_WORKER_ROLES, 'gate', 'ledger', 'design'].includes(t.role)).map(t => ({ card: t.card_id, role: t.role, brief: `你是 ${t.role}-<name>，只做卡 ${t.card_id}，回执写 ${P.dirRel}/evidence/${b.id}/${t.role}-${t.card_id}.json` })),
    qa_waits_for_handoff: b.tasks.filter(t => L.isCardWorker(t.role)).map(t => ({ card: t.card_id, role: t.role, qa: t.card.qa_agent, after: `${t.role} 交卷后 loop.mjs handoff --card ${t.card_id}` })),
    batch_agents: L.BATCH_ROLES(P).map(r => ({ role: r, evidence: `${P.dirRel}/evidence/${b.id}/${r}.json`, brief: brief(b, null, r), when: '本批所有卡测试 PASS 之后' })),
    ready_to_dispatch_in_parallel: readyBatches(s).map(x => x.id),
    required_evidence: L.requiredEvidence(P, b).map(x => `${P.dirRel}/evidence/${b.id}/${x.file}`),
    next: '只唤醒交卷方（dev/ci）。每张卡交卷后跑 handoff --card <id>，脚本冻结代码再给出 qa 的 brief；测试不得与交卷同时进行；qa 是验证方，qa 卡不可 handoff' }, 0);
}

if (cmd === 'handoff') {
  const s = load(); require(s, cmd);
  const cid = typeof flags.card === 'string' ? flags.card : null;
  if (!cid) out({ verdict: 'FAIL', reason: 'CARD_REQUIRED --card <id>' }, 1);
  const b = s.batches.find(x => x.tasks.some(t => t.card_id === cid && L.isCardWorker(t.role)));
  if (!b) out({ verdict: 'FAIL', reason: 'CARD_NOT_WORKER_TASK ' + cid + '（只有交卷方工种卡（' + L.CARD_WORKER_ROLES.join('/') + '）能 handoff；qa 是验证方，不能自交自测）' }, 1);
  if (!(BATCH_ALLOWED[b.status] || []).includes('handoff')) out({ verdict: 'FAIL', reason: `BATCH_STATE_FORBIDS batch=${b.id} status=${b.status} action=handoff` }, 1);
  const t = b.tasks.find(x => x.card_id === cid);
  const r = L.workerReceiptProblems(P, s, b, t);
  if (r.length) out({ verdict: 'FAIL', reason: 'HANDOFF_REJECTED ' + t.role + ' 还没合格交卷', reasons: r }, 1);
  const h = L.makeHandoff(P, b, t);
  const redo = !!(b.handoffs || {})[cid];
  (b.handoffs ||= {})[cid] = h;
  s.history.push({ at: h.at, event: 'handoff', batch: b.id, card: cid, handoff_id: h.handoff_id, redo });
  L.saveState(P, s);
  b.__spec = s.spec_sha256;
  out({ verdict: 'PASS', batch: b.id, card: cid, role: t.role, handoff_id: h.handoff_id, frozen_files: Object.keys(h.files), ...(redo ? { note: '重新交接：之前的测试回执已作废' } : {}),
    wake: { role: 'qa', agent: t.card.qa_agent, brief: brief(b, t, null, h) } }, 0);
}

if (cmd === 'evidence') {
  const s = load(); require(s, cmd);
  const b = pickBatch(s, cmd);
  const r = L.checkEvidence(P, s, b);
  if (r.reasons.length) { s.last_reasons = r.reasons; L.saveState(P, s); out({ verdict: 'FAIL', reason: 'EVIDENCE_INCOMPLETE', batch: b.id, reasons: r.reasons, rows: r.rows, notes: r.notes || [] }, 1); }
  b.evidence_sha256 = Object.fromEntries(L.requiredEvidence(P, b).map(x => [x.file, L.shaFile(join(L.evDir(P, b.id), x.file))]));
  b.status = 'evidence_collected'; s.last_reasons = [];
  L.saveState(P, s);
  out({ verdict: 'PASS', state: s.state, batch: b.id, batch_status: b.status, rows: r.rows, notes: r.notes || [], next: 'accept（内部跑五层验收）' }, 0);
}

if (cmd === 'accept') {
  const s = load(); require(s, cmd);
  const b = pickBatch(s, cmd);
  const reasons = [];
  for (const [f, h] of Object.entries(b.evidence_sha256 || {})) {
    const p = join(L.evDir(P, b.id), f);
    if (!existsSync(p) || L.shaFile(p) !== h) reasons.push('EVIDENCE_CHANGED_AFTER_COLLECT ' + f);
  }
  const ev = L.checkEvidence(P, s, b); reasons.push(...ev.reasons); const evNotes = ev.notes || [];
  const fl = L.fiveLayers(P, s, b); reasons.push(...fl.reasons);
  reasons.push(...L.generatedCheck(P));
  if (reasons.length) {
    b.status = 'reopened'; s.last_reasons = reasons;
    s.history.push({ at: nowISO(), event: 'accept-fail→REOPEN', batch: b.id });
    L.saveState(P, s);
    out({ verdict: 'FAIL', state: s.state, batch: b.id, batch_status: 'reopened', reasons, notes: evNotes, layers: fl.layers, next: 'dispatch --batch ' + b.id + ' 回流本批，不跳过、不降级' }, 1);
  }
  b.status = 'accepted'; s.last_reasons = [];
  L.saveState(P, s);
  out({ verdict: 'PASS', state: s.state, batch: b.id, batch_status: 'accepted', notes: evNotes, layers: fl.layers, next: 'release（overall 文件 + CI）' }, 0);
}

// **合并前置门（machine iron rule）**：合并 PR 之前必须先过这一条，否则一律视为绕开 CI。
// 事故：2026-11-19 Lead 在 ci 工种回执未产出、批次未 accept 时就合并了 PR，事后补写回执。
if (cmd === 'premerge') {
  const s = load(); require(s, cmd);
  const b = pickBatch(s, cmd);
  const r = L.premergeCheck(P, s, b);
  if (!r.ok) out({ verdict: 'REJECT', ...r, hint: '合并被拒：dev/qa/gate/ledger 交回执 → accept → ci 工种出 evidence/<batch>/ci.json → 再 premerge' }, 1);
  out({ verdict: 'PASS', merge_allowed: true, batch: r.batch, head: r.head, runs: r.runs }, 0);
}
if (cmd === 'release') {
  const s = load(); require(s, cmd);
  const b = pickBatch(s, cmd);
  const reasons = [];
  const ov = L.overallOk(P);
  if (!ov.ok) reasons.push('OVERALL_FALSE ' + ov.file);
  // ci 回执**必须**不参与脏检查：它把自己绑在 `pushed.head_sha == 当前 HEAD` 上，
  // 一旦把回执提交进去，HEAD 就变了、回执立刻自相矛盾（提交即失效）。
  // 所以 ci.json 只能作为运行时产物留在工作树里；CI 是否真绿由 `ciStatus()` 现场
  // 用 `gh run list --commit <HEAD>` 复查，回执不是唯一证据源，排除它不放宽任何判据。
  // 路径解析走 L.releaseDirtyFiles()（内部用 L.statusPath()）：旧 slice(3) 在 git() trim 后
  // 会吃掉路径首字符，使 ci.json / state 的豁免与 otherOk 范围判断对已跟踪被修改文件全部失配。
  const dirtyFiles = L.releaseDirtyFiles(P, s, b);
  if (dirtyFiles === null) reasons.push('GIT_UNAVAILABLE'); else if (dirtyFiles.length) reasons.push('WORKTREE_DIRTY：发布前全部提交并推送 ' + dirtyFiles.slice(0, 5).join(','));
  reasons.push(...L.generatedCheck(P));
  // CI 强制项（无后门）：①必须有 ci 工种回执且与当前 HEAD 对得上 ②当前 HEAD 必须真跑过远端 CI 且全绿
  const ciRec = L.ciReceiptCheck(P, b);
  if (!ciRec.ok) reasons.push(ciRec.reason);
  const ci = L.ciStatus(P);
  if (!ci.ok) reasons.push(ci.reason);
  if (reasons.length) out({ verdict: 'FAIL', reason: 'RELEASE_BLOCKED', batch: b.id, reasons }, 1);
  b.release_head = ci.head; b.status = 'released';
  L.saveState(P, s);
  out({ verdict: 'PASS', state: s.state, batch: b.id, batch_status: 'released', head: ci.head, ci_runs: ci.runs, ...(ci.warning ? { warning: ci.warning } : {}), next: 'deliver' }, 0);
}

if (cmd === 'deliver') {
  const s = load(); require(s, cmd);
  const b = pickBatch(s, cmd);
  const d = { schema: 'agent-workforce-delivery/v1', at: nowISO(), batch: b.id, phase: b.phase, head: b.release_head,
    report: '只报用户能操作什么、看到什么、与验收口径是否一致、过/不过。不谎报完成。', user_signoff: false };
  for (const i of s.issues || []) if (i.batch === b.id && i.status === 'open') { i.status = 'redelivered'; i.redeliveredAt = d.at; }
  s.deliveries.push(d); b.status = 'delivered'; b.deliveredAt = d.at;
  syncGlobal(s);
  L.saveState(P, s);
  out({ verdict: 'PASS', state: s.state, batch: b.id, delivery: d, ready_to_dispatch: readyBatches(s).map(x => x.id),
    next: s.state === 'ALL_DELIVERED' ? '所有批次已交付；仍不宣布完成，等 issue' : 'dispatch 下一个可开工的批次；不宣布完成' }, 0);
}

if (cmd === 'ready') {
  const s = load();
  const r = readyBatches(s);
  out({ verdict: 'PASS', state: s?.state || 'IDLE', active_batches: L.activeBatches(s).map(x => x.id), ready_to_dispatch: r.map(x => x.id),
    blocked: (s?.batches || []).filter(b => ['pending', 'reopened'].includes(b.status) && !r.includes(b)).map(b => ({ batch: b.id, reasons: [...batchPrereq(s, b), ...L.parallelConflicts(P, s, b)] })) }, 0);
}

if (cmd === 'user-issue') {
  const s = load(); require(s, cmd);
  const ip = typeof flags.issue === 'string' ? resolve(flags.issue) : null;
  if (!ip || !existsSync(ip)) out({ verdict: 'FAIL', reason: 'ISSUE_FILE_MISSING：--issue <' + P.dirRel + '/issues/ISSUE-xxx.json>' }, 1);
  const issue = L.readJSON(ip);
  if (issue.__error) out({ verdict: 'FAIL', reason: 'INVALID_ISSUE_JSON' }, 1);
  const need = ['issue_id', 'raised_by', 'artifact', 'layer', 'expected', 'observed', 'severity', 'trace_to_card'];
  const miss = need.filter(k => !issue[k]);
  const reasons = [];
  if (miss.length) reasons.push('ISSUE_FIELDS_MISSING ' + miss.join(','));
  if (issue.raised_by !== 'user') reasons.push('ISSUE_NOT_FROM_USER');
  if (!L.LAYERS.includes(L.layerId(issue.layer))) reasons.push('ISSUE_LAYER_INVALID（event|logic|interface|state|view）');
  if ((s.issues || []).some(x => x.issue_id === issue.issue_id)) reasons.push('ISSUE_DUPLICATE ' + issue.issue_id);
  const c = L.userCommitted(P, ip); if (!c.ok) reasons.push('ISSUE_' + c.reason);
  const b = s.batches.find(x => x.tasks.some(t => t.card_id === issue.trace_to_card));
  if (!b) reasons.push('TRACE_CARD_UNKNOWN ' + issue.trace_to_card);
  else if (b.status === 'pending') reasons.push('TRACE_CARD_NOT_STARTED ' + issue.trace_to_card);
  if (reasons.length) out({ verdict: 'FAIL', reason: 'USER_ISSUE_REJECTED', reasons }, 1);
  s.issues.push({ issue_id: issue.issue_id, layer: L.layerId(issue.layer), severity: issue.severity, trace_to_card: issue.trace_to_card,
    batch: b.id, status: 'open', sha256: L.shaFile(ip), commit: c.commit, at: nowISO() });
  if (b.status === 'delivered') b.reopened_from_delivered = true;
  b.status = 'reopened';
  syncGlobal(s);
  L.saveState(P, s);
  out({ verdict: 'PASS', state: s.state, batch: b.id, batch_status: 'reopened', issue_id: issue.issue_id,
    next: ['view', 'event', 'logic'].includes(L.layerId(issue.layer)) ? 'dispatch --batch ' + b.id + ' 回流（把 issue 写进 reopen 卡）' : 'dispatch --batch ' + b.id + ' 回流；若需改 spec，由架构师 spec-revise' }, 0);
}

if (cmd === 'reopen') {
  const s = load(); require(s, cmd);
  if (typeof flags.reason !== 'string') out({ verdict: 'FAIL', reason: 'REASON_REQUIRED --reason "<为什么回流>"' }, 1);
  const b = pickBatch(s, cmd);
  b.status = 'reopened'; s.last_reasons = [flags.reason];
  s.history.push({ at: nowISO(), event: 'reopen', batch: b.id, reason: flags.reason });
  L.saveState(P, s);
  out({ verdict: 'PASS', state: s.state, batch: b.id, batch_status: 'reopened' }, 0);
}


out({ verdict: 'FAIL', reason: 'UNKNOWN_COMMAND ' + cmd,
  commands: ['status', 'guard', 'check-drift', 'validate-skill', 'scaffold', 'config-check', 'ready', ...new Set(Object.values(ALLOWED).flat())] }, 1);


