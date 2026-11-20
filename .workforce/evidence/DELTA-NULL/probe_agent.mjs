// Empirically prove the two evidence gates and the path rule from lib.mjs.
//   1) evDir(P, batchId)  -> required dir is evidence/<batch.id>/
//   2) checkEvidence      -> new RegExp('^' + role + '-').test(j.agent)
//   3) workerReceiptProblems -> agentMatchesSeat(j.agent, card.owner_agent)
import * as L from '../../../.agents/skills/agent-workforce-loop/scripts/lib.mjs';

const P = L.paths('D:/llmrust-0.1.5', {});
const s = L.readState(P);
const b = L.currentBatch(s, 'B-DELTA-NULL');

console.log('=== state ===');
console.log('  state           =', s.state);
console.log('  batch.id        =', JSON.stringify(b.id));
console.log('  spec_sha256     =', s.spec_sha256);
console.log();

console.log('=== PATH RULE (lib.mjs:461 evDir) ===');
console.log('  evDir(P, batch.id) =', L.evDir(P, b.id));
console.log('  required file      =', L.evDir(P, b.id) + '\\dev-C-DELTA-NULL.json');
console.log('  NOTE: batch.id is "B-DELTA-NULL", so the evidence directory must be');
console.log('        .workforce/evidence/B-DELTA-NULL/  — NOT evidence/DELTA-NULL/');
console.log();

const t = b.tasks.find(x => x.card_id === 'C-DELTA-NULL');
console.log('=== CARD SEAT ===');
console.log('  task.role       =', JSON.stringify(t.role));
console.log('  card.owner_agent=', JSON.stringify(t.card.owner_agent));
console.log('  card.qa_agent   =', JSON.stringify(t.card.qa_agent));
console.log('  card.write_scopes =', JSON.stringify(t.card.write_scopes));
console.log();

console.log('=== AGENT FIELD: the two gates are mutually exclusive with owner_agent="llmrust-dev" ===');
const candidates = ['dev-llmrust', 'llmrust-dev', 'dev', 'dev-seat'];
for (const a of candidates) {
  const gateA = new RegExp('^' + t.role + '-').test(a);          // checkEvidence
  const gateB = L.agentMatchesSeat(a, t.card.owner_agent);       // workerReceiptProblems
  console.log('  agent=' + a.padEnd(14) +
              ' checkEvidence(^dev-)= ' + String(gateA).padEnd(6) +
              ' workerReceipt(owner)= ' + String(gateB).padEnd(6) +
              ' => ' + (gateA && gateB ? 'PASS BOTH' : 'CANNOT PASS BOTH'));
}
console.log();

console.log('=== Would it pass if the card said owner_agent="dev" (role name, no hyphen)? ===');
for (const a of candidates) {
  const gateA = new RegExp('^' + t.role + '-').test(a);
  const gateB = L.agentMatchesSeat(a, 'dev');
  if (gateA && gateB) console.log('  PASS BOTH: agent=' + a + ' owner_agent=dev');
}
