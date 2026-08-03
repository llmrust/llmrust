# Compatibility & Upgrade Notes — llmrust 0.1.3

> **Release date: 2026-08-03**

> Audience: downstream crates upgrading from 0.1.1 or 0.1.2 (and especially anyone
> affected by the yanked 0.1.2).
>
> Machine-readable baseline: [`docs/api-inventory.json`](../docs/api-inventory.json)
> (schema `llmrust-api-inventory/1.0`, `task: API-001`, `issue: 100`). This document is
> the human-readable companion; where they disagree, `api-inventory.json` is authoritative.

## 1. Why 0.1.3 exists

0.1.3 is a **governance + freeze + incident-remediation** release. It is **not a feature
release**. Its purpose:

- Lock the public API surface for the 0.1.x line (Track ① of API-002).
- Remediate the 0.1.2 publishing incident (see §2).
- Carry two narrow, behavior-touching fixes that were already in flight (see §4) — these are
  **not** "feature work"; one is a log-noise reduction and the other is a documentation
  correction that aligns published metadata with existing behavior.

Because 0.1.3 is not a feature release, **no new provider capabilities are introduced**.

## 2. The 0.1.2 incident and yank semantics

`0.1.2` was **yanked** from crates.io. Important, precise semantics:

- **Yank ≠ deletion.** The `0.1.2` artifact still exists on crates.io and can be resolved by
  any `Cargo.lock` that already pins it. Yanking only prevents *new* resolution from picking
  `0.1.2` when no version is pinned. We do **not** claim `0.1.2` is unrecoverable, and we do
  **not** describe the yank as a "removal".
- **`0.1.2` stays yanked.** It is **not** un-yanked by the 0.1.3 release. 0.1.3 becomes the
  latest non-yanked version; `0.1.2` exits the "latest" position but remains installable via
  an explicit pin.
- **Avoiding the dirty version.** If your `Cargo.lock` still resolves `0.1.2`, do not hand-edit
  it to force `0.1.2` to persist. Either:
  - pin explicitly (`llmrust = "=0.1.2"`) if you must stay, or
  - upgrade to `llmrust = "0.1.3"` (recommended — see §6).

## 3. API surface freeze (machine baseline)

The freeze is defined in [`docs/api-inventory.json`](../docs/api-inventory.json). Key facts:

- Classifications used: `STABLE`, `STABLE-ADDITIVE` (requires `#[non_exhaustive]`),
  `UNSTABLE`.
- The `diffs.v0_1_2_to_current_main` array is **empty** — this proves that API-002 (fsync /
  schema-version plumbing) and API-003 (Retry/Provider contract freeze) introduced **zero
  changes to the public API surface**. They are internal/contract fixes, not API changes.
- The only API-surface diff in the entire 0.1.x line is `diffs.v0_1_1_to_v0_1_2`:
  `ThinkingConfig` (enum), `ChatRequest.thinking` (field), and `ChatRequest::with_thinking`
  (method) were added — all non-breaking, adopted as the 0.1.3 baseline per adjudication **D7**.
- The proxy module DTOs are classified `UNSTABLE` (adjudication **D6**) and are exempt from the
  semver gate (feature-gated, not in the default-feature build the gate checks).

### 3.1 Reconciliation sign-off (claims ↔ baseline)

| This doc claims | `api-inventory.json` anchor | Status |
|-----------------|-----------------------------|--------|
| Freeze baseline = `api-inventory.json` | `task: API-001`, `issue: 100` | ✅ |
| `v0_1_2_to_current_main` empty (API-002/003 zero API diff) | `diffs.v0_1_2_to_current_main: []` | ✅ |
| `ThinkingConfig`/`ChatRequest.thinking`/`with_thinking` added in 0.1.2, adopted D7 | `diffs.v0_1_1_to_v0_1_2`, adjudication `D7` | ✅ |
| `FinishReason` set frozen for 0.1.x | adjudication `D1` (`FinishReason` `STABLE`) | ✅ |
| `ChatResponse` frozen | adjudication `D2` (`ChatResponse` `STABLE`) | ✅ |
| proxy DTOs `UNSTABLE`, gate-exempt | adjudication `D6` | ✅ |

## 4. Behavior remediations shipped in 0.1.3

These are the only behavior-touching changes in 0.1.3, and both are narrow:

- **E-002 — `n > 1` advisory de-duplicated.** `warn_if_unsupported_n` now emits the advisory
  once per `(provider, n)` for the process lifetime, instead of repeating it on every
  `RetryProvider` retry attempt. Pure log-noise reduction; **no functional change**. Signature,
  `Provider` trait, and retry policy are untouched.
- **429 retry-policy documentation corrected.** `RetryProvider` does **not** retry HTTP `429`;
  it retries only `5xx`, network errors, and transient stream errors. The previously published
  `llmrust.capabilities.json` incorrectly listed `"429 (rate limit)"` under `retry_on`; this is
  corrected to match `should_retry` (all `4xx`, including `429`, return `false`). **Runtime
  behavior is unchanged** — only the published metadata was wrong.
  - **Important distinction:** the **Router** *does* fail over on `429` (it treats `429` as
    transient and switches deployment). That is a *separate* mechanism from `RetryProvider`'s
    retry policy and is unchanged. Do not conflate the two.

## 5. Future API debt (0.2 evaluation — NOT promised)

The following are explicitly **evaluation items for 0.2**, not commitments for 0.1.x. We list
them so downstream authors know where the line is drawn:

- **`FinishReason` set extension.** The variant set is **frozen for 0.1.x** (adding a variant
  would be breaking). Any new variant lands only in 0.2. Use `FinishReason::Other` for values
  not yet modeled.
- **`ChatResponse` `#[non_exhaustive]`.** Currently not applied; adding it later is breaking and
  is a 0.2 decision (adjudication **D2**).
- **`ThinkingConfig` root re-export.** `ThinkingConfig` is `STABLE` but intentionally **not**
  root-reexported (adjudication **D3**); access it via `llmrust::types::ThinkingConfig`. Any
  re-export change is a 0.2 decision.

## 6. Upgrade guidance

- **Cargo.toml:** change `llmrust = "0.1.1"` (or `"0.1.2"`) to `llmrust = "0.1.3"`.
- **Lockfile:** `cargo update -p llmrust` (or remove the pinned `0.1.2` entry and re-resolve).
- **Code migration:** **none required.** No public type, method, or field was removed or renamed.
  The `ThinkingConfig` API added in 0.1.2 is unchanged in 0.1.3.
- **If you were on `0.1.2`:** upgrade to `0.1.3` to leave the yanked version (see §2). There is
  no behavioral regression versus `0.1.2` beyond the E-002 log-noise reduction and the 429
  *documentation* correction (runtime retry behavior is identical).

```rust
// Minimal compile-check snippet — no behavior change versus 0.1.2.
use llmrust::types::{ChatRequest, ThinkingConfig};

let req = ChatRequest::new("openai/gpt-4o", "summarize this")
    .with_thinking(ThinkingConfig::Disabled);
assert!(req.thinking.is_some());
```

## 7. Errata acknowledgements

- **E-002** — `n > 1` advisory amplification under `RetryProvider` (fixed in 0.1.3, §4).
- **E-004** — `FinishReason` documentation wording: clarified that the variant set is frozen
  for 0.1.x (see `AGENTS.md` and §5). Docs-only; no code change.
- **D4** — `ThinkingConfig` was undocumented in `AGENTS.md` / `CAPABILITIES.md`; now documented
  as request-side contract, adopted D7, with explicit "no provider implements thinking yet"
  status (see `CAPABILITIES.md` and §3/§5).
