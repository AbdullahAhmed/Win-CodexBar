//! Resolves a scanner's [`CostReportingPeriod`] into concrete scan windows.

use chrono::{DateTime, Duration, NaiveDate, Utc};

use super::CostScanner;
use crate::cost_reporting_period::{CostReportingPeriod, clamp_window_days, cost_bucket_zone};

/// Inclusive local-day window plus the instant transcript scanners cut off at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ScanWindow {
    /// First day of the window (also reported as `period_start`).
    pub start: NaiveDate,
    /// Last day of the window (today).
    pub end: NaiveDate,
    /// Records before this instant are outside the window.
    pub cutoff: DateTime<Utc>,
    /// Resolved day count for scanners that take a day count.
    pub days: u32,
}

impl CostScanner {
    /// The window in the pinned bucket zone, resolved from the calendar.
    ///
    /// `earliest` is the first day with source data; it only matters for
    /// [`CostReportingPeriod::AllAvailable`]. Rolling(N) covers N days
    /// including today, and the cutoff is the start of the first day.
    pub(super) fn calendar_window(
        &self,
        now: DateTime<Utc>,
        earliest: Option<NaiveDate>,
    ) -> ScanWindow {
        let tz = cost_bucket_zone();
        let bounds = self.period.bounds(now, tz, earliest);
        ScanWindow {
            start: bounds.start,
            end: bounds.end,
            cutoff: tz.start_of_day_utc(bounds.start),
            days: clamp_window_days(bounds.days()),
        }
    }

    /// The window the Claude scans use, anchored on the bucket zone's today.
    ///
    /// Rolling(1) is the zone's calendar day (not a trailing 24 hours), so the
    /// one-day scan agrees with today's bucket in the chart snapshot. Other
    /// periods keep [`Self::transcript_window`].
    ///
    /// TODO: the simpler end state is `calendar_window` for every period, as
    /// Codex does (codex/scan.rs); deferred because it moves 30-day boundaries.
    pub(super) fn claude_window(&self, now: DateTime<Utc>) -> ScanWindow {
        if self.period == CostReportingPeriod::Rolling(1) {
            return self.calendar_window(now, None);
        }
        self.transcript_window(now, cost_bucket_zone().date(now))
    }

    /// The window used by the Claude and Pi transcript scanners.
    ///
    /// A rolling window keeps its historical shape (a `now - N * 24h` cutoff
    /// and `today - N` as the reported start, with `today` supplied by the
    /// caller); month to date and all available history use exact calendar
    /// midnights so no record from the previous month leaks in.
    pub(super) fn transcript_window(&self, now: DateTime<Utc>, today: NaiveDate) -> ScanWindow {
        match self.period {
            CostReportingPeriod::Rolling(days) => ScanWindow {
                start: today - Duration::days(i64::from(days)),
                end: today,
                cutoff: now - Duration::days(i64::from(days)),
                days,
            },
            CostReportingPeriod::MonthToDate | CostReportingPeriod::AllAvailable => {
                self.calendar_window(now, None)
            }
        }
    }
}
