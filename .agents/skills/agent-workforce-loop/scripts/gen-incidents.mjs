#!/usr/bin/env node
/**
 * 失误台账 → 机器文档（散文由机器产出，禁手写）：
 *   incidents.json  →  references/generated/incidents.md
 * 用法：node scripts/gen-incidents.mjs          重新生成
 *       node scripts/gen-incidents.mjs --check  只比对不写；有差异 exit 1
 * gen-docs.mjs 会 import 本文件的 buildIncidentsDoc()，把生成物纳入同一套
 * 「--check 逐字节校验 + MANIFEST sha256」链路 ⇒ 手改生成物即红。
 */
import { readFileSync, writeFileSync, existsSync, mkdirSync } from 'node:fs';
import { join, dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const SKILL = resolve(join(dirname(fileURLToPath(import.meta.url)), '..'));
const OUT_REL = 'references/generated/incidents.md';
const HEAD = '<!-- GENERATED-FROM: incidents.json —— 由 scripts/gen-incidents.mjs 生成，禁止手改；gen-docs.mjs --check 逐字节校验 -->\n\n';
const lf = (s) => s.replace(/\r\n/g, '\n');

/** 纯函数：读 incidents.json，产出 { path, content, problems }。gen-docs 与 CLI 共用。 */
export function buildIncidentsDoc(skillDir = SKILL) {
  const problems = [];
  const p = join(skillDir, 'incidents.json');
  if (!existsSync(p)) return { path: OUT_REL, content: '', problems: ['INCIDENTS_JSON_MISSING'] };
  let j;
  try { j = JSON.parse(readFileSync(p, 'utf8').replace(/^\uFEFF/, '')); }
  catch (e) { return { path: OUT_REL, content: '', problems: ['INCIDENTS_JSON_INVALID ' + String(e.message || e).split('\n')[0]] }; }
  const list = Array.isArray(j.incidents) ? j.incidents : [];
  const cell = (x) => String(x == null ? '' : x).replace(/\|/g, '\\|').replace(/\r?\n/g, ' ');
  const rows = list.map((inc) => {
    const codes = (inc.guard || []).map((g) => '`' + (g.code || '?') + '`').join('<br>') || '—';
    const tests = (inc.guard || []).map((g) => (g.selftest ? '`' + g.selftest + '`' : '—')).join('<br>') || '—';
    return `| ${cell(inc.id)} | ${cell(inc.kind)} | ${cell(inc.symptom)} | ${codes} | ${tests} | ${cell(inc.status)} |`;
  });
  const content = HEAD +
    '# 失误台账与进化（生成）\n\n' +
    '唯一真源 = `incidents.json`。本表由 `scripts/gen-incidents.mjs` 机器生成，**禁止手改**；' +
    '`node scripts/gen-docs.mjs --check` 会逐字节校验（手改即红），`node scripts/check-incidents.mjs` 另做结构/守卫/期限校验。\n\n' +
    '| id | kind | 现象（客观） | 守卫 code | selftest 用例 | 状态 |\n' +
    '|---|---|---|---|---|---|\n' +
    (rows.length ? rows.join('\n') : '| _（空）_ | — | — | — | — | — |') + '\n';
  return { path: OUT_REL, content, problems };
}

const IS_MAIN = (() => { try { return resolve(process.argv[1] || '') === fileURLToPath(import.meta.url); } catch { return false; } })();
if (IS_MAIN) {
const CHECK = process.argv.includes('--check');
const { path: OUT_REL2, content, problems } = buildIncidentsDoc(SKILL);
const target = join(SKILL, OUT_REL2);
const local = [...problems];
if (!local.length) {
  if (CHECK) {
    if (!existsSync(target)) local.push('GENERATED_DOC_MISSING ' + OUT_REL2);
    else if (lf(readFileSync(target, 'utf8')) !== content) local.push('GENERATED_DOC_HAND_EDITED ' + OUT_REL2 + '（禁止手改；改 incidents.json 后重跑 node scripts/gen-incidents.mjs）');
  } else { mkdirSync(dirname(target), { recursive: true }); writeFileSync(target, content); }
}
console.log(JSON.stringify({ verdict: local.length ? 'FAIL' : 'PASS', mode: CHECK ? 'check' : 'write', generated: [OUT_REL2], incidents: (JSON.parse(existsSync(join(SKILL, 'incidents.json')) ? readFileSync(join(SKILL, 'incidents.json'), 'utf8') : '{"incidents":[]}').incidents || []).length, problems: local }, null, 2));
process.exit(local.length ? 1 : 0);
}
