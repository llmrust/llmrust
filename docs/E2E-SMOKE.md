# E2E Smoke — Operator & Secret Setup Guide

Low-budget real-upstream smoke testing for llmrust (SPCC §11.7 `E2E-001`).
This document is the operator guide: how the harness runs, which secrets must
be configured, and what a healthy run looks like. It is **not** a substitute
for local contract tests — E2E exists only to surface protocol drift that
fixtures cannot cover.

- Harness: `examples/e2e_smoke.rs`
- Workflow: `.github/workflows/e2e-smoke.yml` (manual `workflow_dispatch` + weekly schedule)
- Gate: `LLMRUST_E2E=1` — without it the harness prints `skipped` and exits 0
  (zero network, zero execution)

---

## 1. Pinned model IDs (verified 2026-08-03)

Model IDs are **exact and official-verified** (SPCC §0.2: external protocol
references must not be from memory). The verification date is 2026-08-03;
re-verify before changing any of these.

| Provider | Model ID | Input → Output ($/1M) | Official source (retrieved 2026-08-03) |
|----------|----------|----------------------|----------------------------------------|
| openai chat | `gpt-5-nano` | 0.05 → 0.40 | <https://platform.openai.com/docs/pricing> |
| openai embeddings | `text-embedding-3-small` | 0.02 (input) | <https://platform.openai.com/docs/pricing> |
| anthropic | `claude-haiku-4-5` | 1.00 → 5.00 | <https://www.anthropic.com/pricing> · <https://platform.claude.com/docs/en/about-claude/models/overview> |
| google | `gemini-3.1-flash-lite` | 0.25 → 1.50 | <https://ai.google.dev/gemini-api/docs/pricing> |
| deepseek | `deepseek-v4-flash` | 0.14 → 0.28 (cache miss) | <https://api-docs.deepseek.com/quick_start/pricing> |
| moonshot | `kimi-k2.6` | 0.95 → 4.00 (cache miss) | <https://platform.kimi.ai/docs/pricing/chat-k26> |
| openrouter | `nvidia/nemotron-3-ultra-550b-a55b:free` | 0 → 0 (free tier) | <https://openrouter.ai/api/v1/models> |
| ollama | `llama3.2` (local, 3B default) | — | <https://ollama.com/library/llama3.2> |

### Why these IDs (drift notes)

- **openai `gpt-5-nano`** — cheapest non-reasoning chat model as of 2026-08-03.
  `gpt-4o-mini` ($0.15/$0.60) is still on sale but no longer the cheapest.
- **anthropic `claude-haiku-4-5`** — `claude-3-5-haiku` has left the current
  lineup; Haiku 4.5 is the cheapest Claude (alias `claude-haiku-4-5`, dated
  snapshot `claude-haiku-4-5-20251001`).
- **google `gemini-3.1-flash-lite`** — `gemini-1.5-flash` was retired
  2025-05-24; Flash-Lite 3.1 is the current cheapest stable flash tier
  (GA 2026-05-07, retirement 2027-05-07).
- **deepseek `deepseek-v4-flash`** — legacy `deepseek-chat` / `deepseek-reasoner`
  were discontinued 2026-07-24 (official changelog 2026-04-24). V4-Flash is the
  current chat model (snapshot `DeepSeek-V4-Flash-0731`).
- **moonshot `kimi-k2.6`** — Moonshot V1 series (`moonshot-v1-8k`) faces a full
  platform sunset on **2026-08-31** (platform rebranded to
  platform.kimi.ai); `kimi-k2.6` is the current general model. If the real run
  reports moonshot connection failures, record the drift evidence as an
  "explained upstream change" — a follow-up task candidate to re-point the
  moonshot `base_url` is tracked (not fixed in this card).
- **openrouter free tier** — the pinned `nvidia/nemotron-3-ultra-550b-a55b:free`
  was re-confirmed present in the live `/api/v1/models` listing on 2026-08-03
  (architect ruling's fallback chain, step ①). Free-tier routes carry rate
  limits; a rate-limited run records `429`/`error` and is retried on the next
  manual run. Fallback chain if it is ever removed: ②
  `nvidia/nemotron-3-nano-omni-30b-a3b:free`, ③ `openrouter/free`, ④ cheapest
  paid tier (≤ $0.05/$0.10 per 1M) pinned at that time.
- **ollama `llama3.2`** — local, no price; skip trigger is **server
  unreachable** (2 s connect probe to `127.0.0.1:11434`), not "no key"
  (architect ruling SHOULD-3).

## 2. Budget ceiling

- **Per run ≤ $0.10** (constant `BUDGET_USD_CENTS = 10` in the harness).
- Fixed minimal prompt `"Reply with the single word: pong"` (≤ 20 words),
  `max_tokens ≤ 128` per chat path, 30 s timeout, concurrency ≤ 2.
- Anthropic reasoning path uses `budget_tokens = 1024` and
  `max_tokens = 2048` (REA-002 requires a non-empty thinking budget; ≈ $0.015
  at Haiku 4.5 pricing — still ≪ $0.10).
- Estimated per-run cost (one request per path): openai ≈ $0.00005,
  anthropic ≈ $0.0007, google ≈ $0.0002, deepseek ≈ $0.00004,
  moonshot ≈ $0.0005, openrouter $0, embeddings ≈ $0.000001.

## 3. Secret configuration (Owner setup)

The workflow references one GitHub Actions secret per provider. **Until a
secret is configured, GitHub passes an empty string and the harness reports
`skipped` (never `failed`).** Before the first real run, the Owner must add
these to **Repository → Settings → Secrets and variables → Actions**:

| Secret name | Provider | Notes |
|-------------|----------|-------|
| `OPENAI_API_KEY` | openai | chat + embeddings |
| `ANTHROPIC_API_KEY` | anthropic | — |
| `GOOGLE_API_KEY` | google | sent via `x-goog-api-key` header |
| `DEEPSEEK_API_KEY` | deepseek | — |
| `MOONSHOT_API_KEY` | moonshot | — |
| `OPENROUTER_API_KEY` | openrouter | free-tier route still needs a key |

Ollama needs no secret — the runner checks for a local server at
`127.0.0.1:11434` (2 s probe) and reports `skipped` when unreachable (this is
the expected state on GitHub Actions runners).

The workflow is **manual + scheduled only** (`workflow_dispatch` +
weekly `schedule`). It is never attached to `pull_request` or `push`, so fork
PRs never enter the secret context.

## 4. Running

```bash
# Local smoke run (real upstream, requires keys)
LLMRUST_E2E=1 cargo run --example e2e_smoke

# Local contract verification (no upstream calls — the CI-safe path)
cargo test --example e2e_smoke
```

Output is a redacted 6-field CSV:

```
provider, model, ok|error|skipped, status, ms, error_kind
```

- `status` is the HTTP status when known (`200` on success), `-` otherwise.
- `error_kind` is one of `http | api | stream | parse | unknown_provider |
  unsupported | timeout | semaphore`.
- **Never** printed: prompts, response bodies, API keys, full URLs.

## 5. Stability & DoD

- DoD (SPCC §11.7): supported paths succeed; `Unsupported` paths behave per
  contract; failure logs are redacted; forks have no secret exposure; two
  consecutive manual runs agree or have an explained upstream change.
- Reasoning subset (O-1 carry-over): openai exercises `reasoning_effort`
  (REA-003 mapping); anthropic exercises extended thinking (REA-002 mapping);
  other providers are per capability matrix — a contract-conformant
  `Unsupported` is a **valid** E2E result, not a failure. The outcome of each
  reasoning path is recorded in the run summary.
- Evidence for the state record after a real run: run URL, provider/model
  identifiers, timestamp, redacted summary, proof the cost stayed within
  $0.10, and the two-run consistency statement.
