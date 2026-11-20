// qa-llmrust: validate the QA receipt with the repo's OWN gate logic (lib.mjs),
// so the receipt is checked by the same code the loop will run — not by my
// opinion of what the format should be.
import * as L from '../../../../.agents/skills/agent-workforce-loop/scripts/lib.mjs';

const P = L.paths('D:/llmrust-0.1.5', {});
const st = L.readState(P);
const batch = L.currentBatch(st, 'B-DELTA-NULL');
const t = batch.tasks.find(x => x.card_id === 'C-DELTA-NULL');

console.log('=== state ===');
console.log('  state       =', st.state);
console.log('  spec_sha256 =', st.spec_sha256);
console.log('  batch.id    =', batch.id);
console.log();

const h = (batch.handoffs || {})['C-DELTA-NULL'];
console.log('=== handoff ===');
console.log('  handoff_id =', h && h.handoff_id);
console.log();

console.log('=== requiredEvidence (is qa-C-DELTA-NULL.json required & where?) ===');
const req = L.requiredEvidence(P, batch).filter(r => r.card === 'C-DELTA-NULL');
for (const r of req) console.log('  ', r.file, 'role=' + r.role);
console.log('  evDir =', L.evDir(P, batch.id));
console.log();

console.log('=== checkEvidence (full batch) ===');
const ce = L.checkEvidence(P, st, batch);
console.log('  reasons:', ce.reasons.length ? ce.reasons : '(none)');
console.log();

console.log('=== handoffProblems for C-DELTA-NULL ===');
const hp = L.handoffProblems(P, batch, t);
console.log('  reasons:', hp.length ? hp : '(none)');
console.log();

console.log('=== fiveLayers ===');
const fl = L.fiveLayers(P, st, batch);
for (const l of fl.layers) {
  console.log('  ' + String(l.layer).padEnd(10), l.pass ? 'PASS' : 'FAIL', l.why || '');
}
console.log('  reasons:', fl.reasons.length ? fl.reasons : '(none)');
console.log();

// Focused QA-receipt-only checks, mirroring checkEvidence's QA branch.
const j = L.readJSON(L.evDir(P, batch.id) + '/qa-C-DELTA-NULL.json');
const problems = [];
for (const k of L.EVIDENCE_FIELDS) if (j[k] === undefined) problems.push('MISSING_FIELD ' + k);
if (j.role !== 'qa') problems.push('ROLE ' + j.role);
if (!/^qa-/.test(j.agent || '')) problems.push('AGENT_NOT_ROLE ' + j.agent);
if (j.declared_by_lead !== false || j.independent !== true) problems.push('NOT_INDEPENDENT');
if (!j.written_by || /^lead(-|$)/.test(j.written_by)) problems.push('WRITTEN_BY_LEAD ' + j.written_by);
if (j.batch_id !== batch.id) problems.push('BATCH ' + j.batch_id);
if (j.card_id !== 'C-DELTA-NULL') problems.push('CARD ' + j.card_id);
if (j.spec_sha256 !== st.spec_sha256) problems.push('STALE_SPEC');
if (!['PASS', 'FAIL'].includes(j.verdict)) problems.push('NO_VERDICT');
if (!Array.isArray(j.command_exit_codes) || !j.command_exit_codes.length) problems.push('NO_COMMAND_OUTPUT');
if (!Array.isArray(j.artifacts) || !j.artifacts.length) problems.push('NO_ARTIFACTS');
if (!L.agentMatchesSeat(j.agent, t.card.qa_agent)) problems.push('QA_AGENT_NOT_CARD_QA');
if (j.handoff_id !== h.handoff_id) problems.push('QA_NOT_FOR_CURRENT_HANDOFF');
if (!Array.isArray(j.files_changed)) problems.push('QA_NO_FILES_CHANGED');
for (const f of j.files_changed || []) if (!L.inScopes(f, t.card.write_scopes)) problems.push('OUT_OF_WRITE_SCOPE ' + f);

console.log('=== QA receipt field check ===');
console.log('  agent        =', JSON.stringify(j.agent));
console.log('  written_by   =', JSON.stringify(j.written_by));
console.log('  verdict      =', j.verdict);
console.log('  handoff_id   =', j.handoff_id, '(expect', h.handoff_id + ')');
console.log('  layers_covered =', JSON.stringify(j.layers_covered), '->', (j.layers_covered || []).map(L.layerId));
console.log('  mock_used    =', j.mock_used);
console.log('  artifacts    =', j.artifacts.length);
const badSha = j.artifacts.filter(a => !/^[0-9a-f]{64}$/.test(a.sha256 || ''));
console.log('  lowercase sha ok =', badSha.length === 0, badSha.map(a => a.path));
const exitsOk = j.command_exit_codes.every(c => c && typeof c.exit_code === 'number');
console.log('  exit codes numeric =', exitsOk);
console.log('  problems:', problems.length ? problems : '(none)');
console.log();
console.log('=== VERDICT ===');
console.log('  QA receipt valid =', problems.length === 0);
