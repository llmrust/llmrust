import os
import subprocess
import sys

REPO = r'D:\llmrust-0.1.5'
BASE = '33a8e3e^'
TICK = chr(96) * 3


def git_show(path, rev):
    r = subprocess.run(['git', '-C', REPO, 'show', f'{rev}:{path}'], capture_output=True)
    return r.stdout.decode('utf-8') if r.returncode == 0 else None


def cur(path):
    full = os.path.join(REPO, path.replace('/', os.sep))
    try:
        return open(full, encoding='utf-8').read()
    except FileNotFoundError:
        return None


def fences(text):
    out = []
    if text is None:
        return out
    for i, ln in enumerate(text.splitlines(), 1):
        s = ln.strip()
        if s.startswith('///') or s.startswith('//!'):
            body = s[3:].strip()
            if body.startswith(TICK):
                out.append((i, body))
    return out


def walk_rs():
    r = subprocess.run(['git', '-C', REPO, 'ls-files', 'src', 'tests'], capture_output=True)
    return [p for p in r.stdout.decode('utf-8').splitlines() if p.endswith('.rs')]


print('=' * 70)
print('Which doc fences carry a rustdoc ATTRIBUTE (ignore / no_run / text / compile_fail)?')
print()

for label, rev in (('BASELINE ' + BASE, BASE), ('CURRENT (working tree)', None)):
    print('---', label, '---')
    total = 0
    for p in walk_rs():
        text = git_show(p, rev) if rev else cur(p)
        for ln, body in fences(text):
            total += 1
            attr = body[3:].strip()
            if attr:
                print(f'  {p}:{ln}: {body}')
    print('  total doc fences =', total)
    print()

print('=' * 70)
print('rustdoc attributes present in current tree (ignore-like only):')
found = 0
for p in walk_rs():
    for ln, body in fences(cur(p)):
        attr = body[3:].strip()
        if attr in ('ignore', 'no_run', 'text', 'compile_fail'):
            print(f'  {p}:{ln}: {body}')
            found += 1
if found == 0:
    print('  (none)')
print()
print('NOTE: `cargo test --doc` reported 2 ignored. A doc-test is also counted as')
print('ignored when it is compiled but not run, and rustdoc marks a block as')
print('ignored via the `ignore` attribute. If no `ignore` attribute exists, the')
print('2 ignored doc-tests come from rustdoc itself and are baseline behaviour,')
print('not a test-weakening introduced by this card.')
