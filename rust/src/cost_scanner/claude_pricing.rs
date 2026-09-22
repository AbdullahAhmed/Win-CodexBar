use crate::core::{ClaudePricingResolution, CostUsagePricing, ModelsDevPricingSnapshot};
use std::collections::HashMap;

pub(super) const FALLBACK_CLAUDE_MODEL: &str = "claude-sonnet-4-6";

#[cfg(test)]
pub(super) struct ClaudePricing;

#[cfg(test)]
impl ClaudePricing {
    pub(super) fn cost_usd_with_cache_ttl(
        model: &str,
        input: u64,
        cache_create: u64,
        cache_create_1h: u64,
        cache_read: u64,
        output: u64,
    ) -> f64 {
        let cache_create_1h = cache_create_1h.min(cache_create);
        let cache_create_5m = cache_create.saturating_sub(cache_create_1h);

        // Standard buckets (input, cache-read, 5-minute cache-write, output),
        // including any long-context tiering, come from the canonical table.
        // Unknown/retired models fall back to Sonnet pricing.
        #[allow(
            clippy::cast_possible_truncation,
            reason = "clamped to i32::MAX before casting"
        )]
        let clamp = |v: u64| v.min(i32::MAX as u64) as i32;
        let base = CostUsagePricing::claude_cost_usd(
            model,
            clamp(input),
            clamp(cache_read),
            clamp(cache_create_5m),
            clamp(output),
        )
        .or_else(|| {
            CostUsagePricing::claude_cost_usd(
                FALLBACK_CLAUDE_MODEL,
                clamp(input),
                clamp(cache_read),
                clamp(cache_create_5m),
                clamp(output),
            )
        })
        .unwrap_or(0.0);

        // Scanner-specific: one-hour cache writes bill at 2x the input rate.
        let input_rate = CostUsagePricing::claude_input_cost_per_token(model)
            .or_else(|| CostUsagePricing::claude_input_cost_per_token(FALLBACK_CLAUDE_MODEL))
            .unwrap_or(0.0);

        base + (cache_create_1h as f64) * input_rate * 2.0
    }
}

/// Per-scan Claude pricing memo.
///
/// Claude logs commonly repeat the same model across many files and records. Keep model
/// normalization and positive/negative models.dev resolution scan-local while retaining the
/// canonical pricing arithmetic and provider-routing rules.
#[derive(Default)]
pub(super) struct ClaudeScanPricingResolver {
    snapshot: Option<ModelsDevPricingSnapshot>,
    pub(super) normalized_models: HashMap<String, String>,
    pub(super) resolutions: HashMap<String, Option<ClaudePricingResolution>>,
    #[cfg(test)]
    pub(super) normalization_cache_misses: usize,
    #[cfg(test)]
    pub(super) resolution_cache_misses: usize,
}

impl ClaudeScanPricingResolver {
    pub(super) const MEMO_ENTRY_LIMIT: usize = 1024;

    #[cfg(test)]
    pub(super) fn with_snapshot(snapshot: ModelsDevPricingSnapshot) -> Self {
        Self {
            snapshot: Some(snapshot),
            ..Self::default()
        }
    }

    pub(super) fn normalize(&mut self, model: &str) -> String {
        if let Some(normalized) = self.normalized_models.get(model) {
            return normalized.clone();
        }
        #[cfg(test)]
        {
            self.normalization_cache_misses += 1;
        }
        let normalized = CostUsagePricing::normalize_claude_model(model);
        if self.normalized_models.len() < Self::MEMO_ENTRY_LIMIT {
            self.normalized_models
                .insert(model.to_string(), normalized.clone());
        }
        normalized
    }

    fn resolve(&mut self, model: &str) -> Option<ClaudePricingResolution> {
        if let Some(resolution) = self.resolutions.get(model) {
            return *resolution;
        }
        #[cfg(test)]
        {
            self.resolution_cache_misses += 1;
        }

        let normalized = self.normalize(model);
        let needs_catalog = self.snapshot.is_none();
        let mut resolution =
            CostUsagePricing::resolve_claude_pricing(model, &normalized, self.snapshot.as_ref());
        if resolution.is_none() && needs_catalog {
            let snapshot = self
                .snapshot
                .get_or_insert_with(crate::core::pricing_snapshot);
            resolution =
                CostUsagePricing::resolve_claude_pricing(model, &normalized, Some(snapshot));
        }
        if self.resolutions.len() < Self::MEMO_ENTRY_LIMIT {
            self.resolutions.insert(model.to_string(), resolution);
        }
        resolution
    }

    pub(super) fn is_known(&mut self, model: &str) -> bool {
        self.resolve(model).is_some()
    }

    pub(super) fn cost_usd_with_cache_ttl(
        &mut self,
        model: &str,
        input: u64,
        cache_create: u64,
        cache_create_1h: u64,
        cache_read: u64,
        output: u64,
    ) -> f64 {
        let cache_create_1h = cache_create_1h.min(cache_create);
        let cache_create_5m = cache_create.saturating_sub(cache_create_1h);

        let resolved = self.resolve(model);
        let billable = resolved.or_else(|| self.resolve(FALLBACK_CLAUDE_MODEL));
        let base = billable
            .map(|pricing| claude_cost_usd_u64(pricing, input, cache_read, cache_create_5m, output))
            .unwrap_or(0.0);
        let input_rate = billable
            .map(CostUsagePricing::claude_input_cost_per_token_from_resolution)
            .unwrap_or(0.0);

        base + (cache_create_1h as f64) * input_rate * 2.0
    }
}

/// Price transcript counters without narrowing them to `i32`. Local history is
/// untrusted input and can contain values far above the API's ordinary range;
/// narrowing those values silently understates spend before aggregation gets a
/// chance to mark non-finite results unavailable.
fn claude_cost_usd_u64(
    resolution: ClaudePricingResolution,
    input: u64,
    cache_read: u64,
    cache_write: u64,
    output: u64,
) -> f64 {
    match resolution {
        ClaudePricingResolution::BuiltIn(pricing) => {
            let tiered = |tokens: u64, base: f64, above: Option<f64>| {
                let Some(threshold) = pricing.threshold_tokens.map(|value| value.max(0) as u64)
                else {
                    return (tokens as f64) * base;
                };
                let Some(above) = above else {
                    return (tokens as f64) * base;
                };
                let below = tokens.min(threshold);
                let over = tokens.saturating_sub(threshold);
                (below as f64) * base + (over as f64) * above
            };

            tiered(
                input,
                pricing.input_cost_per_token,
                pricing.input_cost_per_token_above_threshold,
            ) + tiered(
                cache_read,
                pricing.cache_read_input_cost_per_token,
                pricing.cache_read_input_cost_per_token_above_threshold,
            ) + tiered(
                cache_write,
                pricing.cache_creation_input_cost_per_token,
                pricing.cache_creation_input_cost_per_token_above_threshold,
            ) + tiered(
                output,
                pricing.output_cost_per_token,
                pricing.output_cost_per_token_above_threshold,
            )
        }
        ClaudePricingResolution::ModelsDev {
            pricing,
            threshold_tokens,
        } => {
            let use_tier = threshold_tokens.is_some_and(|threshold| {
                input
                    .checked_add(cache_read)
                    .and_then(|value| value.checked_add(cache_write))
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

            (input as f64) * input_rate
                + (cache_read as f64) * cache_read_rate
                + (cache_write as f64) * cache_write_rate
                + (output as f64) * output_rate
        }
    }
}
