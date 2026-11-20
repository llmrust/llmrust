import re
txt = open(r'D:\llmrust-0.1.5\.workforce\evidence\DELTA-NULL\card-c2-list.txt', encoding='utf-8').read()
names = ['stream_terminal_chunk_with_null_delta_parses',
         'stream_delta_content_still_parses_with_option',
         'stream_terminal_chunk_with_absent_delta_parses']
ok = True
for n in names:
    hits = [ln for ln in txt.splitlines() if ln.strip().startswith('providers::compat::tests::'+n) or ln.strip().startswith(n)]
    print(n, 'occurrences=', len(hits))
    for h in hits:
        print('   ', h.strip())
        if ': ignored' in h:
            print('   !! HAS : ignored'); ok = False
    if len(hits) != 1:
        ok = False
print('C2_LIST_PASS' if ok else 'C2_LIST_FAIL')
