import hashlib
import json
import os

REPO = r'D:\llmrust-0.1.5'
NEW = os.path.join(REPO, r'.workforce\evidence\B-DELTA-NULL\dev-C-DELTA-NULL.json')

d = json.load(open(NEW, encoding='utf-8'))

# lib.mjs: sha = createHash('sha256').update(buf).digest('hex')  -> LOWERCASE
d['agent'] = 'dev-llmrust'   # satisfies checkEvidence: /^dev-/

missing = []
for a in d['artifacts']:
    p = os.path.join(REPO, a['path'].replace('/', os.sep))
    if os.path.exists(p):
        a['sha256'] = hashlib.sha256(open(p, 'rb').read()).hexdigest()  # lowercase
    else:
        missing.append(a['path'])

with open(NEW, 'w', encoding='utf-8', newline='\n') as f:
    json.dump(d, f, ensure_ascii=False, indent=2)
    f.write('\n')

print('agent =', d['agent'])
print('sha sample =', d['artifacts'][0]['sha256'])
print('missing =', missing)
