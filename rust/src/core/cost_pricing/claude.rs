use super::super::{claude_routed_pricing, models_dev_pricing};
use super::{CLAUDE_PRICING, CODEX_PRICING, ClaudePricing, CostUsagePricing};

const CODEX_LONG_CONTEXT_THRESHOLD_FOR_CLAUDE: u64 = 272_000;

pub(crate) fn bundled_codex_long_context_threshold(model: &str) -> Option<u64> {
    let key = CostUsagePricing::normalize_codex_model(model);
    CODEX_PRICING.get(key.as_str()).and_then(|pricing| {
        pricing
            .long_context
            .map(|_| CODEX_LONG_CONTEXT_THRESHOLD_FOR_CLAUDE)
    })
}

/// Resolved Claude pricing source used by the local scanner's invocation memo.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ClaudePricingResolution {
    BuiltIn(ClaudePricing),
    ModelsDev {
        pricing: models_dev_pricing::DynamicModelPricing,
        threshold_tokens: Option<u64>,
    },
}

impl CostUsagePricing {
    /// Normalize a Claude model name for pricing lookup
    pub fn normalize_claude_model(raw: &str) -> String {
        let mut trimmed = raw.trim().to_string();

        // Remove "anthropic." prefix
        if let Some(rest) = trimmed.strip_prefix("anthropic.") {
            trimmed = rest.to_string();
        }

        // Handle nested model names like "anthropic.claude-sonnet-4.claude-sonnet-4-20250514"
        if trimmed.contains("claude-")
            && let Some(last_dot) = trimmed.rfind('.')
        {
            let tail = &trimmed[last_dot + 1..];
            if tail.starts_with("claude-") {
                trimmed = tail.to_string();
            }
        }

        // Remove version suffix like "-v1:0"
        let version_pattern = regex_lite::Regex::new(r"-v\d+:\d+$").unwrap();
        trimmed = version_pattern.replace(&trimmed, "").to_string();

        // Try without date suffix if base exists in pricing
        let date_pattern = regex_lite::Regex::new(r"-\d{8}$").unwrap();
        if let Some(mat) = date_pattern.find(&trimmed) {
            let base = &trimmed[..mat.start()];
            if CLAUDE_PRICING.contains_key(base) {
                return base.to_string();
            }
        }

        trimmed
    }

    /// Resolve one Claude model without rereading the models.dev artifact.
    pub(crate) fn resolve_claude_pricing(
        model: &str,
        normalized: &str,
        pricing_snapshot: Option<&models_dev_pricing::ModelsDevPricingSnapshot>,
    ) -> Option<ClaudePricingResolution> {
        if let Some(pricing) = CLAUDE_PRICING.get(normalized) {
            return Some(ClaudePricingResolution::BuiltIn(*pricing));
        }
        pricing_snapshot
            .and_then(|snapshot| {
                claude_routed_pricing::resolve_with_snapshot(model, normalized, snapshot)
            })
            .map(
                |(pricing, threshold_tokens)| ClaudePricingResolution::ModelsDev {
                    pricing,
                    threshold_tokens,
                },
            )
    }

    /// Calculate cost from a previously resolved Claude pricing source.
    pub(crate) fn claude_cost_usd_from_resolution(
        resolution: ClaudePricingResolution,
        input_tokens: i32,
        cache_read_input_tokens: i32,
        cache_creation_input_tokens: i32,
        output_tokens: i32,
    ) -> f64 {
        Self::claude_cost_usd_u64_from_resolution(
            resolution,
            u64::try_from(input_tokens).unwrap_or(0),
            u64::try_from(cache_read_input_tokens).unwrap_or(0),
            u64::try_from(cache_creation_input_tokens).unwrap_or(0),
            u64::try_from(output_tokens).unwrap_or(0),
        )
    }

    /// Calculate cost from a resolved Claude pricing source without narrowing
    /// untrusted local-history counters to the API-oriented signed type.
    pub(crate) fn claude_cost_usd_u64_from_resolution(
        resolution: ClaudePricingResolution,
        input_tokens: u64,
        cache_read_input_tokens: u64,
        cache_creation_input_tokens: u64,
        output_tokens: u64,
    ) -> f64 {
        match resolution {
            ClaudePricingResolution::BuiltIn(pricing) => {
                fn tiered(
                    tokens: u64,
                    base: f64,
                    above: Option<f64>,
                    threshold: Option<i32>,
                ) -> f64 {
                    match (threshold, above) {
                        (Some(thresh), Some(above_rate)) => {
                            let thresh = u64::try_from(thresh).unwrap_or(0);
                            let below = tokens.min(thresh);
                            let over = tokens.saturating_sub(thresh);
                            (below as f64) * base + (over as f64) * above_rate
                        }
                        _ => (tokens as f64) * base,
                    }
                }

                tiered(
                    input_tokens,
                    pricing.input_cost_per_token,
                    pricing.input_cost_per_token_above_threshold,
                    pricing.threshold_tokens,
                ) + tiered(
                    cache_read_input_tokens,
                    pricing.cache_read_input_cost_per_token,
                    pricing.cache_read_input_cost_per_token_above_threshold,
                    pricing.threshold_tokens,
                ) + tiered(
                    cache_creation_input_tokens,
                    pricing.cache_creation_input_cost_per_token,
                    pricing.cache_creation_input_cost_per_token_above_threshold,
                    pricing.threshold_tokens,
                ) + tiered(
                    output_tokens,
                    pricing.output_cost_per_token,
                    pricing.output_cost_per_token_above_threshold,
                    pricing.threshold_tokens,
                )
            }
            ClaudePricingResolution::ModelsDev {
                pricing,
                threshold_tokens,
            } => {
                let use_tier = threshold_tokens.is_some_and(|threshold| {
                    input_tokens
                        .checked_add(cache_read_input_tokens)
                        .and_then(|value| value.checked_add(cache_creation_input_tokens))
                        .is_none_or(|total| total > threshold)
                });
                let pick = |base: f64, above: Option<f64>| {
                    if use_tier {
                        above.unwrap_or(base)
                    } else {
                        base
                    }
                };
                let input_rate = pick(
                    pricing.input_cost_per_token,
                    pricing.input_cost_per_token_above_threshold,
                );
                let cache_read_rate = if use_tier {
                    pricing
                        .cache_read_input_cost_per_token_above_threshold
                        .or(pricing.cache_read_input_cost_per_token)
                        .unwrap_or(input_rate)
                } else {
                    pricing
                        .cache_read_input_cost_per_token
                        .unwrap_or(input_rate)
                };
                let cache_write_rate = if use_tier {
                    pricing
                        .cache_write_input_cost_per_token_above_threshold
                        .or(pricing.cache_write_input_cost_per_token)
                        .unwrap_or(input_rate)
                } else {
                    pricing
                        .cache_write_input_cost_per_token
                        .unwrap_or(input_rate)
                };
                let output_rate = pick(
                    pricing.output_cost_per_token,
                    pricing.output_cost_per_token_above_threshold,
                );

                (input_tokens as f64) * input_rate
                    + (cache_read_input_tokens as f64) * cache_read_rate
                    + (cache_creation_input_tokens as f64) * cache_write_rate
                    + (output_tokens as f64) * output_rate
            }
        }
    }

    pub(crate) fn claude_input_cost_per_token_from_resolution(
        resolution: ClaudePricingResolution,
    ) -> f64 {
        match resolution {
            ClaudePricingResolution::BuiltIn(pricing) => pricing.input_cost_per_token,
            ClaudePricingResolution::ModelsDev { pricing, .. } => pricing.input_cost_per_token,
        }
    }

    #[cfg(test)]
    pub(crate) fn claude_models_dev_target(model: &str) -> Option<(&'static str, String)> {
        claude_routed_pricing::models_dev_target(model, Self::normalize_claude_model(model))
    }

    /// Calculate cost for Claude usage in USD
    pub fn claude_cost_usd(
        model: &str,
        input_tokens: i32,
        cache_read_input_tokens: i32,
        cache_creation_input_tokens: i32,
        output_tokens: i32,
    ) -> Option<f64> {
        let key = Self::normalize_claude_model(model);
        if let Some(pricing) = CLAUDE_PRICING.get(key.as_str()) {
            return Some(Self::claude_cost_usd_from_resolution(
                ClaudePricingResolution::BuiltIn(*pricing),
                input_tokens,
                cache_read_input_tokens,
                cache_creation_input_tokens,
                output_tokens,
            ));
        }

        claude_routed_pricing::cost_usd(
            model,
            Self::normalize_claude_model(model),
            input_tokens,
            cache_read_input_tokens,
            cache_creation_input_tokens,
            output_tokens,
        )
    }

    /// Base per-token input rate for a Claude model. Exposed for callers that
    /// need a rate the standard cost function doesn't model — e.g. the usage
    /// scanner's one-hour cache-write premium, billed at 2x the input rate.
    pub fn claude_input_cost_per_token(model: &str) -> Option<f64> {
        let key = Self::normalize_claude_model(model);
        if let Some(pricing) = CLAUDE_PRICING.get(key.as_str()) {
            return Some(pricing.input_cost_per_token);
        }
        claude_routed_pricing::input_cost_per_token(model, Self::normalize_claude_model(model))
    }
}
