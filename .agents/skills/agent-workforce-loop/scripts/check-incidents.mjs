#!/usr/bin/env node
/**
 * 失误台账校验器（有牙）。由 validate-skill.mjs 调用，也可单独跑。
 *
 * 判据（任一不满足即 FAIL，exit 1）：
 *  1. schema / incidents 数组 / evidence 非空 / id 形如 INC-001 且唯一
 *  2. kind、status 必须在白名单
 *  3. 每条 incident 至少一个 guard（不许只记录不设防 ⇒ INCIDENT_WITHOUT_ENFORCEMENT）
 *  4. guard[].code 必须在 scripts/*.mjs 里真实存在（与 gen-docs 同一套扫描口径）
 *  5. guard[].in 必须是 scripts/ 下真实文件名；guard[].selftest 非空时必须在 selftest.mjs 里找到同名用例
 *  6. status=accepted_risk 时 accepted_risk.rationale+review_by 必填，review_by 过期即 FAIL
 *  7. references/generated/incidents.md 必须与 incidents.json 机器生成结果逐字节一致（INCIDENTS_DOC_DRIFT）
 */
import { readFileSync, existsSync, readdirSync } from 'node:fs';
import { join, dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { buildIncidentsDoc } from './gen-incidents.mjs';

const SKILL = resolve(join(dirname(fileURLToPath(import.meta.url)), '..'));
const KINDS = ['PREMATURE_MERGE', 'IDENTITY_FORGERY', 'CIRCULAR_RECEIPT', 'PATH_PARSE_BUG', 'CRITERIA_UNCHECKABLE',
  'METRIC_GAMING', 'SPEC_CHANGED_IN_FLIGHT', 'DEAD_SCOPE_PATH', 'INTENTIONAL_BYPASS', 'GATE_UNUSABLE_TIMING', 'GATE_BYPASS_BY_SEQUENCE', 'GATE_BOOTSTRAP', 'SCOPE_MISSING_REQUIRED_CALLSITE', 'CRITERIA_AGGREGATION_AMBIGUOUS'];
const STATUSES = ['enforced', 'accepted_risk'];
const reasons = [];

const incPath = join(SKILL, 'incidents.json');
if (!existsSync(incPath)) {
  console.log(JSON.stringify({ verdict: 'FAIL', reasons: ['INCIDENTS_JSON_MISSING'] }, null, 2));
  process.exit(1);
}
let j;
try { j = JSON.parse(readFileSync(incPath, 'utf8').replace(/^\uFEFF/, '')); }
catch (e) { console.log(JSON.stringify({ verdict: 'FAIL', reasons: ['INCIDENTS_JSON_INVALID ' + String(e.message || e).split('\n')[0]] }, null, 2)); process.exit(1); }
if (j.schema !== 'agent-workforce-incidents/v1') reasons.push('INCIDENT_SCHEMA_INVALID ' + j.schema);
const list = Array.isArray(j.incidents) ? j.incidents : null;
if (!list) reasons.push('INCIDENT_SCHEMA_INVALID incidents[] 缺失');
const incidents = list || [];

// 代码扫描口径：与 gen-docs 一致（排除 selftest / gen-docs / machine）
const scripts = readdirSync(join(SKILL, 'scripts')).filter(f => f.endsWith('.mjs') && !['selftest.mjs', 'gen-docs.mjs', 'machine.mjs'].includes(f)).sort();
const codeSet = new Set();
for (const f of scripts) {
  const src = readFileSync(join(SKILL, 'scripts', f), 'utf8');
  for (const m of src.matchAll(/['`](?:[A-Z]+_)?([A-Z][A-Z0-9]*(?:_[A-Z0-9]+)+)\b/g)) codeSet.add(m[0].slice(1));
}
const scriptNames = new Set(readdirSync(join(SKILL, 'scripts')).filter(f => f.endsWith('.mjs')));
const selftestSrc = readFileSync(join(SKILL, 'scripts', 'selftest.mjs'), 'utf8');
const now = new Date();
const today = now.toISOString().slice(0, 10);
const seen = new Set();
const rows = [];
let guardCount = 0;

incidents.forEach((inc, idx) => {
  const id = inc && inc.id;
  const tag = id || ('#' + idx);
  if (!id || !/^INC-\d{3}$/.test(String(id))) reasons.push('INCIDENT_ID_INVALID ' + tag);
  else if (seen.has(id)) reasons.push('INCIDENT_ID_DUPLICATE ' + id);
  else seen.add(id);
  if (!KINDS.includes(inc.kind)) reasons.push('INCIDENT_KIND_UNKNOWN ' + tag + ' ' + inc.kind);
  if (!STATUSES.includes(inc.status)) reasons.push('INCIDENT_STATUS_INVALID ' + tag + ' ' + inc.status);
  if (!Array.isArray(inc.evidence) || !inc.evidence.length) reasons.push('INCIDENT_EVIDENCE_EMPTY ' + tag);
  const guards = Array.isArray(inc.guard) ? inc.guard : [];
  if (!guards.length) reasons.push('INCIDENT_WITHOUT_ENFORCEMENT ' + tag);
  for (const g of guards) {
    guardCount++;
    const code = String((g && g.code) || '');
    if (!codeSet.has(code)) reasons.push('INCIDENT_GUARD_CODE_UNKNOWN ' + tag + ' ' + code);
    if (!scriptNames.has(String((g && g.in) || ''))) reasons.push('INCIDENT_GUARD_LOCATION_INVALID ' + tag + ' ' + (g && g.in));
    const st = String((g && g.selftest) || '');
    if (st && !selftestSrc.includes(st)) reasons.push('INCIDENT_SELFTEST_MISSING ' + tag + ' ' + st);
  }
  if (inc.status === 'accepted_risk') {
    const ar = inc.accepted_risk || {};
    if (!ar.rationale || !/^\d{4}-\d{2}-\d{2}$/.test(String(ar.review_by || ''))) reasons.push('INCIDENT_RISK_REVIEW_MISSING ' + tag);
    else if (String(ar.review_by) < today) reasons.push('INCIDENT_RISK_REVIEW_EXPIRED ' + tag + ' review_by=' + ar.review_by + ' today=' + today);
  }
  rows.push({ id: id || tag, kind: inc.kind, status: inc.status, guards: guards.map(g => (g && g.code) || '?'), selftests: guards.map(g => (g && g.selftest) || '').filter(Boolean) });
});

// 生成物与数据一致（逐字节）
const built = buildIncidentsDoc(SKILL);
if (built.problems.length) reasons.push(...built.problems);
else {
  const target = join(SKILL, built.path);
  if (!existsSync(target)) reasons.push('INCIDENTS_DOC_DRIFT ' + built.path + '（缺生成物；重跑 node scripts/gen-incidents.mjs）');
  else if (readFileSync(target, 'utf8').replace(/\r\n/g, '\n') !== built.content) reasons.push('INCIDENTS_DOC_DRIFT ' + built.path + '（手改或未重生成；重跑 node scripts/gen-incidents.mjs）');
}

console.log(JSON.stringify({ verdict: reasons.length ? 'FAIL' : 'PASS', incidents: incidents.length, guards: guardCount, schemas: { kinds: KINDS.length, statuses: STATUSES.length }, reasons, rows }, null, 2));
process.exit(reasons.length ? 1 : 0);
