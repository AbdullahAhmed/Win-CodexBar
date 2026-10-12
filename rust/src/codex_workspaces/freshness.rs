//! When a cached workspaces snapshot may be reused, and one scan per process.

use std::sync::{Mutex, MutexGuard, PoisonError};

use chrono::{DateTime, Duration, Utc};

use super::indexer::{CodexWorkspacesIndex, IndexError};
use super::types::{CodexLocalProjectUsageSnapshot, Progress};
use crate::cost_reporting_period::CostTimeZone;

/// Bound reuse so background reads discover new usage without a forced refresh.
pub(super) const SNAPSHOT_CACHE_TTL: Duration = Duration::minutes(5);

/// A cache stamped slightly ahead of `now` (another process published while we
/// were reading) is still fresh; anything further ahead is a clock rollback.
pub(super) const CLOCK_SKEW_TOLERANCE: Duration = Duration::seconds(60);

/// One scan-and-publish at a time per process, so callers that hit an expired
/// cache together do not each rescan.
static SCAN_LOCK: Mutex<()> = Mutex::new(());

pub(super) fn scan_guard() -> MutexGuard<'static, ()> {
    SCAN_LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

/// True when a snapshot published at `cached_updated_at` can still serve `now`:
/// under `SNAPSHOT_CACHE_TTL` old, not beyond the skew tolerance in the future,
/// and from the same reporting date.
pub(super) fn is_reusable(
    cached_updated_at: DateTime<Utc>,
    now: DateTime<Utc>,
    zone: &CostTimeZone,
) -> bool {
    let age = now - cached_updated_at;
    age < SNAPSHOT_CACHE_TTL
        && age >= -CLOCK_SKEW_TOLERANCE
        && zone.date(cached_updated_at) == zone.date(now)
}

impl CodexWorkspacesIndex {
    pub fn load_snapshot<F>(
        &self,
        force_refresh: bool,
        progress: F,
    ) -> Result<CodexLocalProjectUsageSnapshot, IndexError>
    where
        F: FnMut(Progress),
    {
        // Take the lock before reading the clock: a waiter's `now` must not
        // predate the snapshot the previous holder published.
        let _guard = scan_guard();
        self.load_snapshot_at(force_refresh, progress, Utc::now())
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn at(h: u32, m: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 8, h, m, s).unwrap()
    }

    const UTC: CostTimeZone = CostTimeZone::UTC;

    #[test]
    fn reuses_just_under_the_ttl_and_rescans_at_exactly_five_minutes() {
        let now = at(12, 5, 0);
        assert!(is_reusable(at(12, 0, 1), now, &UTC), "4:59 old");
        assert!(!is_reusable(at(12, 0, 0), now, &UTC), "exactly 5:00 old");
        assert!(!is_reusable(at(11, 59, 0), now, &UTC), "6:00 old");
    }

    #[test]
    fn reuses_a_snapshot_stamped_at_now() {
        assert!(is_reusable(at(12, 0, 0), at(12, 0, 0), &UTC));
    }

    #[test]
    fn tolerates_small_future_skew_but_not_rollback() {
        let now = at(12, 0, 0);
        assert!(is_reusable(at(12, 0, 30), now, &UTC), "30 s ahead");
        assert!(is_reusable(at(12, 1, 0), now, &UTC), "exactly 60 s ahead");
        assert!(!is_reusable(at(12, 1, 1), now, &UTC), "61 s ahead");
        assert!(!is_reusable(at(12, 10, 0), now, &UTC), "10 min ahead");
    }

    #[test]
    fn rescans_across_the_reporting_date_even_when_young() {
        // A fresh stamp a few seconds before midnight, read just after it.
        let before = at(23, 59, 59);
        let after = before + Duration::seconds(2);
        assert!(!is_reusable(before, after, &UTC));
    }

    #[test]
    fn skew_across_midnight_is_a_different_date() {
        let now = at(23, 59, 30);
        assert!(!is_reusable(now + Duration::seconds(45), now, &UTC));
    }
}
