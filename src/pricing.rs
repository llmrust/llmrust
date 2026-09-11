//! Cost estimation for token [`Usage`].
//!
//! llmrust reports raw token counts in [`Usage`]; it has
//! no idea what any given model charges. [`ModelPricing`] lets a caller attach
//! per-1K-token prices and turn a [`Usage`] into an
//! estimated dollar amount, which is handy for budget tracking and request
//! tracing.
//!
//! Prices are expressed in US dollars per 1,000 tokens, matching how the major
//! providers publish their rates. Prompt (input) and completion (output)
//! tokens are billed separately.
//!
//! ```rust
//! use llmrust::{ModelPricing, Usage};
//!
//! let pricing = ModelPricing::new(0.0025, 0.01);
//! let usage = Usage {
//!     prompt_tokens: 1_000,
//!     completion_tokens: 500,
//!     total_tokens: 1_500,
//!     ..Default::default()
//! };
//! // 1.0 * 0.0025 + 0.5 * 0.01 = 0.0075
//! let cost = pricing.estimate_cost(&usage);
//! assert!((cost - 0.0075).abs() < 1e-9);
//! ```

use serde::{Deserialize, Serialize};

use crate::types::Usage;

/// **Cache pricing (CAP-006 R2)** — US dollars per 1,000 tokens.
///
/// Kept as a **separate type** on purpose: `ModelPricing` is externally
/// constructible, so adding fields to it is a **breaking** change (caught by the
/// `constructible_struct_adds_field` semver check). A new type plus new methods
/// is purely additive.
///
/// Rates are **per model, never global defaults**: measured cache-read
/// multipliers span 0.0083× (MiMo) to 0.5× (Zhipu) across providers.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct CachePricing {
    /// Cache-hit (read) price. `None` = fall back to the prompt rate.
    pub read_per_1k: Option<f64>,
    /// Cache-write price, 5-minute tier. `None` = fall back to the 1-hour tier,
    /// then to the prompt rate. Anthropic bills 1.25× the input rate here;
    /// most other providers bill nothing extra for writes.
    pub write_5m_per_1k: Option<f64>,
    /// Cache-write price, 1-hour tier. `None` = fall back to the prompt rate.
    /// Anthropic bills 2× the input rate here; providers without a fixed TTL
    /// (e.g. LRU-evicted caches) leave it `None`.
    pub write_1h_per_1k: Option<f64>,
}

impl CachePricing {
    /// Construct from the three cache rates (any may be `None`).
    pub const fn new(
        read_per_1k: Option<f64>,
        write_5m_per_1k: Option<f64>,
        write_1h_per_1k: Option<f64>,
    ) -> Self {
        Self {
            read_per_1k,
            write_5m_per_1k,
            write_1h_per_1k,
        }
    }

    /// Whether any cache rate is configured.
    pub const fn is_configured(&self) -> bool {
        self.read_per_1k.is_some()
            || self.write_5m_per_1k.is_some()
            || self.write_1h_per_1k.is_some()
    }
}

/// Per-token pricing for a model, in US dollars per 1,000 tokens.
///
/// Prompt (input) and completion (output) tokens are usually billed at
/// different rates, so they are tracked separately. Construct one with
/// [`ModelPricing::new`] and combine it with a [`Usage`]
/// via [`ModelPricing::estimate_cost`] (or the
/// [`Usage::estimated_cost`](crate::types::Usage::estimated_cost) convenience
/// method). Cache rates live in [`CachePricing`] and are applied by
/// [`ModelPricing::estimate_cost_with_cache`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ModelPricing {
    /// US dollars charged per 1,000 prompt (input) tokens.
    pub prompt_per_1k: f64,
    /// US dollars charged per 1,000 completion (output) tokens.
    pub completion_per_1k: f64,
}

impl ModelPricing {
    /// Create a pricing table from per-1,000-token prompt and completion
    /// prices, both in US dollars.
    pub const fn new(prompt_per_1k: f64, completion_per_1k: f64) -> Self {
        Self {
            prompt_per_1k,
            completion_per_1k,
        }
    }

    /// Estimate the cost, in US dollars, of the given token usage.
    ///
    /// The estimate is
    /// `prompt_tokens / 1000 * prompt_per_1k + completion_tokens / 1000 * completion_per_1k`.
    /// It reflects only the prices supplied here and does not account for
    /// provider-specific discounts, cached-token rates, or rounding — use
    /// [`Self::estimate_cost_with_cache`] for cache-aware accounting.
    pub fn estimate_cost(&self, usage: &Usage) -> f64 {
        let prompt_cost = usage.prompt_tokens as f64 / 1000.0 * self.prompt_per_1k;
        let completion_cost = usage.completion_tokens as f64 / 1000.0 * self.completion_per_1k;
        prompt_cost + completion_cost
    }

    /// Estimate the cost **with cache pricing** (`CNT-001` / `CAP-006` R2).
    ///
    /// `Usage::cache_read_tokens` / `Usage::cache_write_tokens` are *subsets* of
    /// `prompt_tokens`, so the prompt count is split rather than added to:
    ///
    /// ```text
    /// uncached = prompt_tokens - cache_read_tokens - cache_write_tokens   (saturating)
    /// cost     = uncached   * prompt_per_1k
    ///          + cache_read * cache.read_per_1k.unwrap_or(prompt_per_1k)
    ///          + cache_writ * cache_write_rate
    ///          + completion * completion_per_1k
    /// ```
    ///
    /// where `cache_write_rate` is the 5-minute tier when given, else the 1-hour
    /// tier, else `prompt_per_1k`. **Unconfigured tiers never silently become
    /// free**, and `total_tokens` is never used (it already includes the cache
    /// counts — using it would double-count).
    ///
    /// With [`CachePricing::default`] (all tiers `None`) this returns exactly
    /// what [`Self::estimate_cost`] returns.
    pub fn estimate_cost_with_cache(&self, usage: &Usage, cache: &CachePricing) -> f64 {
        let completion_cost = usage.completion_tokens as f64 / 1000.0 * self.completion_per_1k;
        if !cache.is_configured() {
            return self.estimate_cost(usage);
        }

        let cached_read = usage.cache_read_tokens.unwrap_or(0);
        let cached_write = usage.cache_write_tokens.unwrap_or(0);
        let uncached = (usage.prompt_tokens)
            .saturating_sub(cached_read)
            .saturating_sub(cached_write);

        let read_rate = cache.read_per_1k.unwrap_or(self.prompt_per_1k);
        let write_rate = cache
            .write_5m_per_1k
            .or(cache.write_1h_per_1k)
            .unwrap_or(self.prompt_per_1k);

        let uncached_cost = uncached as f64 / 1000.0 * self.prompt_per_1k;
        let read_cost = cached_read as f64 / 1000.0 * read_rate;
        let write_cost = cached_write as f64 / 1000.0 * write_rate;

        uncached_cost + read_cost + write_cost + completion_cost
    }
}

impl Usage {
    /// Estimate the cost of this usage, in US dollars, under the given
    /// [`ModelPricing`].
    ///
    /// Convenience wrapper around [`ModelPricing::estimate_cost`].
    pub fn estimated_cost(&self, pricing: &ModelPricing) -> f64 {
        pricing.estimate_cost(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(prompt: u64, completion: u64) -> Usage {
        Usage {
            prompt_tokens: prompt,
            completion_tokens: completion,
            total_tokens: prompt + completion,
            ..Default::default()
        }
    }

    #[test]
    fn estimates_cost_from_separate_rates() {
        let pricing = ModelPricing::new(0.0025, 0.01);
        let cost = pricing.estimate_cost(&usage(1_000, 500));
        // 1.0 * 0.0025 + 0.5 * 0.01 = 0.0075
        assert!((cost - 0.0075).abs() < 1e-9, "got {cost}");
    }

    #[test]
    fn zero_usage_is_zero_cost() {
        let pricing = ModelPricing::new(0.0025, 0.01);
        let cost = pricing.estimate_cost(&usage(0, 0));
        assert!(cost.abs() < 1e-9, "got {cost}");
    }

    #[test]
    fn usage_method_matches_pricing_method() {
        let pricing = ModelPricing::new(0.003, 0.015);
        let u = usage(1_234, 567);
        let via_usage = u.estimated_cost(&pricing);
        let via_pricing = pricing.estimate_cost(&u);
        assert!((via_usage - via_pricing).abs() < 1e-12, "methods disagree");
    }

    #[test]
    fn prompt_and_completion_priced_independently() {
        // Only completion tokens are priced here.
        let pricing = ModelPricing::new(0.0, 0.02);
        let cost = pricing.estimate_cost(&usage(10_000, 1_000));
        // 0 + 1.0 * 0.02 = 0.02
        assert!((cost - 0.02).abs() < 1e-9, "got {cost}");
    }

    #[test]
    fn serde_round_trips() {
        let pricing = ModelPricing::new(0.0025, 0.01);
        let json = serde_json::to_string(&pricing).unwrap();
        let back: ModelPricing = serde_json::from_str(&json).unwrap();
        assert_eq!(pricing, back);
    }

    #[test]
    fn estimated_cost_does_not_double_count_cache_or_reasoning_tokens() {
        // API-002 invariant: cost must derive only from prompt + completion
        // tokens. `total_tokens` already includes cache/reasoning counts, so
        // adding them again would double-count. Because ModelPricing only
        // carries prompt/completion rates, `estimated_cost` ignores
        // cache/reasoning tokens entirely — this test pins that behavior so a
        // future "add cache pricing" change cannot silently start
        // double-counting against `total_tokens`.
        let pricing = ModelPricing::new(1.0, 2.0);
        let with_cache = Usage {
            prompt_tokens: 1_000,
            completion_tokens: 2_000,
            total_tokens: 5_000, // already includes cache + reasoning
            cache_read_tokens: Some(1_500),
            cache_write_tokens: Some(500),
            reasoning_tokens: Some(1_000),
        };
        // expected: 1000/1000*1.0 + 2000/1000*2.0 = 1.0 + 4.0 = 5.0
        let cost = with_cache.estimated_cost(&pricing);
        assert!((cost - 5.0).abs() < 1e-9, "cost was {cost}, expected 5.0");

        // Dropping cache/reasoning tokens must NOT change the cost — proof they
        // are not counted (no double-counting).
        let without_cache = Usage {
            prompt_tokens: 1_000,
            completion_tokens: 2_000,
            total_tokens: 3_000,
            ..Default::default()
        };
        assert!(
            (without_cache.estimated_cost(&pricing) - 5.0).abs() < 1e-9,
            "cost must not depend on cache/reasoning tokens"
        );
    }

    // ---- CAP-006 R2: cache pricing path ----

    /// Non-breaking guarantee: with no cache rates configured, the cache-aware
    /// entry point returns exactly what the legacy entry point returns.
    #[test]
    fn unconfigured_cache_pricing_matches_legacy_estimate() {
        let pricing = ModelPricing::new(1.0, 2.0);
        let u = Usage {
            prompt_tokens: 1_000,
            completion_tokens: 2_000,
            total_tokens: 5_000,
            cache_read_tokens: Some(400),
            cache_write_tokens: Some(100),
            ..Default::default()
        };
        let cache = CachePricing::default();
        assert!(!cache.is_configured());
        assert!((pricing.estimate_cost_with_cache(&u, &cache) - 5.0).abs() < 1e-12);
        assert_eq!(
            pricing.estimate_cost_with_cache(&u, &cache),
            pricing.estimate_cost(&u)
        );
    }

    /// Anthropic-shaped example, in **per-1k** units (the API's unit):
    /// input 0.003/1k, read 0.0003/1k (0.1×), 5m write 0.00375/1k (1.25×),
    /// 1h write 0.006/1k (2×). 1,000 prompt tokens of which 600 read-hit and
    /// 100 written, plus 500 output.
    ///
    /// uncached 300 → 0.0009 ; read 600 → 0.00018 ; write 100 → 0.000375 ;
    /// output 500 → 0.0075  ⇒ 0.008955
    #[test]
    fn cache_prices_split_the_prompt_count() {
        let pricing = ModelPricing::new(0.003, 0.015);
        let cache = CachePricing::new(Some(0.0003), Some(0.00375), Some(0.006));
        let u = Usage {
            prompt_tokens: 1_000,
            completion_tokens: 500,
            total_tokens: 1_500,
            cache_read_tokens: Some(600),
            cache_write_tokens: Some(100),
            ..Default::default()
        };
        let expected = 0.0009 + 0.00018 + 0.000375 + 0.0075;
        let got = pricing.estimate_cost_with_cache(&u, &cache);
        assert!(
            (got - expected).abs() < 1e-12,
            "got {got}, expected {expected}"
        );
    }

    /// A cache hit must be **cheaper** than the same tokens uncached (the whole
    /// point of the money line), and a 1-hour write must cost more than a
    /// 5-minute write.
    #[test]
    fn cache_hit_is_cheaper_than_uncached_and_1h_write_costs_more() {
        let base = ModelPricing::new(0.003, 0.015);
        let uncached = Usage {
            prompt_tokens: 1_000,
            completion_tokens: 0,
            total_tokens: 1_000,
            ..Default::default()
        };
        let cached = Usage {
            prompt_tokens: 1_000,
            completion_tokens: 0,
            total_tokens: 1_000,
            cache_read_tokens: Some(1_000),
            ..Default::default()
        };
        let read = CachePricing::new(Some(0.0003), None, None);
        assert!(
            base.estimate_cost_with_cache(&cached, &read)
                < base.estimate_cost_with_cache(&uncached, &read),
            "a cache hit must be cheaper"
        );

        let written = Usage {
            prompt_tokens: 1_000,
            completion_tokens: 0,
            total_tokens: 1_000,
            cache_write_tokens: Some(1_000),
            ..Default::default()
        };
        let five_min = CachePricing::new(None, Some(0.00375), None);
        let one_hour = CachePricing::new(None, None, Some(0.006));
        assert!(
            base.estimate_cost_with_cache(&written, &one_hour)
                > base.estimate_cost_with_cache(&written, &five_min),
            "the 1-hour write tier must cost more than the 5-minute tier"
        );
    }

    /// Unconfigured tiers must never silently become free: an unset write price
    /// falls back to the prompt rate rather than 0.
    #[test]
    fn unconfigured_write_tier_falls_back_to_prompt_rate_not_zero() {
        let pricing = ModelPricing::new(0.003, 0.015);
        let cache = CachePricing::new(Some(0.0003), None, None);
        let u = Usage {
            prompt_tokens: 1_000,
            completion_tokens: 0,
            total_tokens: 1_000,
            cache_write_tokens: Some(1_000),
            ..Default::default()
        };
        // write is unconfigured → billed at prompt rate 0.003, NOT free (0.0)
        assert!((pricing.estimate_cost_with_cache(&u, &cache) - 0.003).abs() < 1e-12);
    }

    /// Saturation: cache counts exceeding the prompt count must not underflow.
    #[test]
    fn cache_counts_larger_than_prompt_do_not_underflow() {
        let pricing = ModelPricing::new(0.003, 0.015);
        let cache = CachePricing::new(Some(0.0003), Some(0.00375), None);
        let u = Usage {
            prompt_tokens: 10,
            completion_tokens: 0,
            total_tokens: 10,
            cache_read_tokens: Some(1_000),
            cache_write_tokens: Some(1_000),
            ..Default::default()
        };
        let cost = pricing.estimate_cost_with_cache(&u, &cache);
        assert!(cost.is_finite() && cost >= 0.0, "got {cost}");
    }

    /// Cache pricing survives serde round-trips (the catalog ships data, not code).
    #[test]
    fn cache_pricing_serde_round_trip() {
        let cache = CachePricing::new(Some(0.0003), Some(0.00375), None);
        let json = serde_json::to_string(&cache).unwrap();
        let back: CachePricing = serde_json::from_str(&json).unwrap();
        assert_eq!(cache, back);
    }

    /// `total_tokens` must never be used: it already includes the cache counts.
    /// Pricing the same tokens with and without a consistent `total_tokens` must
    /// not change the result.
    #[test]
    fn cache_pricing_ignores_total_tokens() {
        let pricing = ModelPricing::new(0.003, 0.015);
        let cache = CachePricing::new(Some(0.0003), Some(0.00375), None);
        let mut u = Usage {
            prompt_tokens: 1_000,
            completion_tokens: 100,
            total_tokens: 1_100,
            cache_read_tokens: Some(500),
            ..Default::default()
        };
        let first = pricing.estimate_cost_with_cache(&u, &cache);
        u.total_tokens = 9_999;
        let second = pricing.estimate_cost_with_cache(&u, &cache);
        assert_eq!(first, second, "total_tokens must not affect the estimate");
    }
}
