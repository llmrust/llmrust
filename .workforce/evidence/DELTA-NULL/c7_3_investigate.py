import os
import re
import subprocess
import sys

REPO = r'D:\llmrust-0.1.5'
BASE = '33a8e3e^'
FENCE = '`' * 3

IGNORE_RE = re.compile(r'^\s*(?:///|//!)\s*' + FENCE + r'\s*ignore')


def git_show(path, rev):
    r = subprocess.run(['git', '-C', REPO, 'show', f'{rev}:{path}'], capture_output=True)
    return r.stdout.decode('utf-8') if r.returncode == 0 else None


def cur(path):
    full = os.path.join(REPO, path.replace('/', os.sep))
    try:
        return open(full, encoding='utf-8').read()
    except FileNotFoundError:
        return None


def ignore_blocks(text):
    """Locate ```ignore fenced doc blocks: return list of (line_no, stripped_line)."""
    out = []
    if text is None:
        return out
    for i, ln in enumerate(text.splitlines(), 1):
        if IGNORE_RE.search(ln):
            out.append((i, ln.strip()))
    return out


def walk_rs():
    r = subprocess.run(['git', '-C', REPO, 'ls-files', 'src', 'tests'], capture_output=True)
    return [p for p in r.stdout.decode('utf-8').splitlines() if p.endswith('.rs')]


print('=' * 70)
print('c7-3 investigation: are the 2 Doc-test `ignored` pre-existing at', BASE, '?')
print()

base_all = {}
cur_all = {}
for p in walk_rs():
    b = ignore_blocks(git_show(p, BASE))
    c = ignore_blocks(cur(p))
    if b:
        base_all[p] = b
    if c:
        cur_all[p] = c

print('--- baseline doc `ignore blocks ---')
for p, v in base_all.items():
    for ln, txt in v:
        print(f'  {p}:{ln}: {txt}')
print('  baseline total =', sum(len(v) for v in base_all.values()))

print()
print('--- current doc `ignore blocks ---')
for p, v in cur_all.items():
    for ln, txt in v:
        print(f'  {p}:{ln}: {txt}')
print('  current total =', sum(len(v) for v in cur_all.values()))

base_set = {(p, txt) for p, v in base_all.items() for _, txt in v}
cur_set = {(p, txt) for p, v in cur_all.items() for _, txt in v}
new = cur_set - base_set
lost = base_set - cur_set
print()
print('  new since baseline (must be empty) =', len(new))
for n in sorted(new):
    print('   NEW:', n)
print('  lost since baseline =', len(lost))
for m in sorted(lost):
    print('   LOST:', m)

pre_existing = len(new) == 0
print()
print('CONCLUSION: doc-test `ignored` count is', 'PRE-EXISTING (not introduced by this card)'
      if pre_existing else 'INTRODUCED BY THIS CARD')
sys.exit(0 if pre_existing else 1)
