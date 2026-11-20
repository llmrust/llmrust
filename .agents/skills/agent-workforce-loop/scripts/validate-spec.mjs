#!/usr/bin/env node
/**
 * 架构师 spec 校验：六层结构 + 任务卡文件 + 审批链进度（签署文件）。
 * 结构 FAIL → exit 1；结构 PASS 时 exit 0，并报告审批链走到哪一步（审批本身由 loop.mjs 迁移时强制）。
 */
import { existsSync } from 'node:fs';
import { join, resolve } from 'node:path';
import * as L from './lib.mjs';
const { flags } = L.parseArgs(process.argv.slice(2));
const repo = resolve(typeof flags.repo === 'string' ? flags.repo : process.cwd());
const P = L.paths(repo, flags);
const specPath = resolve(typeof flags.spec === 'string' ? flags.spec : P.spec);
if (!existsSync(specPath)) L.out({ verdict: 'FAIL', reason: 'SPEC_MISSING ' + specPath }, 1);
const sp = L.readJSON(specPath);
const v = L.validateSpecStructure(sp, P);
const cardErr = [];
for (const p of sp.phases || []) for (const b of p.batches || []) for (const t of b.tasks || []) {
  if (!t.card) continue;
  const c = L.loadCard(P, t.card);
  if (c.error) cardErr.push(c.error);
  else if (c.card.phase !== p.id) cardErr.push('CARD_PHASE_MISMATCH ' + c.id);
}
const specSha = L.shaFile(specPath);
const w = L.checkWorkerSignoffs(P, specSha);
const lead = L.readSignoff(P, 'lead');
const user = L.readSignoff(P, 'user');
const reasons = [...v.reasons, ...cardErr];
L.out({ verdict: reasons.length ? 'FAIL' : 'PASS', spec: specPath, spec_sha256: specSha, produced_by: sp.produced_by,
  layers: v.layers, reasons,
  approval_chain: {
    workers: w.reasons.length ? 'PENDING: ' + w.reasons.join('; ') : 'ALL_AGREED',
    lead: lead.present && lead.approved === true && lead.spec_sha256 === specSha ? 'APPROVED' : 'PENDING',
    user: user.present && user.confirmed === true && user.spec_sha256 === specSha ? (L.userCommitted(P, user.path).ok ? 'CONFIRMED' : 'NOT_COMMITTED_BY_ACCEPTOR') : 'PENDING',
  } }, reasons.length ? 1 : 0);
