// Confirm release's dirty check now sees a clean tree (ci.json + state exempted).
import * as L from '../../../../.agents/skills/agent-workforce-loop/scripts/lib.mjs';

const P = L.paths('D:/llmrust-0.1.5', {});
const s = L.readState(P);
const b = L.currentBatch(s, 'B-DELTA-NULL') || (s.batches || []).find(x => x.id === 'B-DELTA-NULL');

console.log('HEAD          =', L.gitHead(P.repo));
console.log('state         =', s.state);
console.log('batch         =', b ? b.id + ' status=' + b.status : '(none)');
console.log();

const dirty = L.releaseDirtyFiles(P, s, b);
console.log('releaseDirtyFiles ->', dirty === null ? 'GIT_UNAVAILABLE' : JSON.stringify(dirty, null, 2));
console.log('WORKTREE_DIRTY would fire?', (dirty === null || dirty.length > 0) ? 'YES (blocked)' : 'NO (clean)');
console.log();

// Also report what release would still block on.
const ciRec = L.ciReceiptCheck(P, b);
console.log('ciReceiptCheck ->', ciRec.ok ? 'ok' : ciRec.reason);
const ci = L.ciStatus(P);
console.log('ciStatus       ->', ci.ok ? 'ok head=' + ci.head : ci.reason);
