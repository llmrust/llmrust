const fs = require('fs');
const s = fs.readFileSync('.agents/skills/agent-workforce-loop/scripts/lib.mjs', 'utf8');
for (const name of ['releaseDirtyFiles', 'statusPath', 'ciReceiptCheck']) {
  const i = s.indexOf('function ' + name);
  const j = s.indexOf('export function ' + name);
  const k = i >= 0 ? i : j;
  if (k < 0) { console.log('=== ' + name + ' NOT FOUND ==='); continue; }
  console.log('=== ' + name + ' ===');
  console.log(s.slice(k, k + 1500));
  console.log();
}
