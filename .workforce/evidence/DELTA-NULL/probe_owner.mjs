// Read-only proof: with the card AS-IS, no `agent` value can pass BOTH gates.
// Shows that changing card.owner_agent to "dev-llmrust" (the skill's own
// convention — selftest.mjs uses `owner_agent: 'dev-' + id`) makes both pass.
import * as L from '../../../.agents/skills/agent-workforce-loop/scripts/lib.mjs';

const P = L.paths('D:/llmrust-0.1.5', {});
const s = L.readState(P);
const b = L.currentBatch(s, 'B-DELTA-NULL');
const t = b.tasks.find(x => x.card_id === 'C-DELTA-NULL');

console.log('=== card as-is ===');
console.log('  card.owner_agent =', JSON.stringify(t.card.owner_agent));
console.log();

const agents = ['dev-llmrust', 'llmrust-dev', 'dev-seat', 'dev'];
console.log('gate A = checkEvidence  : /^dev-/.test(agent)');
console.log('gate B = workerReceipt  : agentMatchesSeat(agent, card.owner_agent)');
console.log();

for (const owner of ['llmrust-dev', 'dev-llmrust', 'dev']) {
  console.log('--- card.owner_agent =', JSON.stringify(owner), '---');
  let any = false;
  for (const a of agents) {
    const A = new RegExp('^dev-').test(a);
    const B = L.agentMatchesSeat(a, owner);
    const both = A && B;
    if (both) any = true;
    console.log('  agent=' + a.padEnd(13) +
                ' A=' + String(A).padEnd(6) +
                ' B=' + String(B).padEnd(6) +
                ' ' + (both ? '<== PASSES BOTH' : ''));
  }
  console.log('  => any agent passes both? ' + any);
  console.log();
}

// Now actually run the real functions against the receipt file with a
// hypothetical card whose owner_agent is "dev-llmrust".
console.log('=== real workerReceiptProblems() with hypothetical owner_agent ===');
for (const owner of ['llmrust-dev', 'dev-llmrust']) {
  const card = { ...t.card, owner_agent: owner };
  const task = { ...t, card };
  const probs = L.workerReceiptProblems(P, s, b, task);
  console.log('  owner_agent=' + owner.padEnd(14) + ' -> ' + (probs.length ? JSON.stringify(probs) : 'NO PROBLEMS'));
}
