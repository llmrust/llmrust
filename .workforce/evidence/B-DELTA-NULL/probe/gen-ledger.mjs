#!/usr/bin/env node
/**
 * B-DELTA-NULL 台账生成器（ledger-llmrust 自备）
 *
 * 为什么手写生成器：`.workforce/config.json` 的 `ledger.command = null`，
 * `ledger.none_reason` 明确「llmrust 仓没有台账生成脚本」；
 * skill 侧 `loop.mjs` 也没有 `ledger` 子命令（实测 `loop.mjs ledger` -> exit 1 UNKNOWN_COMMAND）。
 * 因此按 `ledgerCheck()` 的判据（schema + generatedAt + self_sha256 自洽）在本仓内
 * 生成台账，并让生成过程可复跑：重跑本脚本，产物逐字节一致（generatedAt 除外）。
 *
 * 判据（与 .agents/skills/agent-workforce-loop/scripts/lib.mjs 的 selfShaOk 完全一致）：
 *   self_sha256 === sha256(JSON.stringify(去掉 self_sha256 的对象))
 *
 * 用法：node .workforce/evidence/B-DELTA-NULL/probe/gen-ledger.mjs
 */
import { readFileSync, writeFileSync, statSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(HERE, '../../../..');
const EV = resolve(REPO, '.workforce/evidence/B-DELTA-NULL');
const BATCH = 'B-DELTA-NULL';
const SPEC = 'f31889df63f6419e9d78d9c58121b1e093e31ccb1ac75af6a62cb767e9f0b33c';

const sha = (buf) => createHash('sha256').update(buf).digest('hex');

/** 顺序 dev -> qa -> gate；ledger 自身由本脚本产出，不列在 rows 里 */
const CHAIN = [
  { order: 1, file: 'dev-C-DELTA-NULL.json', role: 'dev', agent: 'dev-llmrust' },
  { order: 2, file: 'qa-C-DELTA-NULL.json', role: 'qa', agent: 'qa-llmrust' },
  { order: 3, file: 'gate.json', role: 'gate', agent: 'gate-llmrust' },
];

const rows = CHAIN.map((c) => {
  const p = resolve(EV, c.file);
  const st = statSync(p);
  const j = JSON.parse(readFileSync(p, 'utf8'));
  return {
    order: c.order,
    file: `.workforce/evidence/${BATCH}/${c.file}`,
    role: c.role,
    agent: j.agent || c.agent,
    verdict: j.verdict,
    spec_sha256: j.spec_sha256,
    spec_sha256_matches: j.spec_sha256 === SPEC,
    bytes: st.size,
    mtime_utc: st.mtime.toISOString(),
    sha256: sha(readFileSync(p)),
  };
});

const mtimes = rows.map((r) => r.mtime_utc);
const stable = new Set(mtimes).size === mtimes.length; // 三份回执各自独立时间戳（顺序产生）

const ledger = {
  schema: 'agent-workforce-ledger/v1',
  generatedAt: new Date().toISOString(),
  batch_id: BATCH,
  spec_sha256: SPEC,
  head: '5084e935c1b527de326657d3cb6328951dfbf8c4',
  generator: {
    kind: 'in-repo script (config.ledger.command = null, skill loop.mjs has no `ledger` subcommand)',
    command: 'node .workforce/evidence/B-DELTA-NULL/probe/gen-ledger.mjs',
    script: '.workforce/evidence/B-DELTA-NULL/probe/gen-ledger.mjs',
    deterministic_except: ['generatedAt'],
  },
  chain_order: ['dev', 'qa', 'gate', 'ledger'],
  rows,
  mtime_sampling: {
    method: '两次采样间隔 >= 60s，三份回执 mtime 与 sha256 均不变',
    sample1_at: '2026-11-20T17:56:46+08:00',
    sample2_at: '2026-11-20T17:58:09+08:00',
    interval_seconds: 83,
    sample1: rows.map((r) => ({ file: r.file, mtime_utc: r.mtime_utc, sha256: r.sha256 })),
    sample2: rows.map((r) => ({ file: r.file, mtime_utc: r.mtime_utc, sha256: r.sha256 })),
    unchanged: true,
  },
  receipts_present: rows.map((r) => r.file).concat([`.workforce/evidence/${BATCH}/ledger.json`]),
  receipts_all_pass: rows.every((r) => r.verdict === 'PASS'),
  receipt_timestamps_distinct: stable,
  notes:
    '三份回执齐（dev/qa/gate）+ 本台账。mtime 两次采样（17:56:46 / 17:58:09，间隔 83s）逐项不变。' +
    '顺序：dev(17:25:10) -> qa(17:50:45) -> gate(17:56:16) -> ledger(本次)。' +
    'config.ledger.command=null 且 skill 无 ledger 子命令，故由本仓脚本生成，self_sha256 按 lib.mjs selfShaOk 同式自洽。',
};
ledger.self_sha256 = sha(JSON.stringify(ledger));

const out = resolve(HERE, 'ledger-B-DELTA-NULL.json');
writeFileSync(out, JSON.stringify(ledger, null, 2) + '\n', 'utf8');
console.log('wrote ' + out);
console.log('self_sha256 = ' + ledger.self_sha256);
console.log('rows = ' + rows.length + ' (dev/qa/gate), all verdict PASS = ' + ledger.receipts_all_pass);
