//! Claude "today" totals and the day-key format the cost buckets share.

use chrono::NaiveDate;
use std::collections::HashMap;

/// Local-usage totals for the calendar day (in the cost bucket zone) a scan
/// bucketed against, read from the same buckets as the daily history so the
/// scan's clock decides which day is "today".
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TodayUsage {
    /// `Some(0.0)` is a known zero; `None` means the cost is unknown (an
    /// unpriced record, or a scan that could not establish coverage).
    pub cost_usd: Option<f64>,
    /// Tokens counted for the day (input + output + cache read + cache write).
    pub tokens: u64,
}

impl TodayUsage {
    /// Read `day` out of the scan's daily buckets.
    pub(super) fn from_buckets(
        day: NaiveDate,
        daily_cost: &HashMap<String, Option<f64>>,
        daily_tokens: &HashMap<String, u64>,
    ) -> Self {
        let key = day_key(day);
        Self {
            cost_usd: daily_cost.get(&key).copied().flatten(),
            tokens: daily_tokens.get(&key).copied().unwrap_or(0),
        }
    }
}

/// The `YYYY-MM-DD` key the daily cost and token buckets are filed under.
pub(crate) fn day_key(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}
