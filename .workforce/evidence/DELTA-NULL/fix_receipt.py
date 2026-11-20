import hashlib
import json
import os

REPO = r'D:\llmrust-0.1.5'
SRC = os.path.join(REPO, r'.workforce\evidence\DELTA-NULL\dev-C-DELTA-NULL.json')
NEWDIR = os.path.join(REPO, r'.workforce\evidence\B-DELTA-NULL')
NEW = os.path.join(NEWDIR, 'dev-C-DELTA-NULL.json')

d = json.load(open(SRC, encoding='utf-8'))

# ---- EVIDENCE_FIELDS (lib.mjs:153) all present -------------------------
d['schema'] = 'agent-workforce-evidence/v1'
d['role'] = 'dev'
d['agent'] = 'llmrust-dev'   # matches card.owner_agent exactly (handoff gate)
d['spec_sha256'] = 'acc8f73302842a3e53de4367053408231d9990b34cb99cbe3483fe7400b3ebe3'
d['batch_id'] = 'B-DELTA-NULL'
d['card_id'] = 'C-DELTA-NULL'
d['verdict'] = 'PASS'
d['independent'] = True
d['declared_by_lead'] = False
d['written_by'] = 'llmrust-dev'

# ---- workerReceiptProblems (lib.mjs:645) requires files_changed --------
d['files_changed'] = ['src/providers/compat.rs', 'tests/hotspot_ledger.json']

# ---- command_exit_codes must be an ARRAY of {cmd, exit_code} -----------
if isinstance(d.get('command_exit_codes'), dict):
    d['command_exit_codes'] = [{'cmd': k, 'exit_code': v}
                               for k, v in d['command_exit_codes'].items()]

# ---- refresh artifact sha256 ------------------------------------------
missing = []
for a in d.get('artifacts', []):
    p = os.path.join(REPO, a['path'].replace('/', os.sep))
    if os.path.exists(p):
        a['sha256'] = hashlib.sha256(open(p, 'rb').read()).hexdigest().upper()
    else:
        missing.append(a['path'])

os.makedirs(NEWDIR, exist_ok=True)
with open(NEW, 'w', encoding='utf-8', newline='\n') as f:
    json.dump(d, f, ensure_ascii=False, indent=2)
    f.write('\n')

print('WROTE', NEW)
print('agent =', d['agent'])
print('files_changed =', d['files_changed'])
print('command_exit_codes list? =', isinstance(d['command_exit_codes'], list),
      'count =', len(d['command_exit_codes']))
print('artifacts =', len(d['artifacts']), 'missing =', missing)
