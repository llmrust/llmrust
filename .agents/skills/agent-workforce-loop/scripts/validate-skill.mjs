#!/usr/bin/env node
/** skill 结构自检（Agent Skills 规范）：frontmatter（容忍 CRLF/BOM）、文件齐全、脚本语法、模板 JSON、SKILL.md 引用的脚本存在、命令引号闭合。 */
import { readFileSync, existsSync } from 'node:fs';
import { join, dirname, resolve, basename } from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
const SKILL = resolve(join(dirname(fileURLToPath(import.meta.url)), '..'));
const reasons = [];
const sk = readFileSync(join(SKILL, 'SKILL.md'), 'utf8').replace(/^\uFEFF/, '').replace(/\r\n/g, '\n');
const fm = /^---\n([\s\S]*?)\n---/.exec(sk);
if (!fm) reasons.push('FRONTMATTER_MISSING');
else {
  const nm = /^name:\s*([a-z0-9-]+)\s*$/m.exec(fm[1]);
  if (!nm) reasons.push('NAME_MISSING');
  else if (nm[1] !== 'agent-workforce-loop') reasons.push('NAME_WRONG ' + nm[1]);
  // Agent Skills 规范：目录名须与 name 一致（解压到别的目录名会导致部分 harness 不加载）
  else if (basename(SKILL) !== nm[1]) reasons.push('DIR_NAME_MISMATCH 目录名 ' + basename(SKILL) + ' ≠ name ' + nm[1]);
  const d = /^description:\s*(.+)$/m.exec(fm[1]);
  if (!d) reasons.push('DESCRIPTION_MISSING'); else if (d[1].length > 1024) reasons.push('DESCRIPTION_TOO_LONG ' + d[1].length);
}
const scripts = ['lib.mjs', 'machine.mjs', 'approve.mjs', 'gen-docs.mjs', 'loop.mjs', 'validate-skill.mjs', 'validate-spec.mjs', 'check-roles.mjs', 'accept-five-layers.mjs', 'selftest.mjs', 'gen-incidents.mjs', 'check-incidents.mjs', 'check-criteria.mjs', 'check-card-scopes.mjs', 'merge-pr.mjs'];
const need = [...scripts.map(f => 'scripts/' + f),
  'references/roles.md', 'references/loop-state-machine.md', 'references/spec-structure.md',
  'references/acceptance-five-layers.md', 'references/flow.md', 'references/file-layout.md',
  'references/config.md', 'references/harness-adapters.md', 'references/generated/state-machine.md', 'references/generated/config-fields.md', 'references/generated/reason-codes.md', 'references/generated/incidents.md', 'incidents.json', 'MANIFEST.json', 'assets/config.template.json',
  'assets/spec.template.json', 'assets/card.template.json', 'assets/evidence.template.json',
  'assets/issue.template.json', 'assets/signoff.template.json', 'assets/acceptors.template.json'];
for (const f of need) if (!existsSync(join(SKILL, f))) reasons.push('MISSING ' + f);
for (const f of scripts) {
  const p = join(SKILL, 'scripts', f);
  if (existsSync(p)) try { execFileSync(process.execPath, ['--check', p], { stdio: 'ignore' }); } catch { reasons.push('SYNTAX ' + f); }
}
for (const t of need.filter(f => f.endsWith('.json'))) {
  const p = join(SKILL, t);
  if (existsSync(p)) try { JSON.parse(readFileSync(p, 'utf8')); } catch { reasons.push('INVALID_JSON ' + t); }
}
for (const m of sk.matchAll(/\$SKILL\/scripts\/([a-z0-9-]+\.mjs)/g)) if (!existsSync(join(SKILL, 'scripts', m[1]))) reasons.push('DANGLING_REF ' + m[1]);
// 代码块里的命令行：双引号必须成对
const blocks = [...sk.matchAll(/```(?:bash|sh)?\n([\s\S]*?)```/g)].map(m => m[1]).join('\n');
blocks.split('\n').forEach((l, i) => { if (((l.match(/"/g) || []).length) % 2) reasons.push('UNBALANCED_QUOTE: ' + l.trim()); });
// 机器生成部分：被手改、清单过期、文档提到不存在的命令或错误码 → FAIL
try { execFileSync(process.execPath, [join(SKILL, 'scripts', 'gen-docs.mjs'), '--check'], { stdio: 'pipe' }); }
catch (e) { try { reasons.push(...JSON.parse(e.stdout.toString()).problems); } catch { reasons.push('GEN_DOCS_CHECK_FAILED'); } }
// 失误台账（结构/守卫/期限/文档漂移）+ 判据可判性（INC-006 新守卫）：子脚本自己处理「安装副本无 .workforce」的 skip
for (const s of ['check-incidents.mjs', 'check-criteria.mjs']) {
  try { execFileSync(process.execPath, [join(SKILL, 'scripts', s)], { stdio: 'pipe' }); }
  catch (e) {
    try { const j = JSON.parse(e.stdout.toString()); reasons.push(...(j.reasons || [s + ' FAILED'])); }
    catch { reasons.push(s + ' FAILED'); }
  }
}
const verdict = reasons.length ? 'FAIL' : 'PASS';
console.log(JSON.stringify({ verdict, skill_dir: SKILL, checked_files: need.length + 1, reasons }, null, 2));
process.exit(reasons.length ? 1 : 0);
