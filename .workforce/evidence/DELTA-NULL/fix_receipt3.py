import hashlib
import json
import os

REPO = r'D:\llmrust-0.1.5'
NEW = os.path.join(REPO, r'.workforce\evidence\B-DELTA-NULL\dev-C-DELTA-NULL.json')

d = json.load(open(NEW, encoding='utf-8'))

d['agent'] = 'dev-llmrust'
d['written_by'] = 'dev-llmrust'
d['spec_sha256'] = 'f31889df63f6419e9d78d9c58121b1e093e31ccb1ac75af6a62cb767e9f0b33c'

# guard against CIRCULAR_RECEIPT: never pin mutable runtime/state files
BANNED = ('.workforce/signoffs/', '.workforce/loop-state.json',
          '.workforce/spec.json', '.workforce/cards/', '.workforce/config.json')
kept, dropped = [], []
for a in d.get('artifacts', []):
    p = a.get('path', '')
    if any(p.startswith(b) or p == b.rstrip('/') for b in BANNED):
        dropped.append(p)
    else:
        kept.append(a)
d['artifacts'] = kept
if dropped:
    print('DROPPED circular-risk artifacts:', dropped)

# recompute sha for the kept artifacts (lowercase, matching lib.mjs shaFile)
missing = []
for a in d['artifacts']:
    full = os.path.join(REPO, a['path'].replace('/', os.sep))
    if os.path.exists(full):
        a['sha256'] = hashlib.sha256(open(full, 'rb').read()).hexdigest()
    else:
        missing.append(a['path'])

with open(NEW, 'w', encoding='utf-8', newline='\n') as f:
    json.dump(d, f, ensure_ascii=False, indent=2)
    f.write('\n')

print('agent      =', d['agent'])
print('written_by =', d['written_by'])
print('spec_sha256=', d['spec_sha256'])
print('artifacts  =', len(d['artifacts']), 'missing =', missing)
print('files_changed =', d.get('files_changed'))
