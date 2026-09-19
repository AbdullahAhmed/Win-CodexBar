use super::super::{claude_routed_pricing, models_dev_pricing};
use super::{CLAUDE_PRICING, ClaudePricing, CostUsagePricing};

/// Resolved Claude pricing source used by the local scanner's invocation memo.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ClaudePricingResolution {
    BuiltIn(ClaudePricing),
    ModelsDev(models_dev_pricing::DynamicModelPricing),
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
            .map(|pricing| {
                ClaudePricingResolution::ModelsDev(Self::apply_bundled_openai_threshold(
                    model, normalized, pricing,
                ))
            })
    }

    /// Claude Code may emit OpenAI models even though the transcript is being
    /// scanned through the Claude cost path. Keep the catalog's rates, but use
    /// the bundled Codex boundary when the model is a known long-context Codex
    /// model. Unknown OpenAI rows retain the catalog-provided boundary.
    fn apply_bundled_openai_threshold(
        model: &str,
        normalized: &str,
        mut pricing: models_dev_pricing::DynamicModelPricing,
    ) -> models_dev_pricing::DynamicModelPricing {
        let Some((provider, lookup_model)) =
            claude_routed_pricing::models_dev_target(model, normalized.to_string())
        else {
            return pricing;
        };
        if provider == "openai"
            && let Some(threshold) =
                super::codex_pricing::bundled_long_context_threshold(&lookup_model)
        {
            pricing.threshold_tokens = Some(threshold);
        }
        pricing
    }

    /// Calculate cost from a previously resolved Claude pricing source.
    pub(crate) fn claude_cost_usd_from_resolution(
        resolution: ClaudePricingResolution,
        input_tokens: i32,
        cache_read_input_tokens: i32,
        cache_creation_input_tokens: i32,
        output_tokens: i32,
    ) -> f64 {
        match resolution {
            ClaudePricingResolution::BuiltIn(pricing) => {
                fn tiered(
                    tokens: i32,
                    base: f64,
                    above: Option<f64>,
                    threshold: Option<i32>,
                ) -> f64 {
                    let tokens = tokens.max(0);
                    match (threshold, above) {
                        (Some(thresh), Some(above_rate)) => {
                            let below = tokens.min(thresh);
                            let over = (tokens - thresh).max(0);
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
            ClaudePricingResolution::ModelsDev(pricing) => {
                claude_routed_pricing::cost_usd_from_pricing(
                    pricing,
                    input_tokens,
                    cache_read_input_tokens,
                    cache_creation_input_tokens,
                    output_tokens,
                )
            }
        }
    }

    pub(crate) fn claude_input_cost_per_token_from_resolution(
        resolution: ClaudePricingResolution,
    ) -> f64 {
        match resolution {
            ClaudePricingResolution::BuiltIn(pricing) => pricing.input_cost_per_token,
            ClaudePricingResolution::ModelsDev(pricing) => pricing.input_cost_per_token,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog_pricing(threshold_tokens: Option<u64>) -> models_dev_pricing::DynamicModelPricing {
        models_dev_pricing::DynamicModelPricing {
            input_cost_per_token: 2e-6,
            output_cost_per_token: 4e-6,
            cache_read_input_cost_per_token: Some(0.25e-6),
            cache_write_input_cost_per_token: Some(3e-6),
            threshold_tokens,
            input_cost_per_token_above_threshold: Some(7e-6),
            output_cost_per_token_above_threshold: Some(11e-6),
            cache_read_input_cost_per_token_above_threshold: Some(0.5e-6),
            cache_write_input_cost_per_token_above_threshold: Some(9e-6),
        }
    }

    #[test]
    fn openai_long_context_uses_the_bundled_codex_boundary_and_catalog_rates() {
        let pricing = CostUsagePricing::apply_bundled_openai_threshold(
            "gpt-5.6-sol",
            "gpt-5.6-sol",
            catalog_pricing(Some(200_000)),
        );

        assert_eq!(pricing.threshold_tokens, Some(272_000));
        assert_eq!(pricing.input_cost_per_token, 2e-6);
        assert_eq!(pricing.output_cost_per_token_above_threshold, Some(11e-6));
    }

    #[test]
    fn aliases_and_explicit_openai_routes_share_the_bundled_boundary() {
        for (model, normalized) in [("gpt-5.6", "gpt-5.6-sol"), ("openai/gpt-5.6", "gpt-5.6")] {
            let pricing = CostUsagePricing::apply_bundled_openai_threshold(
                model,
                normalized,
                catalog_pricing(Some(200_000)),
            );
            assert_eq!(pricing.threshold_tokens, Some(272_000), "{model}");
        }
    }

    #[test]
    fn non_openai_and_unknown_openai_models_keep_catalog_boundaries() {
        let anthropic = CostUsagePricing::apply_bundled_openai_threshold(
            "anthropic/threshold-fixture",
            "anthropic/threshold-fixture",
            catalog_pricing(Some(200_000)),
        );
        let unknown_openai = CostUsagePricing::apply_bundled_openai_threshold(
            "openai/threshold-fixture",
            "openai/threshold-fixture",
            catalog_pricing(Some(200_000)),
        );

        assert_eq!(anthropic.threshold_tokens, Some(200_000));
        assert_eq!(unknown_openai.threshold_tokens, Some(200_000));
    }
}
