#!/usr/bin/env node
/**
 * 判据可判性校验器（INC-006 的新守卫）。由 validate-skill.mjs 调用，也可单独跑：
 *   node scripts/check-criteria.mjs [--repo <repo>] [--dir <workforce 目录>]
 *
 * 扫 <workforce>/cards/*.json 的 acceptance_criteria，三条判据（命中即红）：
 *   1. CRITERIA_SAMPLING_AMBIGUOUS：出现 P95/P99/min-of-N 等择优样式，却没有取样口径
 *      （必须写明 N≥k / 跑 N 次 / ≥k 轮）也没有「中位数 / median」⇒ 可被「取最好一次」洗绿。
 *   2. CRITERIA_ABSOLUTE_COUNT_UNVERIFIABLE：出现「恰好 N 个 / 全部 success / 必须 N 个 job 全 success」
 *      等绝对数量断言，却没有「设计性/结构性跳过」说明 ⇒ 与设计性 skipped 冲突、结构上不可判。
 *   3. CRITERIA_NO_BASELINE_PIN：出现「不劣于基线」却没有基线文件路径或 sha ⇒ 基线不可复核。
 *
 * 无 .workforce/cards 时 PASS + skipped（安装副本/纯 skill 包场景）；卡片存在但判据全合规 ⇒ PASS。
 */
import { readFileSync, existsSync, readdirSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { dirname } from 'node:path';
import * as L from './lib.mjs';

const SKILL = resolve(join(dirname(fileURLToPath(import.meta.url)), '..'));
const { flags } = L.parseArgs(process.argv.slice(2));
const repoDefault = resolve(SKILL, '..', '..', '..');
const P = L.paths(typeof flags.repo === 'string' ? resolve(flags.repo) : repoDefault, flags);

const SAMPLING = /P95|P99|min-of-(?:N|n|\d+)/i;
const SAMPLE_SPEC = /(?:N|n)\s*(?:≥|>=|=)\s*\d+|(?:≥|>=)\s*\d+\s*(?:\/\s*N|轮|次)|跑\s*\d+\s*次|取\s*\d+\s*次|min-of-(?:3|5|10)\b/i;
const MEDIAN = /中位数|median/i;
const ABSOLUTE = /恰好\s*\d+\s*个|全部\s*`?success|全\s*success|必须\s*\d+\s*个\s*(?:check|job)[^。；\n]{0,20}(?:success|pass)|100%\s*(?:全绿|pass|success)/i;
const SKIP_EXPL = /结构性跳过|设计性跳过|按设计跳过|docs-only|skipping|跳过项/i;
const BASELINE = /不劣于基线|不低于基线|不得劣于基线/;
const BASELINE_PIN = /(?:\.workforce|evidence)\/[\w./-]+|sha256\s*[=:]\s*[0-9a-f]{8,}|[0-9a-f]{40,64}/i;
// INC-013：判据点名「改调用方/入口/调用点/由 X 持有/返回 RAII/在 X 里加」时，write_scopes 必须覆盖判据里点名的 .rs。
const CALLSITE = /改调用方|调用方|调用点|入口|由[^，。；\n]{0,24}持有|返回\s*RAII|RAII/i;
// .rs 落点前缀：原先硬编码 crates/tools/scripts，只对 beeli 的 crates 布局生效。
// 适配：改为「静态基线 ∪ config.business_dirs ∪ Rust 仓常见源码目录」，静态基线原样保留（只做并集，不删任何前缀），
// 因此既有判据口径不变，只是新增识别 llmrust 的 src/ tests/ benches/ examples/。
const RS_STATIC = ['crates', 'tools', 'scripts', '.workforce'];
const RS_RUST_DIRS = ['src', 'tests', 'benches', 'examples'];
const RS_PREFIX = [...new Set([...RS_STATIC, ...RS_RUST_DIRS,
  ...((L.cfg(P).business_dirs || []).map(d => String(d).replace(/\/+$/, '')).filter(Boolean))])];
const RS_PATH = new RegExp('\\b(?:' + RS_PREFIX.map(x => x.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('|') + ')/[A-Za-z0-9_./-]+\\.rs\\b', 'g');
// INC-014：相对比值类判据（×k / 不劣于 / ≤基线×k）必须明示**聚合口径**
// （min-of-5 / median-of-5 / all-rounds / single-run），否则 min 与 median 两种聚合结论可相反、判据不可唯一机判。
const REL_RATIO = /(?:≤|<=|<)\s*\d+(?:\.\d+)?\s*[×xX倍]|不劣于|优于基线|相对基线/;
const AGG_TOKEN = /min-of-(?:\d+|N)|median-of-(?:\d+|N)|median\b|max-of-(?:\d+|N)|all-rounds|single-run|均值口径|聚合口径|中位数/i;

const reasons = [];
let cards = 0; let criteria = 0;
if (!existsSync(P.cards)) {
  console.log(JSON.stringify({ verdict: 'PASS', skipped: true, reason: 'NO_WORKFORCE_CARDS ' + P.cards, cards: 0, criteria: 0, reasons: [] }, null, 2));
  process.exit(0);
}
for (const f of readdirSync(P.cards).filter(x => x.endsWith('.json')).sort()) {
  let c;
  try { c = L.readJSON(join(P.cards, f)); } catch { reasons.push('CARD_INVALID_JSON ' + f); continue; }
  if (c.__error) { reasons.push('CARD_INVALID_JSON ' + f); continue; }
  cards++;
  const list = Array.isArray(c.acceptance_criteria) ? c.acceptance_criteria : [];
  const cardId = c.card_id || f.replace(/\.json$/, '');
  // 抽样口径是「卡级」声明：允许把 min-of-N + N≥k 写在本卡另一条判据里（跨条互指），
  // 只要整卡有取样口径/中位数即可；否则整卡只能靠「取最好一次」洗绿 ⇒ 红一次。
  const allText = list.map(x => String(x || '')).join('\n');
  if (SAMPLING.test(allText) && !(SAMPLE_SPEC.test(allText) || MEDIAN.test(allText))) {
    const idx = list.findIndex(x => SAMPLING.test(String(x || '')));
    reasons.push('CRITERIA_SAMPLING_AMBIGUOUS ' + cardId + ' criteria[' + Math.max(0, idx) + ']');
  }
  // INC-013：卡面 scope 漏了判据隐含的必需落点（调用方/入口/RAII 持有方）⇒ CRITERIA_SCOPE_GAP
  if (CALLSITE.test(allText)) {
    const named = [...new Set(allText.match(RS_PATH) || [])];
    const inScope = (f) => (c.write_scopes || []).some(x => f === x || f.startsWith(x.endsWith('/') ? x : x + '/'));
    const missing = named.filter(f => !inScope(f));
    if (missing.length) reasons.push('CRITERIA_SCOPE_GAP ' + cardId + ' 判据点名调用点/入口但 write_scopes 未覆盖：' + missing.join(','));
  }
  // INC-014：相对比值类判据必须明示聚合口径（卡级 aggregation 字段或判据里写 min-of-5/median-of-5/…）
  if (REL_RATIO.test(allText)) {
    const declared = (typeof c.aggregation === 'string' && AGG_TOKEN.test(c.aggregation)) || AGG_TOKEN.test(allText);
    if (!declared) reasons.push('CRITERIA_AGGREGATION_AMBIGUOUS ' + cardId + ' 相对比值类判据未明示聚合口径（min-of-5 / median-of-5 / all-rounds / single-run）');
  }
  list.forEach((t0, i) => {
    const t = String(t0 || '');
    criteria++;
    const where = cardId + ' criteria[' + i + ']';
    if (ABSOLUTE.test(t) && !SKIP_EXPL.test(t)) reasons.push('CRITERIA_ABSOLUTE_COUNT_UNVERIFIABLE ' + where);
    if (BASELINE.test(t) && !BASELINE_PIN.test(t)) reasons.push('CRITERIA_NO_BASELINE_PIN ' + where);
  });
}
console.log(JSON.stringify({ verdict: reasons.length ? 'FAIL' : 'PASS', repo: P.repo, cards, criteria, reasons }, null, 2));
process.exit(reasons.length ? 1 : 0);
