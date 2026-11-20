// Verify the receipt against BOTH real gates using the CURRENT card, without
// needing a live batch (state was reset to SPEC_DRAFT by spec-revise).
// Builds a synthetic batch/task from the real card file + real state, then
// calls the real lib.mjs functions.
import * as L from '../../../.agents/skills/agent-workforce-loop/scripts/lib.mjs';
import { readFileSync } from 'node:fs';

const P = L.paths('D:/llmrust-0.1.5', {});
const s = L.readState(P);
const card = L.readJSON(P.cards + '/C-DELTA-NULL.json');
const receiptPath = L.evDir(P, 'B-DELTA-NULL') + '/dev-C-DELTA-NULL.json';
const j = L.readJSON(receiptPath);

console.log('=== current facts ===');
console.log('  state            =', s.state, '(reset by spec-revise)');
console.log('  state.spec_sha256=', s.spec_sha256);
console.log('  card.owner_agent =', JSON.stringify(card.owner_agent));
console.log('  card.qa_agent    =', JSON.stringify(card.qa_agent));
console.log();

console.log('=== receipt ===');
console.log('  agent       =', JSON.stringify(j.agent));
console.log('  written_by  =', JSON.stringify(j.written_by));
console.log('  spec_sha256 =', j.spec_sha256);
console.log('  batch_id    =', JSON.stringify(j.batch_id));
console.log('  card_id     =', JSON.stringify(j.card_id));
console.log('  role        =', JSON.stringify(j.role));
console.log('  verdict     =', JSON.stringify(j.verdict));
console.log('  independent =', j.independent);
console.log('  declared_by_lead =', j.declared_by_lead);
console.log('  files_changed    =', JSON.stringify(j.files_changed));
console.log();

const task = { task_id: 'T-DELTA-NULL', role: 'dev', card_id: 'C-DELTA-NULL', card };
const batch = { id: 'B-DELTA-NULL', tasks: [task], handoffs: {} };

console.log('=== GATE 1: workerReceiptProblems (the one handoff uses) ===');
const probs = L.workerReceiptProblems(P, s, batch, task);
console.log('  ' + (probs.length ? JSON.stringify(probs, null, 2) : 'NO PROBLEMS -> handoff would PASS'));
console.log();

console.log('=== GATE 2: checkEvidence (the one check-roles uses) ===');
const ev = L.checkEvidence(P, s, batch);
const mine = (ev.reasons || []).filter(r => r.includes('dev-C-DELTA-NULL'));
console.log('  all reasons     =', JSON.stringify(ev.reasons, null, 2));
console.log('  reasons about MY receipt =', mine.length ? JSON.stringify(mine) : 'NONE -> my receipt is clean');
console.log();

console.log('=== EVIDENCE_FIELDS completeness ===');
const miss = L.EVIDENCE_FIELDS.filter(k => j[k] === undefined);
console.log('  missing fields =', miss.length ? miss.join(',') : 'none');
console.log();

const gate1 = probs.length === 0;
const gate2 = mine.length === 0;
console.log('GATE1(handoff)  :', gate1 ? 'PASS' : 'FAIL');
console.log('GATE2(check-roles, my row) :', gate2 ? 'PASS' : 'FAIL');
console.log('BOTH            :', (gate1 && gate2) ? 'PASS' : 'FAIL');
