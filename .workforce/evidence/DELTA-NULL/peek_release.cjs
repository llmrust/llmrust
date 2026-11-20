const fs = require('fs');
const s = fs.readFileSync('.agents/skills/agent-workforce-loop/scripts/loop.mjs', 'utf8');
const i = s.indexOf("cmd === 'release'");
console.log('--- release handler ---');
console.log(s.slice(i, i + 2600));
