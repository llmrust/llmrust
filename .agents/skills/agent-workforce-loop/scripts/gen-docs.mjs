#!/usr/bin/env node
/**
 * 生成 skill 自己的机器文档，禁止手改：
 *   references/generated/state-machine.md  ← scripts/machine.mjs 的 STATES / BATCH_STATES
 *   references/generated/config-fields.md  ← lib.mjs 的 DEFAULT_CONFIG + machine.mjs 的 CONFIG_DOCS
 *   references/generated/reason-codes.md   ← 扫描 scripts/*.mjs 里实际会输出的错误码
 *   assets/config.template.json            ← DEFAULT_CONFIG
 *   MANIFEST.json                          ← 包内每个文件的 sha256
 * 用法：node scripts/gen-docs.mjs          重新生成
 *       node scripts/gen-docs.mjs --check  只比对，不写；有差异 exit 1（validate-skill 调用）
 * 另外检查手写文档和代码对得上：文档里提到的 loop 命令、错误码必须真实存在。
 */
import { readFileSync, writeFileSync, existsSync, mkdirSync, readdirSync, statSync } from 'node:fs';
import { join, dirname, resolve, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import * as M from './machine.mjs';
import { DEFAULT_CONFIG } from './lib.mjs';
import { buildIncidentsDoc } from './gen-incidents.mjs';

const SKILL = resolve(join(dirname(fileURLToPath(import.meta.url)), '..'));
const CHECK = process.argv.includes('--check');
const HEAD = '<!-- 由 scripts/gen-docs.mjs 生成，禁止手改。改 scripts/machine.mjs 或 lib.mjs 后重跑 node scripts/gen-docs.mjs -->\n\n';
const rel = (p) => relative(SKILL, p).split(sep).join('/');
const sha = (b) => createHash('sha256').update(b).digest('hex');
const lf = (s) => s.replace(/\r\n/g, '\n');
const problems = [];
const outFiles = {};

// ---------- 状态机 ----------
const row = (k, v) => `| \`${k}\` | ${v.allowed.length ? v.allowed.map(a => '`' + a + '`').join('、') : '—'} | ${v.checks} |`;
outFiles['references/generated/state-machine.md'] = HEAD +
  '# 状态机（生成）\n\n## 全局状态\n\n| 状态 | 允许动作 | 脚本内部检查 |\n|---|---|---|\n' +
  Object.entries(M.STATES).map(([k, v]) => row(k, v)).join('\n') +
  '\n\n## 批次状态\n\n批次动作用 `--batch <id>` 指定；只有一个批次能做该动作时可省略。\n\n| 批次状态 | 允许动作 | 脚本内部检查 |\n|---|---|---|\n' +
  Object.entries(M.BATCH_STATES).map(([k, v]) => row(k, v)).join('\n') +
  '\n\n只读命令（任何状态可跑）：' + M.READONLY.map(c => '`' + c + '`').join('、') + '。只有架构师（--as architect-*）能跑：' + M.ARCHITECT_ONLY.map(c => '`' + c + '`').join('、') + '。\n';

// ---------- 配置字段 ----------
const flat = (o, pre = '') => Object.entries(o).flatMap(([k, v]) =>
  v && typeof v === 'object' && !Array.isArray(v) ? flat(v, pre + k + '.') : [[pre + k, v]]);
const fields = flat(DEFAULT_CONFIG);
for (const [k] of fields) if (!M.CONFIG_DOCS[k]) problems.push('CONFIG_FIELD_UNDOCUMENTED ' + k + '（在 machine.mjs 的 CONFIG_DOCS 补说明）');
for (const k of Object.keys(M.CONFIG_DOCS)) if (!fields.some(([f]) => f === k)) problems.push('CONFIG_DOC_FOR_UNKNOWN_FIELD ' + k);
outFiles['references/generated/config-fields.md'] = HEAD +
  '# config.json 字段（生成）\n\n| 字段 | 默认 | 作用 |\n|---|---|---|\n' +
  fields.map(([k, v]) => `| \`${k}\` | \`${JSON.stringify(v)}\` | ${M.CONFIG_DOCS[k] || ''} |`).join('\n') + '\n';

// ---------- 错误码 ----------
const scripts = readdirSync(join(SKILL, 'scripts')).filter(f => f.endsWith('.mjs') && !['selftest.mjs', 'gen-docs.mjs', 'machine.mjs'].includes(f)).sort();
const codes = new Map();
for (const f of scripts) {
  const src = readFileSync(join(SKILL, 'scripts', f), 'utf8');
  for (const m of src.matchAll(/['`](?:[A-Z]+_)?([A-Z][A-Z0-9]*(?:_[A-Z0-9]+)+)\b/g)) {
    const c = m[0].slice(1);
    if (!codes.has(c)) codes.set(c, new Set());
    codes.get(c).add(f);
  }
}
const codeList = [...codes.keys()].sort();
outFiles['references/generated/reason-codes.md'] = HEAD +
  '# 错误码（生成：扫描脚本里实际会输出的码）\n\n脚本拒绝时 `reason` / `reasons` 里出现的码。含义看码名和紧跟的中文说明；判定逻辑以脚本为准。\n\n| 错误码 | 出现在 |\n|---|---|\n' +
  codeList.map(c => `| \`${c}\` | ${[...codes.get(c)].join('、')} |`).join('\n') + '\n';

// ---------- 失误台账（机器生成；真源 incidents.json） ----------
const incDoc = buildIncidentsDoc(SKILL);
for (const p of incDoc.problems) problems.push(p);
if (!incDoc.problems.length) outFiles[incDoc.path] = incDoc.content;

// ---------- 配置模板 ----------
outFiles['assets/config.template.json'] = JSON.stringify({ _note: '由 scripts/gen-docs.mjs 从默认配置生成，禁止手改。项目按需改这份副本后交用户本地审核', ...DEFAULT_CONFIG }, null, 2) + '\n';

// ---------- 手写文档与代码对账 ----------
const known = new Set([...Object.values(M.STATES).flatMap(v => v.allowed), ...M.READONLY, 'self-test']);
const prose = ['SKILL.md', ...readdirSync(join(SKILL, 'references')).filter(f => f.endsWith('.md')).map(f => 'references/' + f)];
const skipCodes = new Set(['SKILL_DIR', 'WORKFORCE_DIR', 'WORKFORCE_HEAD', 'MANIFEST_DRIFT', 'GENERATED_DOC_HAND_EDITED']);
for (const f of prose) {
  const t = readFileSync(join(SKILL, f), 'utf8');
  for (const m of t.matchAll(/loop\.mjs[`"]?\s+([a-z][a-z-]+)/g)) if (!known.has(m[1])) problems.push('DOC_UNKNOWN_COMMAND ' + f + ' loop.mjs ' + m[1]);
  for (const m of t.matchAll(/\b([A-Z][A-Z0-9]*(?:_[A-Z0-9]+){1,})\b/g)) {
    const c = m[1];
    if (skipCodes.has(c) || /^(SPEC|STATE)$/.test(c)) continue;
    if (Object.keys(M.STATES).includes(c)) continue;
    if (!codes.has(c) && !codeList.some(k => k.startsWith(c + '_') || k.endsWith('_' + c))) problems.push('DOC_UNKNOWN_CODE ' + f + ' ' + c);
  }
}

// ---------- 写出 / 比对 ----------
for (const [f, content] of Object.entries(outFiles)) {
  const p = join(SKILL, f);
  if (CHECK) {
    if (!existsSync(p)) problems.push('GENERATED_DOC_MISSING ' + f);
    else if (lf(readFileSync(p, 'utf8')) !== content) problems.push('GENERATED_DOC_HAND_EDITED ' + f + '（禁止手改；改源头后重跑 node scripts/gen-docs.mjs）');
  } else { mkdirSync(dirname(p), { recursive: true }); writeFileSync(p, content); }
}

// ---------- MANIFEST ----------
const walk = (d) => readdirSync(d).flatMap(x => { const p = join(d, x); return statSync(p).isDirectory() ? walk(p) : [p]; });
const files = walk(SKILL).map(rel).filter(f => f !== 'MANIFEST.json' && !f.startsWith('.')).sort();
const manifest = { _note: '由 scripts/gen-docs.mjs 生成，禁止手改。每个文件的 sha256（换行统一为 LF 后计算）', files: Object.fromEntries(files.map(f => [f, sha(lf(readFileSync(join(SKILL, f), 'utf8')))])) };
const mText = JSON.stringify(manifest, null, 2) + '\n';
if (CHECK) {
  const mp = join(SKILL, 'MANIFEST.json');
  if (!existsSync(mp)) problems.push('MANIFEST_MISSING');
  else {
    const old = JSON.parse(readFileSync(mp, 'utf8')).files || {};
    for (const f of new Set([...Object.keys(old), ...files])) if (old[f] !== manifest.files[f]) problems.push('MANIFEST_DRIFT ' + f + '（文件改了但没重新生成清单，或清单被手改）');
  }
} else writeFileSync(join(SKILL, 'MANIFEST.json'), mText);

console.log(JSON.stringify({ verdict: problems.length ? 'FAIL' : 'PASS', mode: CHECK ? 'check' : 'write', generated: Object.keys(outFiles), codes: codeList.length, manifest_files: files.length, problems }, null, 2));
process.exit(problems.length ? 1 : 0);
