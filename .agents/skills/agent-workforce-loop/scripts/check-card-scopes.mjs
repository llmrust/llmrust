#!/usr/bin/env node
/**
 * 卡面 scope 自检（INC-008 的守卫，固化自 `.workforce/tmp/arch_v5_selfcheck.js` 的三口径）：
 *   node scripts/check-card-scopes.mjs [--repo <repo>] [--dir <workforce 目录>]
 *
 * ① 死路径 CARD_SCOPE_DEAD_PATH：write_scopes（文件项）/ exact_file_paths 指向盘上不存在的路径
 *    ⇒ accept 时必然炸 OUT_OF_WRITE_SCOPE。
 * ② 未声明跨卡依赖 CARD_SCOPE_UNDECLARED_DEP：判据引用的证据路径落在别卡更具体的 scope 内，
 *    但本卡 blocked_by 未声明该卡 ⇒ 顺序依赖未登记。
 * ③ 幽灵路径 CARD_SCOPE_GHOST_PATH：判据引用的路径既不属于任何卡、盘上也不存在
 *    ⇒ 本卡要写它却不在 scope 内。
 *
 * 无 .workforce/cards 时 PASS + skipped（安装副本场景）。
 */
import { readFileSync, existsSync, readdirSync, statSync } from 'node:fs';
import { join, resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import * as L from './lib.mjs';

const SKILL = resolve(join(dirname(fileURLToPath(import.meta.url)), '..'));
const { flags } = L.parseArgs(process.argv.slice(2));
const repoDefault = resolve(SKILL, '..', '..', '..');
const P = L.paths(typeof flags.repo === 'string' ? resolve(flags.repo) : repoDefault, flags);
// 全局共享目录白名单：命中者不参与「跨卡归属」判定（谁都不能独占整棵共享树）。
// llmrust 适配：在原口径（evidence/ .workforce/ crates/ tools/ scripts/）之上补 Rust 仓常见目录
// src/ tests/ benches/ examples/；只做并集，判定规则本身不变。
const GLOBAL = new Set(['evidence/', '.workforce/', 'crates/', 'tools/', 'scripts/',
  'src/', 'tests/', 'benches/', 'examples/']);

const reasons = [];
if (!existsSync(P.cards)) {
  console.log(JSON.stringify({ verdict: 'PASS', skipped: true, reason: 'NO_WORKFORCE_CARDS ' + P.cards, cards: 0, reasons: [] }, null, 2));
  process.exit(0);
}
const cards = new Map();
for (const f of readdirSync(P.cards).filter(x => x.endsWith('.json')).sort()) {
  const c = L.readJSON(join(P.cards, f));
  if (!c.__error) cards.set(c.card_id || f.replace(/\.json$/, ''), c);
}
const exists = (p) => { try { statSync(p); return true; } catch { return false; } };
const inScopes = (f, scopes) => (scopes || []).some(s => f === s || f.startsWith(s.endsWith('/') ? s : s + '/'));
const ownerOf = (p, self) => {
  let best = null, bestLen = 0;
  for (const [id, c] of cards) {
    if (id === self) continue;
    for (const s of c.write_scopes || []) {
      if (GLOBAL.has(s)) continue;
      if (inScopes(p, [s]) && s.length > bestLen) { best = id; bestLen = s.length; }
    }
  }
  return best;
};
const DEP_RE = /(?:\.workforce\/)?evidence\/[A-Za-z0-9_.\-/]+/g;
let deadN = 0, undN = 0, ghostN = 0;
for (const [id, c] of cards) {
  for (const s of c.write_scopes || []) {
    if (!s.endsWith('/') && !exists(resolve(P.repo, s))) { deadN++; reasons.push('CARD_SCOPE_DEAD_PATH ' + id + ' -> ' + s); }
  }
  for (const f of c.exact_file_paths || []) {
    if (!exists(resolve(P.repo, f))) { deadN++; reasons.push('CARD_SCOPE_DEAD_PATH ' + id + ' -> ' + f); }
  }
  const text = (c.acceptance_criteria || []).join('\n');
  const cited = new Set((text.match(DEP_RE) || []).map(x => x.replace(/[.,;:)\]`]+$/, '')));
  for (const p of cited) {
    if (p.startsWith('.workforce/evidence/')) continue; // 批次级回执由 loop 管
    if (inScopes(p, c.write_scopes)) continue;         // 自有产物
    const owner = ownerOf(p, id);
    if (owner) {
      if (!(c.blocked_by || []).includes(owner)) { undN++; reasons.push('CARD_SCOPE_UNDECLARED_DEP ' + id + ' 引用 ' + p + '（owner=' + owner + '，blocked_by 未声明）'); }
    } else if (!exists(resolve(P.repo, p))) {
      ghostN++; reasons.push('CARD_SCOPE_GHOST_PATH ' + id + ' 引用 ' + p + '（无 owner 且盘上不存在）');
    }
  }
}
console.log(JSON.stringify({ verdict: reasons.length ? 'FAIL' : 'PASS', repo: P.repo, cards: cards.size, dead: deadN, undeclared: undN, ghost: ghostN, reasons }, null, 2));
process.exit(reasons.length ? 1 : 0);
