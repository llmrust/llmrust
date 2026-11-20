const fs = require('fs');
const s = fs.readFileSync('.agents/skills/agent-workforce-loop/scripts/lib.mjs', 'utf8');
for (const name of ['generatedCheck', 'inScopes']) {
  const i = s.indexOf('function ' + name);
  if (i < 0) { console.log('=== ' + name + ' NOT FOUND ==='); continue; }
  console.log('=== ' + name + ' ===');
  console.log(s.slice(i, i + 1200));
  console.log();
}
