import sys

p = 'src/providers/compat.rs'
s = open(p, encoding='utf-8').read()
mode = sys.argv[1]

STRUCT_FIXED = '    #[serde(default)]\n    delta: Option<CompDelta>,'
STRUCT_BUGGY = '    delta: CompDelta,'

BLOCK_FIXED = (
    '            // DELTA-NULL: `delta` may be absent or explicitly `null` on terminal\n'
    '            // frames; substitute an empty delta so downstream logic is unchanged.\n'
    '            let empty_delta = CompDelta::default();\n'
    '            let delta = choice.delta.as_ref().unwrap_or(&empty_delta);\n'
    '            if let Some(deltas) = &delta.tool_calls {'
)
BLOCK_BUGGY = '            if let Some(deltas) = &choice.delta.tool_calls {'

THINK_FIXED = (
    '            let thinking = delta\n'
    '                .reasoning\n'
    '                .clone()\n'
    '                .or_else(|| delta.reasoning_content.clone());'
)
THINK_BUGGY = (
    '            let thinking = choice\n'
    '                .delta\n'
    '                .reasoning\n'
    '                .clone()\n'
    '                .or_else(|| choice.delta.reasoning_content.clone());'
)

CONTENT_FIXED = '                delta: delta.content.clone().unwrap_or_default(),'
CONTENT_BUGGY = '                delta: choice.delta.content.clone().unwrap_or_default(),'

if mode == 'buggy':
    pairs = [(STRUCT_FIXED, STRUCT_BUGGY), (BLOCK_FIXED, BLOCK_BUGGY),
             (THINK_FIXED, THINK_BUGGY), (CONTENT_FIXED, CONTENT_BUGGY)]
else:
    pairs = [(STRUCT_BUGGY, STRUCT_FIXED), (BLOCK_BUGGY, BLOCK_FIXED),
             (THINK_BUGGY, THINK_FIXED), (CONTENT_BUGGY, CONTENT_FIXED)]

for old, new in pairs:
    if old not in s:
        raise SystemExit('NOT FOUND for mode %s: %r' % (mode, old[:60]))
    s = s.replace(old, new, 1)

open(p, 'w', encoding='utf-8', newline='\n').write(s)
print('APPLIED mode=%s' % mode)
