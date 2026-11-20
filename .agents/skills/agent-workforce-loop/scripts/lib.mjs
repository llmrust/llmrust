/**
 * agent-workforce-loop v1 共享库：参数、配置、状态、spec、签署、回执、漂移、五层验收。
 * 与具体 agent harness 无关：只依赖 Node 与 git；项目差异全部来自 <workforce 目录>/config.json。
 * 所有判定都在这里，loop.mjs / check-roles.mjs / accept-five-layers.mjs / validate-spec.mjs 共用，
 * 保证「脚本单独跑」和「状态迁移内部跑」是同一套判据。
 */
import { readFileSync, writeFileSync, existsSync, mkdirSync, readdirSync } from 'node:fs';
import { join, dirname, resolve, relative, sep } from 'node:path';
import { execFileSync } from 'node:child_process';
import { createHash, generateKeyPairSync, createPrivateKey, createPublicKey, sign as cSign, verify as cVerify, scryptSync, randomBytes, createCipheriv, createDecipheriv } from 'node:crypto';

export const sha = (buf) => createHash('sha256').update(buf).digest('hex');
export const shaFile = (p) => sha(readFileSync(p));
export const norm = (p) => p.split(sep).join('/').replace(/\\/g, '/');

// ---------- CRLF 假漂移检测（**只报警，不改判据**） ----------
// 事故（2026-11-20，llmrust B-DELTA-NULL）：Windows 上 `core.autocrlf=true` 且仓库无 .gitattributes 时，
// 任何 git checkout/restore 都会把 LF blob 物化成 CRLF 工作区文件。shaFile() 哈希的是**原始字节**，
// 而 config_sha256 / acceptors_sha256 是在 scaffold 阶段按 LF 内容锁定的，于是出现
// 「内容逐字相同、git status 干净、但 sha 对不上」的 CONFIG_DRIFT 假红（实测 1761-1704=57 字节 = 换行数）。
// 这里**只做识别与报警**：绝不改变 shaFile() 的哈希口径，否则会静默改变所有既有锁的语义。
export const hasCRLF = (buf) => buf.includes('\r\n');
/** 按 LF 归一化后计算 sha（与 gen-docs.mjs 的 MANIFEST 口径一致），用于识别「只是行尾不同」 */
export const shaFileLF = (p) => sha(Buffer.from(readFileSync(p, 'utf8').replace(/\r\n/g, '\n'), 'utf8'));
/** 若文件是 CRLF 且 LF 归一化后与 givenSha 相同 ⇒ 返回 'CRLF_NORMALIZED <file>'，否则 null */
export function crlfNormalizedNote(p, givenSha) {
  try {
    const buf = readFileSync(p);
    if (!hasCRLF(buf)) return null;
    if (!givenSha) return null;
    if (shaFileLF(p) !== String(givenSha)) return null;
    return 'CRLF_NORMALIZED ' + norm(p) + '：工作区文件是 CRLF，按 LF 归一化后与锁定的 sha 一致（内容相同，仅行尾不同；多半是 Windows core.autocrlf=true 物化造成，不是真实改动）';
  } catch { return null; }
}
/** git 层风险：core.autocrlf=true 且仓库无 .gitattributes ⇒ 'AUTOCRLF_RISK ...'（warning 级，不阻断） */
export function autocrlfRisk(repo) {
  try {
    const v = execFileSync('git', ['config', '--get', 'core.autocrlf'], { cwd: repo, encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).trim().toLowerCase();
    if (!['true', 'input'].includes(v)) return null;
    if (existsSync(join(repo, '.gitattributes'))) return null;
    return 'AUTOCRLF_RISK：git core.autocrlf=' + v + ' 且仓库无 .gitattributes；git 物化会把 LF 文件写成 CRLF，导致按原始字节算的 sha 与 LF 锁不一致（CONFIG_DRIFT 假红）。建议 git config --local core.autocrlf false';
  } catch { return null; }
}

export function parseArgs(argv) {
  const pos = [];
  const flags = {};
  for (let i = 0; i < argv.length; i++) {
    const v = argv[i];
    if (v.startsWith('--')) {
      const k = v.slice(2);
      const nx = argv[i + 1];
      if (nx === undefined || nx.startsWith('--')) flags[k] = true;
      else { flags[k] = nx; i++; }
    } else pos.push(v);
  }
  return { pos, flags };
}

export function out(o, code) {
  console.log(JSON.stringify(o, null, 2));
  process.exit(code);
}

export function readJSON(p) {
  try { return JSON.parse(readFileSync(p, 'utf8').replace(/^\uFEFF/, '')); }
  catch (e) { return { __error: String(e.message || e) }; }
}

// ---------- 路径 ----------
/** workforce 目录：--dir > 环境变量 WORKFORCE_DIR > .workforce */
export function paths(repo, flags = {}) {
  const dirRel = norm(typeof flags.dir === 'string' ? flags.dir : (process.env.WORKFORCE_DIR || '.workforce')).replace(/\/$/, '');
  const W = join(repo, dirRel);
  return {
    repo, W, dirRel,
    config: join(W, 'config.json'),
    state: join(W, 'loop-state.json'),
    spec: join(W, 'spec.json'),
    cards: join(W, 'cards'),
    signoffs: join(W, 'signoffs'),
    evidence: join(W, 'evidence'),
    issues: join(W, 'issues'),
    acceptors: join(W, 'acceptors.json'),
    approverPub: join(W, 'approver.pub.json'),
    approvals: join(W, 'approvals'),
  };
}

// ---------- 配置（项目差异只在这里） ----------
export const ALL_WORKER_ROLES = ['architect', 'dev', 'qa', 'gate', 'ci', 'ledger', 'design'];
export const OPTIONAL_ROLES = ['design'];
export const LAYERS = ['event', 'logic', 'interface', 'state', 'view'];
const LAYER_ALIAS = { 事件: 'event', 逻辑: 'logic', 接口: 'interface', 状态: 'state', 视图: 'view' };
export const layerId = (x) => LAYER_ALIAS[x] || String(x || '').toLowerCase();
export const DEFAULT_CONFIG = {
  schema: 'agent-workforce-config/v1',
  business_dirs: ['src/'],
  roles: { design: true },
  architect_required_inputs: [],
  gates: { required_commands: [], require_no_mock: true },
  rule_id_pattern: null,
  release: { overall_file: null, overall_pointer: '/summary/overall' },
  ci: { provider: 'github', repo: null, branch: null, command: null, none_reason: null },
  frontend: { dirs: [], web_guidance: true },
  ledger: { command: null, none_reason: null },
  acceptance: { user_confirm: false },
  parallel: { max_batches: 2 },
  generated: [],
  trust: { anchor: 'local_review', remote: 'origin', branch: 'main', allowed_signers: null, none_reason: null },
};
const merge = (a, b) => {
  const o = { ...a };
  for (const [k, v] of Object.entries(b || {})) o[k] = v && typeof v === 'object' && !Array.isArray(v) && a[k] && typeof a[k] === 'object' ? merge(a[k], v) : v;
  return o;
};
let _cfg = null;
/** 读取并校验 config.json；problems 非空时所有迁移拒绝 */
export function loadConfig(P) {
  if (_cfg && _cfg.path === P.config) return _cfg;
  const problems = [];
  if (!existsSync(P.config)) {
    _cfg = { path: P.config, present: false, sha256: null, c: DEFAULT_CONFIG, problems: ['CONFIG_MISSING ' + P.dirRel + '/config.json（先跑 loop.mjs scaffold，再由验收者确认并 git 提交）'] };
    return _cfg;
  }
  const raw = readJSON(P.config);
  if (raw.__error) problems.push('CONFIG_INVALID_JSON');
  const c = merge(DEFAULT_CONFIG, raw.__error ? {} : raw);
  if (!Array.isArray(c.business_dirs) || !c.business_dirs.length) problems.push('CONFIG_NO_BUSINESS_DIRS');
  if (c.business_dirs.some(d => !String(d).endsWith('/'))) problems.push('CONFIG_BUSINESS_DIR_NEEDS_TRAILING_SLASH');
  if (!Array.isArray(c.gates.required_commands) || !c.gates.required_commands.length) problems.push('CONFIG_NO_GATE_COMMANDS（gates.required_commands 至少一条，如 "npm test"）');
  for (const r of Object.keys(c.roles || {})) if (!OPTIONAL_ROLES.includes(r)) problems.push('CONFIG_ROLE_NOT_OPTIONAL ' + r + '（只有 design 可关闭）');
  // CI 是**强制项**：只允许真远端 CI（provider=github）。command/none 都是绕开，一律拒绝配置。
  if (c.ci.provider === 'none') problems.push('CONFIG_CI_BYPASS_FORBIDDEN：ci.provider="none" 等于绕开 CI，不允许。CI 必须接真远端（provider="github"）');
  else if (c.ci.provider === 'command') problems.push('CONFIG_CI_BYPASS_FORBIDDEN：ci.provider="command" 只是本地跑命令、不是远端 CI，等于绕开。改为 provider="github" + ci.repo');
  else if (c.ci.provider !== 'github') problems.push('CONFIG_CI_PROVIDER_INVALID ' + c.ci.provider);
  if (c.ci.provider === 'github' && !/^[\w.-]+\/[\w.-]+$/.test(String(c.ci.repo || ''))) problems.push('CONFIG_CI_REPO_MISSING：provider="github" 时必须写 ci.repo="owner/name"（CI 跑在哪个远端仓库）');
  if (c.ci.none_reason || c.ci.command) problems.push('CONFIG_CI_LEGACY_FIELDS：ci.command / ci.none_reason 已废弃（它们是绕开口子），请删除');
  if (!c.ledger.command && !c.ledger.none_reason) problems.push('CONFIG_LEDGER_UNVERIFIED（ledger.command 填台账生成命令，供 accept 重跑比对；确实没有则写 ledger.none_reason）');
  if (!['local_review', 'remote', 'signed', 'none'].includes(c.trust.anchor)) problems.push('CONFIG_TRUST_ANCHOR_INVALID ' + c.trust.anchor);
  if (!Array.isArray(c.generated) || c.generated.some(g => !g || !g.path || !g.command)) problems.push('CONFIG_GENERATED_INVALID（每项需 path + command）');
  if (c.trust.anchor === 'remote' && (!c.trust.remote || !c.trust.branch)) problems.push('CONFIG_TRUST_REMOTE_MISSING');
  if (c.trust.anchor === 'signed' && !c.trust.allowed_signers) problems.push('CONFIG_TRUST_ALLOWED_SIGNERS_MISSING');
  if (c.trust.anchor === 'none' && !c.trust.none_reason) problems.push('CONFIG_TRUST_NONE_NEEDS_REASON');
  if (typeof c.acceptance.user_confirm !== 'boolean') problems.push('CONFIG_ACCEPTANCE_USER_CONFIRM_NOT_BOOLEAN');
  if (!Number.isInteger(c.parallel.max_batches) || c.parallel.max_batches < 1) problems.push('CONFIG_PARALLEL_MAX_BATCHES_INVALID');
  if (c.rule_id_pattern) { try { new RegExp(c.rule_id_pattern); } catch { problems.push('CONFIG_RULE_ID_PATTERN_INVALID'); } }
  _cfg = { path: P.config, present: true, sha256: shaFile(P.config), c, problems };
  // 只报警不改判据：CRLF 物化会让 shaFile 与 LF 锁不一致（CONFIG_DRIFT 假红），这里给出可读诊断
  _cfg.crlfNote = crlfNormalizedNote(P.config, _cfg.sha256) || null;
  _cfg.autocrlfRisk = autocrlfRisk(P.repo) || null;
  return _cfg;
}
export const cfg = (P) => loadConfig(P).c;
export const workerRoles = (P) => ALL_WORKER_ROLES.filter(r => !OPTIONAL_ROLES.includes(r) || cfg(P).roles[r] !== false);
// ci 不在 BATCH_ROLES：它的回执必须在 accept 之后、release 之前产生（推送只能发生在验收之后），
// 否则 evidence 阶段就要求它会造成"未验收不能推、不推又没有 CI"的死锁。release 处强制校验（ciReceiptCheck）。
export const BATCH_ROLES = (P) => ['gate', 'ledger', 'design'].filter(r => workerRoles(P).includes(r));
// 「交卷方」卡角色：自己交付、经 handoff 冻结文件、再由 qa 独立测试。qa 是验证方，**不在**集合里（不能自交自测）。
// 有意不含 gate/ledger（批次级回执，无卡）与 architect/lead（只产出 spec/审批）。
export const CARD_WORKER_ROLES = ['dev', 'ci'];
export const isCardWorker = (r) => CARD_WORKER_ROLES.includes(r);
/** 卡上的 owner_agent / qa_agent 可以是具体 agent 名（如 dev-C1）或工种名（如 ci / qa，无连字符）。
 *  具体名必须精确匹配；工种名接受该工种席位（<role>-<name>），因为卡只指定了工种而不是某个席位。 */
export const agentMatchesSeat = (agent, seat) => {
  const a = String(agent || ''), s = String(seat || '');
  if (!a || !s) return false;
  if (a === s) return true;
  return !s.includes('-') && a.startsWith(s + '-');
};
export function jsonPointer(obj, ptr) {
  if (!ptr || ptr === '/') return obj;
  return ptr.split('/').slice(1).map(x => x.replace(/~1/g, '/').replace(/~0/g, '~')).reduce((o, k) => (o == null ? undefined : o[k]), obj);
}
export function overallOk(P) {
  const r = cfg(P).release;
  if (!r.overall_file) return { ok: true, skipped: true };
  const p = join(P.repo, r.overall_file);
  const j = existsSync(p) ? readJSON(p) : null;
  return { ok: jsonPointer(j, r.overall_pointer) === true, skipped: false, file: r.overall_file };
}

export const CARD_FIELDS = ['card_id', 'phase', 'owner_agent', 'qa_agent', 'write_scopes',
  'acceptance_criteria', 'independent_test', 'blocked_by', 'checkpoint'];
export const EVIDENCE_FIELDS = ['schema', 'role', 'agent', 'spec_sha256', 'batch_id', 'artifacts',
  'command_exit_codes', 'verdict', 'independent', 'declared_by_lead'];

// ---------- 状态 ----------
export function readState(P) { return existsSync(P.state) ? readJSON(P.state) : null; }
export function saveState(P, s) {
  mkdirSync(dirname(P.state), { recursive: true });
  delete s.self_sha256;
  s.generatedAt = new Date().toISOString();
  s.self_sha256 = sha(JSON.stringify(s));
  writeFileSync(P.state, JSON.stringify(s, null, 2) + '\n', 'utf8');
}
/** 状态文件被手改：self_sha256 对不上 */
export function stateTampered(s) {
  if (!s || !s.self_sha256) return false;
  const c = { ...s }; const want = c.self_sha256; delete c.self_sha256;
  return sha(JSON.stringify(c)) !== want;
}

// ---------- git ----------
export function git(repo, args) {
  try { return execFileSync('git', args, { cwd: repo, encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).trim(); }
  catch { return null; }
}
/**
 * 解析 `git status --porcelain`（v1）的一条记录 → 工作区相对路径。
 *
 * 为什么不能用固定 `slice(3)`：`git()` 会 trim 整个输出，**第一条记录**的前导状态空格
 * 因此可能丢失（`" M x"` → `"M x"`）。此时 `slice(3)` 会吃掉路径首字符 ——
 * 例如 `.workforce/evidence/<batch>/ci.json` 变成 `workforce/...`，
 * 于是 release 里对 ci 回执 / state 文件的豁免、以及 otherOk 的范围判断全部失配（真 bug）。
 * 改为「1~2 字符状态码 + 空白 + 路径」解析：trim 与未 trim 两种形态都能得到真实路径。
 * 重命名/复制记录（v1：`R  old -> new`）取 ` -> ` 之后的**新路径**；双引号包裹的路径去包裹。
 */
export function statusPath(rec) {
  const s = String(rec == null ? '' : rec);
  const m = s.match(/^([ MADRCU?!]{1,2})\s+(.*)$/);
  let p = (m ? m[2] : s).trim();
  const st = s.trim();
  if (/^[RC]/.test(st) && p.includes(' -> ')) p = p.split(' -> ').pop().trim();
  if (p.length >= 2 && p.startsWith('"') && p.endsWith('"')) p = p.slice(1, -1);
  return p;
}
/**
 * release 前的工作树脏文件（必须干净才允许 release）。排除项不变：
 *   ① 本批 state 文件（工具自身状态） ② 本批 ci 回执（它绑 pushed.head_sha，提交即自废）
 *   ③ 其他在做的批次的 write_scopes / 回执目录。
 * 路径解析统一走 statusPath()：旧实现的 `slice(3)` 在 git() trim 掉首条记录前导空格时
 * 会吃掉路径首字符，导致 ①/②/③ 的豁免与范围判断对「已跟踪且被修改」的文件全部失配（真 bug）。
 */
export function releaseDirtyFiles(P, st, b) {
  const dirty = git(P.repo, ['status', '--porcelain', '-uall']);
  if (dirty === null) return null;
  const others = activeBatches(st).filter(x => x.id !== b.id);
  const otherOk = (f) => inScopes(f, others.flatMap(x => x.tasks.flatMap(t => t.card.write_scopes)))
    || others.some(x => f.startsWith(P.dirRel + '/evidence/' + x.id + '/'));
  const relState = norm(relative(P.repo, P.state));
  const ciReceiptRel = P.dirRel + '/evidence/' + b.id + '/ci.json';
  return dirty.split('\n').filter(Boolean).map(statusPath)
    .filter(f => f !== relState && f !== ciReceiptRel && !otherOk(f));
}
export function gitHead(repo) { return git(repo, ['rev-parse', 'HEAD']); }
/** 文件已提交、工作区无改动、最后一次提交作者邮箱 */
export function committedBy(repo, file) {
  const rel = norm(relative(repo, file));
  const tracked = git(repo, ['ls-files', '--error-unmatch', rel]);
  if (tracked === null) return { ok: false, reason: 'NOT_COMMITTED ' + rel };
  const dirty = git(repo, ['status', '--porcelain', '--', rel]);
  if (dirty) return { ok: false, reason: 'UNCOMMITTED_CHANGES ' + rel };
  const email = git(repo, ['log', '-1', '--format=%ae', '--', rel]);
  const commit = git(repo, ['log', '-1', '--format=%H', '--', rel]);
  return { ok: !!email, email: (email || '').toLowerCase(), commit };
}
/**
 * 信任锚：git 作者邮箱谁都能填，acceptors.json 也可能被 agent 抢先自写自提交（首次占位）。
 * 所以验收者提交还必须满足仓外锚点之一：
 *   remote — 提交已进入远端受保护分支（git ls-remote 取远端 tip，agent 凭据必须推不上该分支）
 *   signed — 提交带签名且 git verify-commit 通过（签名私钥只在用户手里）
 *   none   — 不设锚（必须写 none_reason；status 永久提示 TRUST_NONE）
 */
export function trustAnchored(P, commit) {
  const T = cfg(P).trust;
  if (T.anchor === 'none') return { ok: true, warning: 'TRUST_NONE：' + T.none_reason };
  if (T.anchor === 'signed') {
    const args = T.allowed_signers ? ['-c', 'gpg.ssh.allowedSignersFile=' + resolve(P.repo, T.allowed_signers), 'verify-commit', commit] : ['verify-commit', commit];
    try { execFileSync('git', args, { cwd: P.repo, stdio: 'ignore' }); return { ok: true }; }
    catch { return { ok: false, reason: 'TRUST_UNSIGNED ' + commit.slice(0, 8) + '（需用户签名提交，git verify-commit 未通过）' }; }
  }
  const ls = git(P.repo, ['ls-remote', T.remote, 'refs/heads/' + T.branch]);
  const tip = ls && ls.split(/\s+/)[0];
  if (!tip) return { ok: false, reason: 'TRUST_REMOTE_UNREACHABLE ' + T.remote + '/' + T.branch };
  if (git(P.repo, ['cat-file', '-e', tip + '^{commit}']) === null) return { ok: false, reason: 'TRUST_REMOTE_TIP_NOT_FETCHED（先 git fetch ' + T.remote + '）' };
  if (git(P.repo, ['merge-base', '--is-ancestor', commit, tip]) === null) return { ok: false, reason: 'TRUST_NOT_ON_PROTECTED_BRANCH ' + commit.slice(0, 8) + ' 未进入 ' + T.remote + '/' + T.branch };
  return { ok: true };
}
export function acceptorEmails(P) {
  // local_review：用户本地审过、由执行侧提交，不比对作者邮箱；靠 spec-confirm 时锁定 sha 防后改
  if (cfg(P).trust.anchor === 'local_review') return { ok: true, emails: [], sha: existsSync(P.acceptors) ? shaFile(P.acceptors) : null, local_review: true };
  if (!existsSync(P.acceptors)) return { ok: false, reason: 'ACCEPTORS_MISSING ' + P.dirRel + '/acceptors.json' };
  const a = readJSON(P.acceptors);
  const emails = (a.emails || []).map(e => String(e).toLowerCase());
  if (!emails.length) return { ok: false, reason: 'ACCEPTORS_EMPTY' };
  const c = committedBy(P.repo, P.acceptors);
  if (!c.ok) return { ok: false, reason: 'ACCEPTORS_' + c.reason };
  if (!emails.includes(c.email)) return { ok: false, reason: 'ACCEPTORS_NOT_COMMITTED_BY_ACCEPTOR ' + c.email };
  const t = trustAnchored(P, c.commit);
  if (!t.ok) return { ok: false, reason: 'ACCEPTORS_' + t.reason };
  return { ok: true, emails, sha: shaFile(P.acceptors), warning: t.warning };
}
// ---------- 用户同意的佐证（local_review） ----------
/**
 * 用户本人在自己终端跑 approve.mjs：输入只有自己知道的审批密码，对「我同意这份文件（sha256）」做 Ed25519 签名。
 * 私钥用密码加密，放在用户主目录（仓库外）；仓库里只有公钥 approver.pub.json。
 * agent 能读到公钥和加密私钥，但没有密码就签不出同意记录；脚本只认签名，不认谁提交的。
 */
export const fingerprintOf = (pubPem) => sha(createPublicKey(pubPem).export({ type: 'spki', format: 'der' })).slice(0, 16).toUpperCase().match(/.{4}/g).join('-');
export function makeApprover(password) {
  if (!password || password.length < 8) throw new Error('APPROVER_PASSWORD_TOO_SHORT（至少 8 位）');
  const { publicKey, privateKey } = generateKeyPairSync('ed25519');
  const pub = publicKey.export({ type: 'spki', format: 'pem' });
  const salt = randomBytes(16), iv = randomBytes(12);
  const key = scryptSync(password, salt, 32, { N: 1 << 15, r: 8, p: 1, maxmem: 64 * 1024 * 1024 });
  const c = createCipheriv('aes-256-gcm', key, iv);
  const data = Buffer.concat([c.update(privateKey.export({ type: 'pkcs8', format: 'der' })), c.final()]);
  const fp = fingerprintOf(pub);
  return {
    store: { schema: 'awl-approver-key/v1', fingerprint: fp, public_key_pem: pub, salt: salt.toString('base64'), iv: iv.toString('base64'), tag: c.getAuthTag().toString('base64'), data: data.toString('base64') },
    pub: { schema: 'awl-approver-pub/v1', fingerprint: fp, public_key_pem: pub, created_at: new Date().toISOString() },
  };
}
export function unlockApprover(store, password) {
  const key = scryptSync(password, Buffer.from(store.salt, 'base64'), 32, { N: 1 << 15, r: 8, p: 1, maxmem: 64 * 1024 * 1024 });
  const d = createDecipheriv('aes-256-gcm', key, Buffer.from(store.iv, 'base64'));
  d.setAuthTag(Buffer.from(store.tag, 'base64'));
  try { return createPrivateKey({ key: Buffer.concat([d.update(Buffer.from(store.data, 'base64')), d.final()]), format: 'der', type: 'pkcs8' }); }
  catch { throw new Error('APPROVER_WRONG_PASSWORD'); }
}
const approvalBody = (r) => JSON.stringify({ schema: r.schema, kind: r.kind, target: r.target, target_sha256: r.target_sha256, statement: r.statement, approved_at: r.approved_at, approver_fingerprint: r.approver_fingerprint });
export function signApproval(priv, fp, kind, target, targetSha) {
  const r = { schema: 'awl-approval/v1', kind, target, target_sha256: targetSha, statement: '我已审核并同意', approved_at: new Date().toISOString(), approver_fingerprint: fp };
  r.signature = cSign(null, Buffer.from(approvalBody(r)), priv).toString('base64');
  return r;
}
export const relP = (P, f) => norm(relative(P.repo, f));
export const approvalFile = (P, kind, targetSha) => join(P.approvals, `${kind}-${targetSha.slice(0, 12)}.json`);
export function readApproverPub(P) {
  if (!existsSync(P.approverPub)) return { ok: false, reason: 'APPROVER_NOT_SET（用户本人在自己的终端跑一次 approve.mjs setup）' };
  const j = readJSON(P.approverPub);
  try { if (fingerprintOf(j.public_key_pem) !== j.fingerprint) return { ok: false, reason: 'APPROVER_PUB_INVALID' }; }
  catch { return { ok: false, reason: 'APPROVER_PUB_INVALID' }; }
  return { ok: true, pem: j.public_key_pem, fingerprint: j.fingerprint, sha: shaFile(P.approverPub) };
}
/** 有没有用户对这份文件当前内容的签名同意 */
export function verifyApproval(P, kind, targetRel, targetSha) {
  const pub = readApproverPub(P);
  if (!pub.ok) return pub;
  const f = approvalFile(P, kind, targetSha);
  const label = kind === 'config' ? '规则表' : kind === 'spec' ? '开工范围' : kind;
  if (!existsSync(f)) return { ok: false, reason: 'USER_APPROVAL_MISSING ' + kind + ' ' + targetSha.slice(0, 12) + '（没有你同意这份' + label + '的佐证：请本人跑 approve.mjs ' + kind + '）' };
  const r = readJSON(f);
  if (r.kind !== kind || r.target !== targetRel || r.target_sha256 !== targetSha) return { ok: false, reason: 'USER_APPROVAL_MISMATCH ' + relP(P, f) };
  if (r.approver_fingerprint !== pub.fingerprint) return { ok: false, reason: 'USER_APPROVAL_WRONG_APPROVER ' + relP(P, f) };
  let ok = false;
  try { ok = cVerify(null, Buffer.from(approvalBody(r)), createPublicKey(pub.pem), Buffer.from(r.signature || '', 'base64')); } catch { ok = false; }
  if (!ok) return { ok: false, reason: 'USER_APPROVAL_SIGNATURE_INVALID ' + relP(P, f) + '（签名对不上：不是你本人用审批密码签的）' };
  return { ok: true, fingerprint: pub.fingerprint, approved_at: r.approved_at, file: relP(P, f) };
}

/** 验收者（用户）亲自提交的文件 */
export function userCommitted(P, file) {
  const acc = acceptorEmails(P);
  if (!acc.ok) return acc;
  const c = committedBy(P.repo, file);
  if (!c.ok) return c;
  if (acc.local_review) return { ok: true, email: c.email, commit: c.commit, local_review: true };
  if (!acc.emails.includes(c.email)) return { ok: false, reason: 'NOT_COMMITTED_BY_ACCEPTOR ' + c.email };
  const t = trustAnchored(P, c.commit);
  if (!t.ok) return { ok: false, reason: t.reason };
  return { ok: true, email: c.email, commit: c.commit, warning: t.warning };
}
/** 业务目录改动（工作区 + base..HEAD） */
export function changedBizFiles(P, base) {
  const repo = P.repo;
  const files = new Set();
  const por = git(repo, ['status', '--porcelain', '-uall']);
  if (por === null) return null;
  for (const l of por.split('\n').filter(Boolean)) files.add(statusPath(l));
  if (base) {
    const d = git(repo, ['diff', '--name-only', base + '..HEAD']);
    if (d) d.split('\n').filter(Boolean).forEach(f => files.add(f));
  }
  return [...files].filter(f => cfg(P).business_dirs.some(d => f.startsWith(d))).sort();
}
/** 前端卡：write_scopes 落在 config.frontend.dirs 下，且 frontend.web_guidance 未关闭 → 必须先跑 Modern Web Guidance */
export const isFrontendCard = (P, card) => cfg(P).frontend.web_guidance !== false &&
  (card?.write_scopes || []).some(s => (cfg(P).frontend.dirs || []).some(d => s.startsWith(d)));
const MWG = /modern-web-guidance(@\S+)?\s+/;
/** dev 回执里的 web_guidance 段；guideIds 供 qa 交叉核对 */
export function checkWebGuidance(P, j, file) {
  const reasons = [];
  const g = j.web_guidance;
  if (!g || typeof g !== 'object') return { reasons: ['WEB_GUIDANCE_MISSING ' + file + '（前端卡实现前必须先跑 modern-web-guidance search/retrieve）'], guideIds: [] };
  const cmds = j.command_exit_codes || [];
  const searches = cmds.filter(c => MWG.test(c.cmd || '') && /\ssearch\s/.test(c.cmd || ''));
  if (!searches.length) reasons.push('WEB_GUIDANCE_NO_SEARCH ' + file);
  if (searches.some(c => c.exit_code !== 0)) reasons.push('WEB_GUIDANCE_SEARCH_FAILED ' + file);
  if (!Array.isArray(g.queries) || !g.queries.length) reasons.push('WEB_GUIDANCE_NO_QUERIES ' + file);
  const ids = Array.isArray(g.guide_ids) ? g.guide_ids : [];
  if (ids.length) {
    const rets = cmds.filter(c => MWG.test(c.cmd || '') && /\sretrieve\s/.test(c.cmd || ''));
    if (!rets.length || rets.some(c => c.exit_code !== 0)) reasons.push('WEB_GUIDANCE_NO_RETRIEVE ' + file);
    for (const id of ids) if (!rets.some(c => (c.cmd || '').includes(id))) reasons.push('WEB_GUIDANCE_ID_NOT_RETRIEVED ' + file + ' ' + id);
    const art = g.retrieved_artifact;
    if (!art || !(j.artifacts || []).some(a => a.path === art)) reasons.push('WEB_GUIDANCE_OUTPUT_NOT_IN_ARTIFACTS ' + file);
    else if (existsSync(resolve(P.repo, art)) && !ids.every(id => readFileSync(resolve(P.repo, art), 'utf8').includes(id))) reasons.push('WEB_GUIDANCE_OUTPUT_LACKS_IDS ' + file);
  } else if (!g.none_applicable_reason) reasons.push('WEB_GUIDANCE_NO_GUIDES_AND_NO_REASON ' + file);
  if (g.deviations !== undefined && !Array.isArray(g.deviations)) reasons.push('WEB_GUIDANCE_DEVIATIONS_NOT_ARRAY ' + file);
  for (const d of g.deviations || []) if (!d.guide_id || !d.overridden_by) reasons.push('WEB_GUIDANCE_DEVIATION_INCOMPLETE ' + file + '（需 guide_id + overridden_by，如项目技术栈裁定编号）');
  return { reasons, guideIds: ids };
}
export const inScopes = (f, scopes) => (scopes || []).some(s => f === s || f.startsWith(s.endsWith('/') ? s : s + '/'));

// ---------- spec ----------
export function validateSpecStructure(sp, P) {
  const ROLES = P ? workerRoles(P) : ALL_WORKER_ROLES;
  const reasons = [];
  const layers = {};
  if (!sp || sp.__error) return { reasons: ['INVALID_JSON'], layers };
  if (!/^architect-/.test(sp.produced_by || '')) reasons.push('SPEC_NOT_PRODUCED_BY_ARCHITECT');
  if (!sp.inputs_read?.length) reasons.push('ARCHITECT_DID_NOT_READ_EXISTING_MATERIAL');
  for (const need of (P ? cfg(P).architect_required_inputs : []) || []) if (!(sp.inputs_read || []).includes(need)) reasons.push('ARCHITECT_REQUIRED_INPUT_NOT_READ ' + need);
  const c = sp.constitution;
  if (!c || c.priority !== 'highest' || !c.articles?.length) reasons.push('CONSTITUTION_INVALID');
  else if (c.articles.some(a => !a.id || !a.text || !a.enforced_by)) reasons.push('CONSTITUTION_ARTICLE_INCOMPLETE');
  layers['1_constitution'] = { articles: c?.articles?.length || 0 };
  const vs = sp.total_vertical_slice;
  if (!vs?.id || !vs.goal || !vs.acceptance || vs.serves_all_modules !== true) reasons.push('TOTAL_VERTICAL_SLICE_INVALID');
  layers['2_total_slice'] = { id: vs?.id || null };
  if (!sp.phases?.length) reasons.push('PHASES_MISSING');
  const ids = new Set();
  const dupe = (id, kind) => { if (ids.has(id)) reasons.push('DUPLICATE_ID ' + kind + ' ' + id); ids.add(id); };
  let batches = 0, tasks = 0; const roles = new Set(); const cardSeen = new Map();
  for (const p of sp.phases || []) {
    if (!p.id) reasons.push('PHASE_NO_ID'); else dupe(p.id, 'phase');
    const pv = p.phase_vertical_slice;
    if (!pv?.id || !pv.goal) reasons.push('PHASE_VERTICAL_SLICE_MISSING ' + p.id);
    else if (pv.serves !== vs?.id) reasons.push('PHASE_SLICE_NOT_SERVING_TOTAL ' + p.id);
    if (!p.batches?.length) reasons.push('BATCHES_MISSING ' + p.id);
    for (const b of p.batches || []) {
      batches++;
      if (!b.id) reasons.push('BATCH_NO_ID'); else dupe(b.id, 'batch');
      if (!b.goal) reasons.push('BATCH_NO_GOAL ' + b.id);
      if (!b.tasks?.length) reasons.push('BATCH_NO_TASKS ' + b.id);
      for (const t of b.tasks || []) {
        tasks++;
        if (t.card) { const cid = cardId(t.card); if (cardSeen.has(cid)) reasons.push('CARD_ASSIGNED_TWICE ' + cid + '（' + cardSeen.get(cid) + ' 与 ' + t.id + '）：一张卡只能归一个任务、一个工种'); else cardSeen.set(cid, t.id); }
        if (!t.id || !t.role || !t.card) reasons.push('TASK_INCOMPLETE ' + (t.id || '?'));
        if (t.role === 'architect' || t.role === 'lead') reasons.push('ROLE_CANNOT_TAKE_TASK ' + t.id + ':' + t.role);
        else if (t.role && !ROLES.includes(t.role)) reasons.push('UNKNOWN_ROLE ' + t.id + ':' + t.role);
        if (t.role) roles.add(t.role);
      }
    }
  }
  layers['3_phases'] = (sp.phases || []).length;
  layers['4_batches'] = batches;
  layers['5_roles'] = [...roles].sort();
  layers['6_cards'] = tasks;
  return { reasons, layers };
}
export const cardId = (ref) => String(ref).split('#').pop().split('/').pop().replace(/\.json$/, '');
export function loadCard(P, ref) {
  const id = cardId(ref);
  const p = join(P.cards, id + '.json');
  if (!existsSync(p)) return { id, error: 'CARD_FILE_MISSING ' + norm(relative(P.repo, p)) };
  const c = readJSON(p);
  if (c.__error) return { id, error: 'CARD_INVALID_JSON ' + id };
  const miss = CARD_FIELDS.filter(k => c[k] === undefined || c[k] === null || c[k] === '');
  if (miss.length) return { id, error: 'CARD_FIELDS_MISSING ' + id + ' ' + miss.join(',') };
  if (c.card_id !== id) return { id, error: 'CARD_ID_MISMATCH ' + id };
  if (!Array.isArray(c.write_scopes) || !c.write_scopes.length) return { id, error: 'CARD_NO_WRITE_SCOPES ' + id };
  if (!Array.isArray(c.acceptance_criteria) || !c.acceptance_criteria.length) return { id, error: 'CARD_NO_ACCEPTANCE ' + id };
  if (!Array.isArray(c.blocked_by)) return { id, error: 'CARD_BLOCKED_BY_NOT_ARRAY ' + id };
  if (c.owner_agent === c.qa_agent) return { id, error: 'CARD_OWNER_IS_QA ' + id + '（开发与测试必须是两个 agent）' };
  return { id, card: c, sha256: shaFile(p) };
}

// ---------- 签署（每方自己写 signoffs/<role>.json，绑定 spec_sha256） ----------
export function readSignoff(P, role) {
  const p = join(P.signoffs, role + '.json');
  if (!existsSync(p)) return { role, present: false, path: p };
  return { role, present: true, path: p, ...readJSON(p) };
}
export function checkWorkerSignoffs(P, specSha) {
  const reasons = []; const rows = [];
  for (const r of workerRoles(P)) {
    const s = readSignoff(P, r);
    rows.push({ role: r, agent: s.agent || null, agreed: s.agreed === true });
    if (!s.present) { reasons.push('SIGNOFF_MISSING ' + r); continue; }
    if (!new RegExp('^' + r + '-').test(s.agent || '')) reasons.push('SIGNOFF_AGENT_NOT_ROLE ' + r + ':' + s.agent);
    if (s.spec_sha256 !== specSha) reasons.push('SIGNOFF_STALE_SPEC ' + r);
    if (s.agreed !== true) reasons.push('NOT_AGREED ' + r);
  }
  const ag = rows.map(x => x.agent).filter(Boolean);
  if (new Set(ag).size !== ag.length) reasons.push('SIGNOFF_AGENT_REUSE');
  return { reasons, rows };
}

// ---------- 回执 ----------
export function evDir(P, batchId) { return join(P.evidence, batchId); }
/**
 * 合并机器记录（INC-012 的「不再依赖回忆」）：合并成功后把事实写入
 * `<workforce>/evidence/<batch>/merge.json`。
 * 字段固定：{at, batch, pr, merge_commit, merge_called:true, head_sha, checks:[job 结论], bootstrap_used}。
 * 只允许 merge-pr.mjs 在**已发生合并**之后调用；dry-run/FAIL 分支不写。
 */
export function writeMergeRecord(P, batchId, rec) {
  const dir = evDir(P, batchId);
  mkdirSync(dir, { recursive: true });
  const j = { at: new Date().toISOString(), batch: batchId, ...rec };
  const target = join(dir, 'merge.json');
  writeFileSync(target, JSON.stringify(j, null, 2) + '\n', 'utf8');
  return target;
}
/**
 * 同批「兄弟回执」artifact 判定。
 *
 * 回执链是**顺序产生**的（dev → qa → gate → ledger → ci）：dev 交卷时后续回执还不存在或返工后必变，
 * 让某份回执在 `artifacts` 里钉同批其它回执的 sha = 要求「预见未来 sha」，结构上不可能稳定，
 * 每次返工都会必然失配（`ARTIFACT_SHA_MISMATCH`），还会掩盖真正的 artifact 问题。
 * 因此：这类条目**跳过存在性与 sha 比对**，由该回执自身的 present / verdict / handoff_id / spec_sha256
 * 校验负责（同一次 checkEvidence 会逐个校验每份回执，一条判据都没放宽）；跳过动作以 NOTE 呈现。
 *
 * 判定范围严格限定「本批 evidence 目录 + 回执命名」：
 *   `<dirRel>/evidence/<batch>/<dev|ci|qa>-<card>.json` 与 `<dirRel>/evidence/<batch>/{gate,ledger,ci,design}.json`
 * 其它路径（含其它批次的回执、普通产物）一律照旧做存在性 + sha 校验。
 */
export function isSiblingReceiptArtifact(P, batch, artifactPath) {
  const raw = String(artifactPath || '');
  if (!raw) return false;
  const rel = norm(relative(P.repo, resolve(P.repo, raw)));
  if (!rel || rel === '..' || rel.startsWith('../')) return false;
  const dir = norm(join(P.dirRel, 'evidence', batch.id)) + '/';
  if (!rel.startsWith(dir)) return false;
  const base = rel.slice(dir.length);
  if (!base || base.includes('/')) return false;
  if (/^(gate|ledger|ci|design)\.json$/.test(base)) return true;
  return /^(dev|ci|qa)-[^/]+\.json$/.test(base);
}
export function requiredEvidence(P, batch) {
  const req = [];
  for (const t of batch.tasks) {
    // 交卷方卡（dev/ci）：<role>-<card>.json + qa-<card>.json（QA 独立测试回执）
    if (t.role && isCardWorker(t.role)) { req.push({ file: `${t.role}-${t.card_id}.json`, role: t.role, card: t.card_id }); req.push({ file: `qa-${t.card_id}.json`, role: 'qa', card: t.card_id }); }
    else if (!['gate', 'ledger', 'design'].includes(t.role)) req.push({ file: `${t.role}-${t.card_id}.json`, role: t.role, card: t.card_id });
  }
  for (const r of BATCH_ROLES(P)) req.push({ file: `${r}.json`, role: r, card: null });
  const seen = new Set();
  return req.filter(x => (seen.has(x.file) ? false : seen.add(x.file)));
}
export function checkEvidence(P, st, batch) {
  const reasons = []; const rows = []; const notes = [];
  const dir = evDir(P, batch.id);
  const agentsByRole = {};
  for (const need of requiredEvidence(P, batch)) {
    const p = join(dir, need.file);
    const row = { file: need.file, role: need.role, present: existsSync(p) };
    rows.push(row);
    if (!row.present) { reasons.push('EVIDENCE_MISSING ' + batch.id + '/' + need.file); continue; }
    const j = readJSON(p);
    if (j.__error) { reasons.push('EVIDENCE_INVALID_JSON ' + need.file); continue; }
    row.agent = j.agent; row.verdict = j.verdict;
    const miss = EVIDENCE_FIELDS.filter(k => j[k] === undefined);
    if (miss.length) reasons.push('EVIDENCE_FIELDS_MISSING ' + need.file + ' ' + miss.join(','));
    if (j.role !== need.role) reasons.push('EVIDENCE_ROLE_MISMATCH ' + need.file);
    if (!new RegExp('^' + need.role + '-').test(j.agent || '')) reasons.push('EVIDENCE_AGENT_NOT_ROLE ' + need.file + ':' + j.agent);
    if (j.declared_by_lead !== false || j.independent !== true) reasons.push('NOT_INDEPENDENT ' + need.file);
    // **冒名签署（铁律，机器强制）**：回执必须由该工种自己的会话产出。
    // 事故（2026-11-19）：Lead 在 ci 工种回执未产出时自行写 evidence/<batch>/ci.json，
    // 并把 `declared_by_lead` 填成 false —— 事后看与真回执无法区分。
    // 因此要求回执显式自陈作者身份：`written_by` 必须存在且不等于 lead / lead-*。
    if (!j.written_by || /^lead(-|$)/.test(String(j.written_by))) {
      reasons.push('RECEIPT_WRITTEN_BY_LEAD ' + need.file + ' written_by=' + (j.written_by || '(缺)'));
    }
    if (j.agent === 'lead' || /^lead-/.test(String(j.agent || ''))) {
      reasons.push('AGENT_IDENTITY_FORGERY ' + need.file + ' agent=' + j.agent);
    }
    if (j.batch_id !== batch.id) reasons.push('EVIDENCE_BATCH_MISMATCH ' + need.file);
    if (need.card && j.card_id !== need.card) reasons.push('EVIDENCE_CARD_MISMATCH ' + need.file);
    if (j.spec_sha256 !== st.spec_sha256) reasons.push('EVIDENCE_STALE_SPEC ' + need.file);
    if (!['PASS', 'FAIL'].includes(j.verdict)) reasons.push('NO_VERDICT ' + need.file);
    if (!Array.isArray(j.command_exit_codes) || !j.command_exit_codes.length) reasons.push('NO_COMMAND_OUTPUT ' + need.file);
    if (!Array.isArray(j.artifacts) || !j.artifacts.length) reasons.push('NO_ARTIFACTS ' + need.file);
    for (const a of j.artifacts || []) {
      // 同批兄弟回执：顺序产生 + 返工必变 ⇒ 钉 sha 结构上不可能稳定，跳过存在性/sha 比对（NOTE 呈现）。
      // 这不是放宽：该回执的 present / verdict / handoff_id / spec_sha256 由本循环逐个校验。
      if (isSiblingReceiptArtifact(P, batch, a.path)) {
        notes.push('RECEIPT_ARTIFACT_SKIPPED ' + need.file + ' ' + a.path + '（同批回执的顺序产生/返工重写，其正确性由该回执自身 present/verdict/handoff_id/spec_sha256 校验负责）');
        continue;
      }
      const ap = resolve(P.repo, a.path || '');
      if (!a.path || !existsSync(ap)) reasons.push('ARTIFACT_MISSING ' + need.file + ' ' + a.path);
      else if (shaFile(ap) !== a.sha256) reasons.push('ARTIFACT_SHA_MISMATCH ' + need.file + ' ' + a.path);
    }
    if (need.card) {
      const card = batch.tasks.find(t => t.card_id === need.card)?.card;
      if (isCardWorker(need.role) && card && !agentMatchesSeat(j.agent, card.owner_agent)) reasons.push(String(need.role).toUpperCase() + '_AGENT_NOT_CARD_OWNER ' + need.file);
      if (need.role === 'qa' && card && !agentMatchesSeat(j.agent, card.qa_agent)) reasons.push('QA_AGENT_NOT_CARD_QA ' + need.file);
      if (need.role === 'dev' && isFrontendCard(P, card)) {
        const wg = checkWebGuidance(P, j, need.file);
        reasons.push(...wg.reasons);
        const qp = join(dir, `qa-${need.card}.json`);
        const qj = existsSync(qp) ? readJSON(qp) : {};
        const ver = qj.guidance_verified || [];
        for (const id of wg.guideIds) if (!ver.includes(id)) reasons.push('QA_DID_NOT_VERIFY_GUIDANCE ' + need.card + ' ' + id);
      }
      if (need.role === 'qa') reasons.push(...handoffProblems(P, batch, batch.tasks.find(t => t.card_id === need.card)));
      if (isCardWorker(need.role)) {
        if (!Array.isArray(j.files_changed)) reasons.push(String(need.role).toUpperCase() + '_NO_FILES_CHANGED ' + need.file);
        for (const f of j.files_changed || []) if (!inScopes(f, card?.write_scopes)) reasons.push('OUT_OF_WRITE_SCOPE ' + need.card + ' ' + f);
      }
    }
    (agentsByRole[j.agent] ||= new Set()).add(need.role);
  }
  for (const [a, rs] of Object.entries(agentsByRole)) if (rs.size > 1) reasons.push('AGENT_REUSE ' + a + ' ' + [...rs].join('+'));
  // 架构师：spec produced_by + 架构师签署
  const arch = readSignoff(P, 'architect');
  if (!arch.present || arch.spec_sha256 !== st.spec_sha256) reasons.push('ARCHITECT_SIGNOFF_MISSING_OR_STALE');
  if (arch.agent && agentsByRole[arch.agent]) reasons.push('ARCHITECT_ALSO_WORKER ' + arch.agent);
  // 漂移：业务改动必须被本批 dev 回执的 files_changed 覆盖
  const drift = checkDrift(P, st, batch);
  reasons.push(...drift.reasons);
  return { reasons, rows, notes, drift };
}
export function checkDrift(P, st, batch) {
  const reasons = [];
  let changed = changedBizFiles(P, batch?.batch_base || batch?.base_head || null);
  if (changed === null) return { reasons: ['GIT_UNAVAILABLE'], changed: [] };
  // 并行批次：其他批次（同期在做或本批开工后才交付的）自己的 write_scopes 由它自己的回执认领，这里跳过
  const others = batch ? concurrentScopes(st, batch) : [];
  changed = changed.filter(f => !inScopes(f, others));
  const claimed = new Map();
  if (batch) {
    for (const t of batch.tasks.filter(t => isCardWorker(t.role))) {
      const p = join(evDir(P, batch.id), `${t.role}-${t.card_id}.json`);
      if (!existsSync(p)) continue;
      const j = readJSON(p);
      for (const f of j.files_changed || []) claimed.set(f, t);
    }
  }
  for (const f of changed) {
    const t = claimed.get(f);
    if (!t) reasons.push('UNCLAIMED_BUSINESS_CHANGE ' + f);
    else if (!inScopes(f, t.card.write_scopes)) reasons.push('OUT_OF_WRITE_SCOPE ' + t.card_id + ' ' + f);
  }
  return { reasons, changed };
}

// ---------- 并行批次 ----------
export const ACTIVE = ['dispatched', 'evidence_collected', 'accepted', 'released'];
export const activeBatches = (st) => (st?.batches || []).filter(b => ACTIVE.includes(b.status));
const batchScopes = (b) => b.tasks.flatMap(t => t.card.write_scopes || []);
/** 与 batch 同期的其他批次的写入范围：正在进行的，或在 batch 开工之后才交付的 */
export function concurrentScopes(st, batch) {
  const since = batch.firstDispatchedAt || '';
  return (st?.batches || []).filter(b => b.id !== batch.id && (ACTIVE.includes(b.status) || (b.deliveredAt && b.deliveredAt >= since) || (b.status === 'reopened' && b.firstDispatchedAt)))
    .flatMap(batchScopes);
}
export const activeScopes = (st, exceptId) => activeBatches(st).filter(b => b.id !== exceptId).flatMap(batchScopes);
const batchAgents = (b) => b.tasks.flatMap(t => [t.card.owner_agent, t.card.qa_agent]).filter(Boolean);
/** 能否与当前在做的批次并行开工 */
export function parallelConflicts(P, st, b) {
  const reasons = [];
  const act = activeBatches(st).filter(x => x.id !== b.id);
  if (act.length >= cfg(P).parallel.max_batches) reasons.push('PARALLEL_LIMIT 同时在做 ' + act.map(x => x.id).join(',') + '（parallel.max_batches=' + cfg(P).parallel.max_batches + '）');
  for (const o of act) {
    for (const a of batchScopes(b)) for (const c of batchScopes(o)) if (inScopes(a, [c]) || inScopes(c, [a])) reasons.push(`PARALLEL_SCOPE_CONFLICT ${b.id}:${a} × ${o.id}:${c}`);
    const ag = new Set(batchAgents(o));
    for (const a of batchAgents(b)) if (ag.has(a)) reasons.push(`AGENT_BUSY ${a} 正在批次 ${o.id} 干活`);
    const oc = new Set(o.tasks.map(t => t.card_id));
    for (const t of b.tasks) if (oc.has(t.card_id)) reasons.push(`CARD_TAKEN ${t.card_id} 已在批次 ${o.id}`);
  }
  return reasons;
}

// ---------- 交卷 → 测试 交接（测试只能在交卷方（dev/ci）交付、代码冻结之后开始） ----------
// 回执错误码按交卷方角色大写前缀动态拼接；下面列全字面量，供 gen-docs 扫描进错误码目录并与手写文档对账：
// 'DEV_RECEIPT_MISSING' 'DEV_AGENT_NOT_CARD_OWNER' 'DEV_NOT_PASS' 'DEV_NO_FILES_CHANGED' 'DEV_RECEIPT_CHANGED_AFTER_HANDOFF' 'DEV_CHANGED_AFTER_HANDOFF'
// 'CI_RECEIPT_MISSING' 'CI_AGENT_NOT_CARD_OWNER' 'CI_NOT_PASS' 'CI_NO_FILES_CHANGED' 'CI_RECEIPT_CHANGED_AFTER_HANDOFF' 'CI_CHANGED_AFTER_HANDOFF'
const fileSha = (P, f) => { const p = resolve(P.repo, f); return existsSync(p) ? shaFile(p) : null; };
/** 交卷回执检查（交卷方角色通用：文件名 `<role>-<card_id>.json`，role 取自任务而不是写死 dev） */
export function workerReceiptProblems(P, st, batch, t) {
  const R = String(t.role || '').toUpperCase();
  const f = `${t.role}-${t.card_id}.json`; const p = join(evDir(P, batch.id), f);
  if (!existsSync(p)) return [R + '_RECEIPT_MISSING ' + f];
  const j = readJSON(p); const r = [];
  if (j.__error) return ['EVIDENCE_INVALID_JSON ' + f];
  if (j.role !== t.role || !agentMatchesSeat(j.agent, t.card.owner_agent)) r.push(R + '_AGENT_NOT_CARD_OWNER ' + f);
  if (j.card_id !== t.card_id || j.batch_id !== batch.id) r.push('EVIDENCE_CARD_MISMATCH ' + f);
  if (j.spec_sha256 !== st.spec_sha256) r.push('EVIDENCE_STALE_SPEC ' + f);
  if (j.verdict !== 'PASS') r.push(R + '_NOT_PASS ' + f);
  if (!Array.isArray(j.files_changed) || !j.files_changed.length) r.push(R + '_NO_FILES_CHANGED ' + f);
  for (const x of j.files_changed || []) if (!inScopes(x, t.card.write_scopes)) r.push('OUT_OF_WRITE_SCOPE ' + t.card_id + ' ' + x);
  return r;
}
/** 兼容旧名（语义已泛化到所有交卷方角色） */
export const devReceiptProblems = workerReceiptProblems;
export function makeHandoff(P, batch, t) {
  const receiptFile = `${t.role}-${t.card_id}.json`;
  const p = join(evDir(P, batch.id), receiptFile);
  const j = readJSON(p);
  const files = Object.fromEntries((j.files_changed || []).map(f => [f, fileSha(P, f)]));
  const at = new Date().toISOString();
  const nonce = createHash('sha256').update(String(Math.random()) + at + process.pid).digest('hex').slice(0, 16);
  const h = { at, role: t.role, card_id: t.card_id, receipt_file: receiptFile, receipt_sha256: shaFile(p), files };
  h.handoff_id = sha(JSON.stringify(h) + nonce).slice(0, 24);
  return h;
}
/** 交接后交卷方又改了代码 / 回执，或测试回执不是针对本次交接 → 测试结果作废 */
export function handoffProblems(P, batch, t) {
  const R = String(t.role || '').toUpperCase();
  const h = (batch.handoffs || {})[t.card_id];
  if (!h) return ['QA_WITHOUT_HANDOFF ' + t.card_id + '（' + t.role + ' 交卷后由 Lead 跑 handoff --card ' + t.card_id + '，测试才能开始）'];
  const r = [];
  const dp = join(evDir(P, batch.id), `${t.role}-${t.card_id}.json`);
  const receiptSha = h.receipt_sha256 || h.dev_receipt_sha256;
  if (!existsSync(dp) || shaFile(dp) !== receiptSha) r.push(R + '_RECEIPT_CHANGED_AFTER_HANDOFF ' + t.card_id + '（重新 handoff，测试重做）');
  for (const [f, s0] of Object.entries(h.files)) if (fileSha(P, f) !== s0) r.push(R + '_CHANGED_AFTER_HANDOFF ' + t.card_id + ' ' + f + '（测试期间 ' + t.role + ' 改了代码，测试作废）');
  const qp = join(evDir(P, batch.id), `qa-${t.card_id}.json`);
  const q = existsSync(qp) ? readJSON(qp) : null;
  if (q && q.handoff_id !== h.handoff_id) r.push('QA_NOT_FOR_CURRENT_HANDOFF ' + t.card_id + '（测试回执须带本次交接的 handoff_id）');
  return r;
}

// ---------- 五层验收 ----------
export function fiveLayers(P, st, batch) {
  const C = cfg(P);
  const reasons = [];
  const dir = evDir(P, batch.id);
  const ev = (f) => (existsSync(join(dir, f)) ? readJSON(join(dir, f)) : null);
  const exitsOk = (j) => (j?.command_exit_codes || []).length > 0 && j.command_exit_codes.every(c => c.exit_code === 0);
  const pass = (j) => j && j.verdict === 'PASS' && exitsOk(j);
  const workerCards = batch.tasks.filter(t => isCardWorker(t.role)).map(t => t.card_id);
  const qa = workerCards.map(c => [c, ev(`qa-${c}.json`)]);
  const layer = (id, ok, why, extra = {}) => { if (!ok) reasons.push('LAYER_FAIL ' + id + ' ' + why); return { layer: id, pass: ok, why: ok ? '' : why, ...extra }; };
  const L = [];
  const qaCover = (k) => qa.every(([, j]) => pass(j) && (j.layers_covered || []).map(layerId).includes(k));
  L.push(layer('event', workerCards.length === 0 || qaCover('event'), 'qa 回执需 PASS、exit 0 且 layers_covered 含 event（或「事件」）'));
  const rx = C.rule_id_pattern ? new RegExp(C.rule_id_pattern) : null;
  const rulesOk = qa.every(([, j]) => (j?.rule_ids || []).length > 0 && (!rx || j.rule_ids.every(r => rx.test(r))));
  L.push(layer('logic', workerCards.length === 0 || (qaCover('logic') && rulesOk), 'qa 回执需覆盖 logic 并列出 rule_ids' + (rx ? '（须匹配 ' + C.rule_id_pattern + '）' : '')));
  const gate = ev('gate.json');
  const cmds = gate?.command_exit_codes || [];
  const missing = C.gates.required_commands.filter(rc => !cmds.some(c => (c.cmd || '').includes(rc) && c.exit_code === 0));
  const gateOk = pass(gate) && !missing.length && (!C.gates.require_no_mock || gate.mock_used === false);
  L.push(layer('interface', gateOk && (workerCards.length === 0 || qaCover('interface')),
    'gate 需逐条跑 config.gates.required_commands 且 exit 0' + (missing.length ? '（缺：' + missing.join(' | ') + '）' : '') + (C.gates.require_no_mock ? '、mock_used=false' : '') + '；qa 覆盖 interface'));
  const led = ev('ledger.json');
  let ledOk = pass(led);
  let ledWhy = 'ledger 回执需 PASS、exit 0，并附台账 JSON';
  if (ledOk) {
    const art = (led.artifacts || []).find(a => /\.json$/.test(a.path || ''));
    const j = art && existsSync(resolve(P.repo, art.path)) ? readJSON(resolve(P.repo, art.path)) : null;
    const lc = ledgerCheck(P, art, j);
    ledOk = lc.ok; if (!lc.ok) ledWhy = lc.why;
  }
  L.push(layer('state', ledOk, ledWhy));
  if (workerRoles(P).includes('design')) {
    const des = ev('design.json');
    const desOk = pass(des) && (des.artifacts || []).some(a => /\.(png|jpe?g|webp)$/i.test(a.path || '')) && (des.prototype_refs || []).length > 0;
    L.push(layer('view', desOk, 'design 需附截图并列出 prototype_refs（原型/设计稿的具体状态）'));
  } else L.push({ layer: 'view', pass: true, skipped: 'config.roles.design=false' });
  const ov = overallOk(P);
  if (!ov.ok) reasons.push('OVERALL_FALSE ' + ov.file + ' ' + C.release.overall_pointer);
  return { reasons, layers: L, overall: ov };
}

/** 机器生成的文件禁止手改：重跑 config.generated 里每条生成命令，结果与仓里的文件逐字节一致（换行统一为 LF）才算数；跑完恢复原文件 */
export function generatedCheck(P) {
  const reasons = [];
  const lf = (b) => b.toString('utf8').replace(/\r\n/g, '\n');
  for (const g of cfg(P).generated || []) {
    const p = resolve(P.repo, g.path);
    if (!existsSync(p)) { reasons.push('GENERATED_FILE_MISSING ' + g.path); continue; }
    const before = readFileSync(p);
    try {
      execFileSync(g.command, { cwd: P.repo, stdio: 'ignore', shell: true });
      if (!existsSync(p)) reasons.push('GENERATED_COMMAND_DID_NOT_WRITE ' + g.path);
      else if (lf(readFileSync(p)) !== lf(before)) reasons.push('GENERATED_HAND_EDITED ' + g.path + '（重跑 ' + g.command + ' 结果不同：被手改或没重新生成。改源头再生成，不许手改）');
    } catch { reasons.push('GENERATED_COMMAND_FAILED ' + g.command); }
    finally { writeFileSync(p, before); }
  }
  return reasons;
}

/** 台账自洽 + （配置了 ledger.command 时）重跑生成命令比对内容，防手写台账 */
export const selfShaOk = (j) => { const c = { ...j }; const w = c.self_sha256; delete c.self_sha256; return !!w && sha(JSON.stringify(c)) === w; };
const ledgerBody = (j) => { const c = { ...j }; delete c.self_sha256; delete c.generatedAt; return JSON.stringify(c); };
export function ledgerCheck(P, art, j) {
  if (!j || !j.schema || !j.generatedAt) return { ok: false, why: '台账 JSON 缺 schema/generatedAt' };
  if (!selfShaOk(j)) return { ok: false, why: 'LEDGER_SELF_SHA_INVALID：self_sha256 ≠ sha256(JSON.stringify(去掉 self_sha256 的对象))' };
  const cmd = cfg(P).ledger.command;
  if (!cmd) return { ok: true, unverified: true };
  const ap = resolve(P.repo, art.path); const before = readFileSync(ap);
  try {
    execFileSync(cmd, { cwd: P.repo, stdio: 'ignore', shell: true });
    const regen = readJSON(ap);
    if (regen.__error) return { ok: false, why: 'LEDGER_REGEN_INVALID ' + cmd };
    if (ledgerBody(regen) !== ledgerBody(j)) return { ok: false, why: 'LEDGER_REGEN_MISMATCH：重跑 ' + cmd + ' 得到的台账与回执附件不同（疑似手写或过期）' };
    return { ok: true };
  } catch (e) { return { ok: false, why: 'LEDGER_COMMAND_FAILED exit=' + (e.status ?? '?') + ' ' + cmd }; }
  finally { writeFileSync(ap, before); }
}

// ---------- 远端 CI（强制项，无后门） ----------
/** HEAD 是否真的在某个远端分支上（git branch -r --contains）。没推过 ⇒ 远端 CI 不可能验证它 */
export function pushedRefs(repo, head) {
  try {
    const raw = execFileSync('git', ['branch', '-r', '--contains', head], { cwd: repo, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
    const refs = raw.split('\n').map(s => s.replace(/^\*?\s+/, '').trim()).filter(Boolean).filter(r => !r.includes('->'));
    return refs;
  } catch { return []; }
}

export function ciStatus(P) {
  const repo = P.repo; const C = cfg(P).ci;
  const head = gitHead(repo);
  if (!head) return { ok: false, reason: 'GIT_UNAVAILABLE' };
  // 强制项：provider 只能是 github。command/none 在配置校验阶段就被拒（CONFIG_CI_BYPASS_FORBIDDEN）
  if (C.provider !== 'github') return { ok: false, reason: 'CI_BYPASS_FORBIDDEN：ci.provider=' + C.provider + '（只允许 github，CI 不得绕开）' };
  if (!C.repo) return { ok: false, reason: 'CI_REPO_MISSING：ci.repo 未配置' };
  // 1) 必须已经推到远端：未推送的提交，远端 CI 根本没跑过，不许当通过
  const refs = pushedRefs(repo, head);
  if (!refs.length) return { ok: false, head, pushed: false, reason: 'CI_NOT_PUSHED：HEAD ' + head.slice(0, 8) + ' 不在任何远端分支上（先 push + 开 PR，CI 才会跑）' };
  // 2) 查该提交的远端 CI 运行记录
  let raw;
  try {
    raw = execFileSync('gh', ['run', 'list', '--repo', C.repo, '--commit', head,
      '--json', 'conclusion,status,workflowName,headSha,url', '--limit', '50'],
      { cwd: repo, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], shell: process.platform === 'win32' });
  } catch (e) { return { ok: false, head, pushed: true, reason: 'CI_UNVERIFIABLE: gh 不可用或未登录（' + String(e.message || e).split('\n')[0] + '）' }; }
  let runs; try { runs = JSON.parse(raw); } catch { return { ok: false, reason: 'CI_OUTPUT_INVALID' }; }
  runs = runs.filter(r => !r.headSha || r.headSha === head);
  if (!runs.length) return { ok: false, head, pushed: true, reason: 'CI_NO_RUN_FOR_HEAD ' + head.slice(0, 8) + '（已推送但没有 CI 运行记录：workflow 可能没监听该分支）' };
  const bad = runs.filter(r => r.status !== 'completed' || r.conclusion !== 'success');
  if (bad.length) return { ok: false, head, pushed: true, runs: runs.length, reason: 'CI_NOT_SUCCESS ' + bad.map(r => r.workflowName + ':' + (r.conclusion || r.status)).join(',') };
  return { ok: true, head, pushed: true, runs: runs.length, refs, runs_detail: runs.map(r => ({ name: r.workflowName, conclusion: r.conclusion, url: r.url })) };
}

/**
 * ci 工种回执校验（<batch>/ci.json）。release 前**必须**有它，且必须与当前 HEAD、真远端运行对得上。
 * 无后门：缺失 / 未推送 / 结论非 success / head 不符 / 自报 bypass 都判红。
 */
export function ciReceiptCheck(P, batch) {
  const f = join(P.evidence, batch.id, 'ci.json');
  if (!existsSync(f)) return { ok: false, reason: 'CI_RECEIPT_MISSING：缺少 ci 工种回执 evidence/' + batch.id + '/ci.json（CI 是强制项，无回执不算发布）' };
  const j = readJSON(f);
  if (j.__error) return { ok: false, reason: 'CI_RECEIPT_INVALID_JSON' };
  if (j.role !== 'ci') return { ok: false, reason: 'CI_RECEIPT_ROLE ' + j.role };
  if (j.verdict !== 'PASS') return { ok: false, reason: 'CI_RECEIPT_NOT_PASS verdict=' + j.verdict };
  if (j.bypass_detected === true) return { ok: false, reason: 'CI_BYPASS_ATTEMPT：回执自报绕开（bypass_detected=true）' };
  if (j.provider !== 'github') return { ok: false, reason: 'CI_BYPASS_FORBIDDEN：回执 provider=' + j.provider };
  const head = gitHead(P.repo);
  if (!head) return { ok: false, reason: 'GIT_UNAVAILABLE' };
  const recHead = j.pushed && j.pushed.head_sha;
  if (!j.pushed || !j.pushed.remote || !recHead) return { ok: false, reason: 'CI_RECEIPT_NO_PUSH：回执缺 pushed.{remote,branch,head_sha}' };
  if (String(recHead) !== head) return { ok: false, reason: 'CI_HEAD_MISMATCH：回执 head_sha=' + String(recHead).slice(0, 8) + ' ≠ 当前 HEAD ' + head.slice(0, 8) };
  const refs = pushedRefs(P.repo, head);
  if (!refs.length) return { ok: false, reason: 'CI_NOT_PUSHED：当前 HEAD 不在任何远端分支上' };
  const runs = Array.isArray(j.workflow_runs) ? j.workflow_runs : [];
  if (!runs.length) return { ok: false, reason: 'CI_RECEIPT_NO_RUNS：回执必须列出被核对的 CI 运行' };
  const bad = runs.filter(r => String(r.head_sha) !== head || r.conclusion !== 'success');
  if (bad.length) return { ok: false, reason: 'CI_NOT_SUCCESS ' + bad.map(r => (r.name || '?') + ':' + (r.conclusion || '?')).join(',') };
  return { ok: true, head, runs: runs.length, repo: cfg(P).ci.repo };
}


/**
 * **合并前置门（铁律，机器强制）**：合并 PR 之前必须先跑 `loop.mjs premerge --batch <id>` 且 exit 0。
 *
 * 事故来源（2026-11-19）：Lead 在 **ci 工种回执尚未产出**、批次尚未 accept 的情况下就合并了 PR，
 * 事后又补写 ci.json（并把 `declared_by_lead` 填成 false）——顺序倒置等于绕开 CI。
 * 该行为**不可检测于事后**（回执看起来齐全），因此把"合并"这件事本身变成必须过门的动作：
 * 只要 premerge 不过，合并就是违规。
 *
 * 判据（任何一条不满足都拒绝合并）：
 *   1. 批次状态必须是 accepted / released / delivered（否则 MERGE_BEFORE_ACCEPT）
 *   2. `evidence/<batch>/ci.json` 必须存在且通过 `ciReceiptCheck`（否则 MERGE_BEFORE_CI_RECEIPT）
 *   3. 当前 HEAD 必须已在远端且该 head 的全部 CI run success（否则 MERGE_CI_NOT_GREEN）
 */
export function premergeCheck(P, st, batch) {
  const reasons = [];
  const b = batch || null;
  if (!b) {
    reasons.push('MERGE_NO_BATCH：找不到批次（并行时必须 --batch <id>）');
  } else if (!['accepted', 'released', 'delivered'].includes(b.status)) {
    reasons.push('MERGE_BEFORE_ACCEPT：批次 ' + b.id + ' 状态=' + b.status + '（必须先 accept，合并不得早于验收）');
  }
  const cr = ciReceiptCheck(P, b || { id: '\u0000missing\u0000' });
  if (!cr.ok) reasons.push('MERGE_BEFORE_CI_RECEIPT：' + cr.reason);
  const cs = ciStatus(P);
  if (!cs.ok) reasons.push('MERGE_CI_NOT_GREEN：' + cs.reason);
  return {
    ok: reasons.length === 0,
    reasons,
    batch: b && b.id,
    head: gitHead(P.repo),
    runs: cs.ok ? cs.runs : undefined,
  };
}

/** 指定 --batch 时取它；否则取唯一一个处在进行中的批次（并行时必须指定） */
export function currentBatch(st, id) {
  if (id) return st?.batches?.find(b => b.id === id) || null;
  const act = activeBatches(st);
  if (act.length === 1) return act[0];
  return st?.batches?.find(b => b.id === st.batch) || null;
}
