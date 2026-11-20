const fs = require('fs');
const s = fs.readFileSync('.agents/skills/agent-workforce-loop/scripts/lib.mjs', 'utf8');
for (const name of ['pushedRefs', 'ciStatus']) {
  const i = s.indexOf('function ' + name);
  if (i < 0) { console.log('=== ' + name + ' NOT FOUND ==='); continue; }
  console.log('=== ' + name + ' ===');
  console.log(s.slice(i, i + 2200));
  console.log();
}
