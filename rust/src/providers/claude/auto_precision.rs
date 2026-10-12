//! Keeps Claude Auto's usage sources on the same percentage precision.
//!
//! Scope: Auto mode only. Claude Code's `/usage` screen prints
//! `Math.floor(utilization)` (evidence: issue #776, Claude Code 2.1.284
//! bundle), while the Admin, Web and OAuth sources report fractional
//! utilization (for example 7.6). Auto alternates between them when a source
//! is rate limited, and the UI rounds to the nearest whole number, so the same
//! utilization showed as 8 from OAuth and 7 from the CLI (issue #776).
//! Flooring every Auto result gives all sources the CLI's precision.
//!
//! OAuth-only and Web-only source modes keep their fractional values on
//! purpose: they never alternate with the CLI, so nothing flips.
//!
//! Consequence by design: forecasts, burndown and notification thresholds see
//! the floored value in Auto, matching what Claude Code itself displays.

use crate::core::{ProviderFetchResult, RateWindow};
use std::sync::atomic::{AtomicU64, Ordering};

/// Unix seconds of the last live non-CLI Auto answer in this process.
static LAST_LIVE_SUCCESS_SECS: AtomicU64 = AtomicU64::new(0);

/// Final step of every Auto fetch: floor all percents, and when the answer is
/// live (not a CLI reading) retire older CLI readings.
///
/// A live Admin/Web/OAuth answer supersedes any older CLI reading. Dropping the
/// in-process CLI result cache and marking the time makes
/// `probe_screen_is_superseded` reject an older cross-process probe screen
/// (`.codexbar-usage-cache.json`, 45 s TTL), so a later rate limit cannot fall
/// back to a staler (lower) value. Marking is per process; another process
/// that reads the file only risks a screen at most 45 s old, floored anyway.
pub(super) fn finish_auto_result(mut result: ProviderFetchResult) -> ProviderFetchResult {
    floor_to_cli_precision(&mut result);
    if !result.source_label.starts_with("cli") {
        super::clear_cli_result_cache();
        LAST_LIVE_SUCCESS_SECS.store(super::unix_now_secs(), Ordering::Relaxed);
    }
    result
}

/// True when a probe screen captured at `captured_at_unix` predates the last
/// live non-CLI answer.
pub(super) fn probe_screen_is_superseded(captured_at_unix: u64) -> bool {
    captured_at_unix < LAST_LIVE_SUCCESS_SECS.load(Ordering::Relaxed)
}

/// Floor every quota window in a result to a whole percent, as the CLI does.
pub(super) fn floor_to_cli_precision(result: &mut ProviderFetchResult) {
    let usage = &mut result.usage;
    floor_window(&mut usage.primary);
    for window in [
        usage.secondary.as_mut(),
        usage.model_specific.as_mut(),
        usage.tertiary.as_mut(),
    ]
    .into_iter()
    .flatten()
    {
        floor_window(window);
    }
    for named in &mut usage.extra_rate_windows {
        floor_window(&mut named.window);
    }
}

fn floor_window(window: &mut RateWindow) {
    if window.used_percent.is_finite() {
        window.used_percent = window.used_percent.floor();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::UsageSnapshot;

    fn result(session: f64, weekly: f64) -> ProviderFetchResult {
        let mut usage = UsageSnapshot::new(RateWindow::new(session));
        usage.secondary = Some(RateWindow::new(weekly));
        ProviderFetchResult::new(usage, "oauth")
    }

    fn shown(result: &ProviderFetchResult) -> (f64, f64) {
        // The UI and tray display the rounded percent. There is no shared Rust
        // formatter to call; see tray/render.rs:181 (`.round() as u32`).
        let weekly = result.usage.secondary.as_ref().expect("weekly");
        (
            result.usage.primary.used_percent.round(),
            weekly.used_percent.round(),
        )
    }

    #[test]
    fn oauth_and_cli_agree_after_flooring() {
        // The same underlying utilization, read from OAuth (fractional) and
        // from the CLI (already floored by Claude Code).
        let cli = result(7.0, 9.0);
        let mut oauth = result(7.6, 9.5);
        assert_eq!(shown(&oauth), (8.0, 10.0), "unfixed OAuth flips to 8/10");

        floor_to_cli_precision(&mut oauth);
        let polls = [&oauth, &cli, &oauth, &cli];
        assert!(polls.iter().all(|poll| shown(poll) == (7.0, 9.0)));
    }

    #[test]
    fn flooring_applies_to_any_source_label() {
        // `finish_auto_result` also touches process globals, so only the pure
        // flooring step is exercised here.
        for label in ["admin", "web", "oauth"] {
            let mut r = result(7.6, 9.5);
            r.source_label = label.to_string();
            floor_to_cli_precision(&mut r);
            assert_eq!(shown(&r), (7.0, 9.0), "{label}");
        }
    }

    #[test]
    fn probe_screen_older_than_live_success_is_superseded() {
        LAST_LIVE_SUCCESS_SECS.store(1_000, Ordering::Relaxed);
        assert!(probe_screen_is_superseded(999));
        assert!(!probe_screen_is_superseded(1_000));
        LAST_LIVE_SUCCESS_SECS.store(0, Ordering::Relaxed);
    }

    #[test]
    fn real_increases_still_show() {
        let mut later = result(8.2, 10.1);
        floor_to_cli_precision(&mut later);
        assert_eq!(shown(&later), (8.0, 10.0));
    }

    #[test]
    fn whole_and_exhausted_values_are_unchanged() {
        let mut full = result(100.0, 0.0);
        floor_to_cli_precision(&mut full);
        assert_eq!(shown(&full), (100.0, 0.0));
    }
}
