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
//! };
//! // 1.0 * 0.0025 + 0.5 * 0.01 = 0.0075
//! let cost = pricing.estimate_cost(&usage);
//! assert!((cost - 0.0075).abs() < 1e-9);
//! ```

use serde::{Deserialize, Serialize};

use crate::types::Usage;

/// Per-token pricing for a model, in US dollars per 1,000 tokens.
///
/// Prompt (input) and completion (output) tokens are usually billed at
/// different rates, so they are tracked separately. Construct one with
/// [`ModelPricing::new`] and combine it with a [`Usage`]
/// via [`ModelPricing::estimate_cost`] (or the
/// [`Usage::estimated_cost`](crate::types::Usage::estimated_cost) convenience
/// method).
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
    /// provider-specific discounts, cached-token rates, or rounding.
    pub fn estimate_cost(&self, usage: &Usage) -> f64 {
        let prompt_cost = usage.prompt_tokens as f64 / 1000.0 * self.prompt_per_1k;
        let completion_cost = usage.completion_tokens as f64 / 1000.0 * self.completion_per_1k;
        prompt_cost + completion_cost
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
}
