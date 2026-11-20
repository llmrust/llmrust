import json, io
p = 'tests/hotspot_ledger.json'
raw = open(p, encoding='utf-8').read()
d = json.loads(raw)
old = d['baseline_lines']['src/providers/compat.rs']
new = 1899
reason = (
    "DELTA-NULL (post-0.1.5 bugfix, task DELTA-NULL): OpenAI-compatible endpoints legitimately send the "
    "TERMINAL stream frame as {\"delta\":null,\"finish_reason\":\"stop\",...} (official OpenAI semantics; also "
    "observed on llama.cpp, some vLLM builds and self-hosted proxies), and some upstreams omit the field "
    "entirely. CompStreamChoice declared `delta: CompDelta` (non-Option), so `delta:null` failed serde with "
    "`missing field `delta`` and parse_sse_line returned LlmError::Parse(\"failed to parse OpenAI-compatible "
    "stream chunk: missing field `delta`\") for the whole stream => the caller surfaced \"Invalid response\" => "
    "every streamed tool-calling turn failed at the real tool call. Measured reproduction (downstream beeli "
    "vertical slice S3): endpoint http://127.0.0.1:18778/v1, both nous/meituan/longcat-2.5:free and "
    "nous/upstage/solar-mini4:free trigger it; terminal frame verbatim: "
    "{\"choices\":[{\"delta\":null,\"index\":0,\"finish_reason\":\"stop\",\"message\":{\"role\":\"assistant\","
    "\"content\":\"\u53ef\u7528\"},...}]}. Fix: `#[serde(default)] delta: Option<CompDelta>` + a single "
    "`empty_delta` placeholder bound at the top of the choices arm, so the three usage sites (tool_calls "
    "ingest, reasoning/reasoning_content, content) keep their logic unchanged. Growth is real fix + tests, "
    "not inflation: +56 wc -l measured (1843 -> 1899), of which the implementation is ~15 lines (struct doc "
    "+ Option/default, the empty_delta binding, the three rewritten usage sites) and ~41 lines are three "
    "regression tests (null delta parses / absent delta parses / normal increment still parses)."
)
d['baseline_lines']['src/providers/compat.rs'] = new
d['adjustments'].append({
    "file": "src/providers/compat.rs",
    "old": old,
    "new": new,
    "reason": reason,
    "authorized_by": "Boss 2026-11-20 \u6307\u4ee4\uff1allmrust \u662f\u6211\u4eec\u81ea\u5df1\u7684\uff0c\u53d1\u73b0\u95ee\u9898\u5c31\u81ea\u5df1\u4fee\u3001\u63d0 PR\u3001\u8dd1 CI\u3001\u5408\u5e76\u53d1\u7248\uff1bledger adjustment per \u00a711.1.4",
    "date": "2026-11-20",
})
open(p, 'w', encoding='utf-8', newline='\n').write(json.dumps(d, ensure_ascii=False, indent=2) + '\n')
print('OK old=', old, 'new=', new, 'adjustments=', len(d['adjustments']))
