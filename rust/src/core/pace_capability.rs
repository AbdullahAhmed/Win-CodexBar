//! Provider reset-window pace (upstream `ProviderPaceCapability`, v0.70.0
//! `Sources/CodexBarCore/Providers/ProviderDescriptor.swift`).
//!
//! Some providers report a quota that refills on a billing date (Copilot,
//! Cursor, Notion, ...) rather than on a fixed session or weekly cadence.
//! Upstream opts those providers in to "reset-window pace": the card scores
//! the window as weekly-style pace against its real length. A 30-day
//! sentinel (or a missing length, for Copilot) is replaced by the calendar
//! month that ends at the reset, so a February cycle is scored as 28 days.
//!
//! Each provider declares its rule in [`super::ProviderId::pace_capability`];
//! rules that need provider knowledge are function pointers owned by that
//! provider's module.

use chrono::{DateTime, Utc};

use super::{RateWindow, UsagePace};

/// Upstream `ProviderPaceCapability.monthlyWindowSentinelMinutes` (30 days).
pub const MONTHLY_WINDOW_SENTINEL_MINUTES: u32 = 30 * 24 * 60;

/// Weekly fallback length used when a reset window has no duration
/// (upstream `UsagePace.weekly(..., defaultWindowMinutes: 10080, ...)`).
const RESET_WINDOW_DEFAULT_MINUTES: u32 = 7 * 24 * 60;

/// Provider-owned predicate over a window at an instant.
pub type PaceWindowPredicate = fn(&RateWindow, DateTime<Utc>) -> bool;
/// Provider-owned predicate over a window.
pub type PaceDurationPredicate = fn(&RateWindow) -> bool;

/// Which windows get reset-window pace (upstream `ProviderPaceWindowRule`).
#[derive(Debug, Clone, Copy)]
pub enum PaceWindowRule {
    Unsupported,
    ResetDatePresent,
    WindowDurationPresent,
    WindowDuration(u32),
    Custom(PaceWindowPredicate),
}

impl PaceWindowRule {
    pub fn matches(&self, window: &RateWindow, now: DateTime<Utc>) -> bool {
        match self {
            Self::Unsupported => false,
            Self::ResetDatePresent => window.resets_at.is_some(),
            Self::WindowDurationPresent => window.window_minutes.is_some(),
            Self::WindowDuration(minutes) => window.window_minutes == Some(*minutes),
            Self::Custom(predicate) => predicate(window, now),
        }
    }
}

/// Which windows take their length from the calendar month ending at the
/// reset (upstream `ProviderPaceDurationRule`).
#[derive(Debug, Clone, Copy)]
pub enum PaceDurationRule {
    Unsupported,
    WindowDurationMissing,
    WindowDuration(u32),
    Custom(PaceDurationPredicate),
}

impl PaceDurationRule {
    pub fn matches(&self, window: &RateWindow) -> bool {
        match self {
            Self::Unsupported => false,
            Self::WindowDurationMissing => window.window_minutes.is_none(),
            Self::WindowDuration(minutes) => window.window_minutes == Some(*minutes),
            Self::Custom(predicate) => predicate(window),
        }
    }
}

/// Reset-window part of upstream `ProviderPaceCapability`. The standard
/// session/weekly lanes stay with the existing per-lane pace on Windows.
#[derive(Debug, Clone, Copy)]
pub struct ProviderPaceCapability {
    pub reset_window_pace: PaceWindowRule,
    pub inferred_monthly_duration: PaceDurationRule,
}

impl ProviderPaceCapability {
    pub const UNSUPPORTED: Self = Self {
        reset_window_pace: PaceWindowRule::Unsupported,
        inferred_monthly_duration: PaceDurationRule::Unsupported,
    };

    /// Upstream `.calendarMonthResetWindow`.
    pub const CALENDAR_MONTH_RESET_WINDOW: Self = Self {
        reset_window_pace: PaceWindowRule::WindowDuration(MONTHLY_WINDOW_SENTINEL_MINUTES),
        inferred_monthly_duration: PaceDurationRule::WindowDuration(
            MONTHLY_WINDOW_SENTINEL_MINUTES,
        ),
    };

    pub const fn reset_window(rule: PaceWindowRule) -> Self {
        Self {
            reset_window_pace: rule,
            inferred_monthly_duration: PaceDurationRule::Unsupported,
        }
    }

    pub fn supports_reset_window_pace(&self, window: &RateWindow, now: DateTime<Utc>) -> bool {
        self.reset_window_pace.matches(window, now)
    }

    pub fn uses_inferred_monthly_duration(&self, window: &RateWindow) -> bool {
        self.inferred_monthly_duration.matches(window)
    }

    /// Upstream `resolvedResetWindowForPace`: a window matching the inferred
    /// monthly rule takes the length of the UTC calendar month ending at its
    /// reset (reset minus one month, day clamped). Anything else is unchanged.
    pub fn resolved_reset_window_for_pace(&self, window: &RateWindow) -> RateWindow {
        let mut resolved = window.clone();
        if self.uses_inferred_monthly_duration(window)
            && let Some(minutes) = window
                .resets_at
                .and_then(RateWindow::calendar_month_window_minutes)
        {
            resolved.window_minutes = Some(minutes);
        }
        resolved
    }

    /// Weekly-style pace for a reset window, or `None` when the provider does
    /// not opt this window in. Mirrors upstream `resetWindowPaceDetail`
    /// (MenuCardView+ModelHelpers.swift) and `CLIRenderer.computePace`:
    /// the window must have quota left, is resolved first, and is scored with
    /// the weekly fallback and the configured work days.
    pub fn reset_window_pace(
        &self,
        window: &RateWindow,
        now: DateTime<Utc>,
        work_days: Option<u8>,
    ) -> Option<UsagePace> {
        if !self.supports_reset_window_pace(window, now) || window.remaining_percent() <= 0.0 {
            return None;
        }
        let pace_window = self.resolved_reset_window_for_pace(window);
        UsagePace::weekly(
            &pace_window,
            Some(now),
            RESET_WINDOW_DEFAULT_MINUTES,
            work_days,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ProviderId;
    use chrono::TimeZone;

    fn at(y: i32, m: u32, d: u32, h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap()
    }

    fn window(used: f64, minutes: Option<u32>, resets_at: DateTime<Utc>) -> RateWindow {
        RateWindow::with_details(used, minutes, Some(resets_at), None)
    }

    fn resolved_minutes(minutes: Option<u32>, resets_at: DateTime<Utc>) -> Option<u32> {
        ProviderPaceCapability::CALENDAR_MONTH_RESET_WINDOW
            .resolved_reset_window_for_pace(&window(10.0, minutes, resets_at))
            .window_minutes
    }

    #[test]
    fn sentinel_resolves_to_the_calendar_month_ending_at_the_reset() {
        // Feb 2026 has 28 days.
        assert_eq!(
            resolved_minutes(Some(43_200), at(2026, 3, 1, 0)),
            Some(40_320)
        );
        // Mar 31 minus one month clamps to Feb 28: 31 days.
        assert_eq!(
            resolved_minutes(Some(43_200), at(2026, 3, 31, 0)),
            Some(44_640)
        );
        // Leap February 2028: 29 days.
        assert_eq!(
            resolved_minutes(Some(43_200), at(2028, 3, 1, 0)),
            Some(41_760)
        );
        // Jan 15 → Dec 15: 31 days.
        assert_eq!(
            resolved_minutes(Some(43_200), at(2026, 1, 15, 9)),
            Some(44_640)
        );
    }

    #[test]
    fn resolution_leaves_non_matching_windows_unchanged() {
        assert_eq!(
            resolved_minutes(Some(10_080), at(2026, 3, 1, 0)),
            Some(10_080)
        );
        assert_eq!(resolved_minutes(None, at(2026, 3, 1, 0)), None);
        let no_reset = RateWindow::with_details(10.0, Some(43_200), None, None);
        assert_eq!(
            ProviderPaceCapability::CALENDAR_MONTH_RESET_WINDOW
                .resolved_reset_window_for_pace(&no_reset)
                .window_minutes,
            Some(43_200)
        );
    }

    #[test]
    fn copilot_resolves_a_window_without_a_duration() {
        let copilot = ProviderId::Copilot.pace_capability();
        let w = window(60.0, None, at(2026, 3, 1, 0));
        assert!(copilot.supports_reset_window_pace(&w, at(2026, 2, 15, 0)));
        assert_eq!(
            copilot.resolved_reset_window_for_pace(&w).window_minutes,
            Some(40_320)
        );
        // An explicit duration is kept.
        let explicit = window(60.0, Some(43_200), at(2026, 3, 1, 0));
        assert_eq!(
            copilot
                .resolved_reset_window_for_pace(&explicit)
                .window_minutes,
            Some(43_200)
        );
    }

    #[test]
    fn copilot_pace_scores_the_february_cycle_as_28_days() {
        // Feb 15 00:00 of a cycle ending Mar 1: 14 of 28 days elapsed.
        let pace = ProviderId::Copilot
            .pace_capability()
            .reset_window_pace(
                &window(60.0, None, at(2026, 3, 1, 0)),
                at(2026, 2, 15, 0),
                None,
            )
            .expect("copilot reset-window pace");
        assert!((pace.expected_used_percent - 50.0).abs() < 1e-9);
        assert!((pace.delta_percent - 10.0).abs() < 1e-9);
        // 40% left at 60% per 14 days: 9 days 8 hours.
        let eta = pace.eta_seconds.expect("eta");
        assert!((eta - 806_400.0).abs() < 1e-6, "eta {eta}");
        assert!(!pace.will_last_to_reset);
    }

    #[test]
    fn a_31_day_cycle_keeps_pace_on_its_first_day() {
        // Reset Mar 31, now Feb 28 12:00: 30.5 days left, longer than the
        // 30-day sentinel, so the raw window has no pace at all.
        let now = at(2026, 2, 28, 12);
        let raw = window(1.0, Some(43_200), at(2026, 3, 31, 0));
        assert!(UsagePace::weekly(&raw, Some(now), 10_080, None).is_none());
        let pace = ProviderId::Notion
            .pace_capability()
            .reset_window_pace(&raw, now, None)
            .expect("resolved 31-day pace");
        // 0.5 of 31 days elapsed.
        assert!(
            (pace.expected_used_percent - 50.0 / 31.0).abs() < 1e-9,
            "expected {}",
            pace.expected_used_percent
        );
    }

    #[test]
    fn notion_sentinel_pace_uses_the_real_month_not_30_days() {
        let now = at(2026, 2, 15, 0);
        let raw = window(50.0, Some(43_200), at(2026, 3, 1, 0));
        let flat = UsagePace::weekly(&raw, Some(now), 10_080, None).expect("flat");
        // A flat 30-day window: 16 of 30 days elapsed.
        assert!((flat.expected_used_percent - 53.333_333_333).abs() < 1e-6);
        let pace = ProviderId::Notion
            .pace_capability()
            .reset_window_pace(&raw, now, None)
            .expect("resolved");
        assert!((pace.expected_used_percent - 50.0).abs() < 1e-9);
    }

    #[test]
    fn exhausted_windows_and_unsupported_providers_get_no_reset_window_pace() {
        let now = at(2026, 2, 15, 0);
        let copilot = ProviderId::Copilot.pace_capability();
        assert!(
            copilot
                .reset_window_pace(&window(100.0, None, at(2026, 3, 1, 0)), now, None)
                .is_none()
        );
        let no_reset = RateWindow::with_details(10.0, None, None, None);
        assert!(!copilot.supports_reset_window_pace(&no_reset, now));
        for provider in [ProviderId::Codex, ProviderId::Claude, ProviderId::Gemini] {
            assert!(
                provider
                    .pace_capability()
                    .reset_window_pace(&window(10.0, Some(43_200), at(2026, 3, 1, 0)), now, None)
                    .is_none(),
                "{provider:?}"
            );
        }
    }

    #[test]
    fn cursor_needs_a_window_duration() {
        let cursor = ProviderId::Cursor.pace_capability();
        let now = at(2026, 2, 15, 0);
        assert!(!cursor.supports_reset_window_pace(&window(10.0, None, at(2026, 3, 1, 0)), now));
        let w = window(10.0, Some(40_320), at(2026, 3, 1, 0));
        assert!(cursor.supports_reset_window_pace(&w, now));
        // Cursor's duration comes from the billing cycle and is never inferred.
        assert_eq!(
            cursor
                .resolved_reset_window_for_pace(&window(10.0, Some(43_200), at(2026, 3, 1, 0)))
                .window_minutes,
            Some(43_200)
        );
    }

    #[test]
    fn kimi_opts_in_only_its_weekly_window() {
        let kimi = ProviderId::Kimi.pace_capability();
        let now = at(2026, 2, 26, 0);
        assert!(
            kimi.supports_reset_window_pace(&window(10.0, Some(10_080), at(2026, 3, 1, 0)), now)
        );
        assert!(!kimi.supports_reset_window_pace(&window(10.0, Some(300), at(2026, 3, 1, 0)), now));
        assert!(
            !kimi.supports_reset_window_pace(&window(10.0, Some(43_200), at(2026, 3, 1, 0)), now)
        );
    }

    #[test]
    fn calendar_month_providers_match_upstream_opt_ins() {
        let now = at(2026, 2, 15, 0);
        let sentinel = window(10.0, Some(43_200), at(2026, 3, 1, 0));
        let weekly = window(10.0, Some(10_080), at(2026, 2, 20, 0));
        for provider in [
            ProviderId::Alibaba,
            ProviderId::AlibabaTokenPlan,
            ProviderId::CommandCode,
            ProviderId::Doubao,
            ProviderId::MiMo,
            ProviderId::StepFun,
            ProviderId::Notion,
            ProviderId::Ollama,
            ProviderId::OpenCodeGo,
        ] {
            let capability = provider.pace_capability();
            assert!(
                capability.supports_reset_window_pace(&sentinel, now),
                "{provider:?}"
            );
            assert_eq!(
                capability
                    .resolved_reset_window_for_pace(&sentinel)
                    .window_minutes,
                Some(40_320),
                "{provider:?}"
            );
            assert!(
                !capability.supports_reset_window_pace(&weekly, now),
                "{provider:?}"
            );
        }
    }

    #[test]
    fn zai_opts_in_only_the_monthly_mcp_window() {
        let zai = ProviderId::Zai.pace_capability();
        let now = at(2026, 2, 15, 0);
        let mcp = RateWindow::with_details(
            10.0,
            Some(43_200),
            Some(at(2026, 3, 1, 0)),
            Some("MCP".to_string()),
        );
        assert!(zai.supports_reset_window_pace(&mcp, now));
        assert_eq!(
            zai.resolved_reset_window_for_pace(&mcp).window_minutes,
            Some(40_320)
        );
        let tokens = window(10.0, Some(43_200), at(2026, 3, 1, 0));
        assert!(!zai.supports_reset_window_pace(&tokens, now));
        let mcp_without_duration =
            RateWindow::with_details(10.0, None, Some(at(2026, 3, 1, 0)), Some("MCP".to_string()));
        assert!(!zai.supports_reset_window_pace(&mcp_without_duration, now));
    }

    #[test]
    fn amp_opts_in_only_renewing_subscription_windows() {
        let amp = ProviderId::Amp.pace_capability();
        let now = at(2026, 2, 15, 0);
        let renewing = RateWindow::with_details(
            10.0,
            Some(40_320),
            Some(at(2026, 3, 1, 0)),
            Some("renews in 14 days".to_string()),
        );
        assert!(amp.supports_reset_window_pace(&renewing, now));
        let free = RateWindow::with_details(
            10.0,
            Some(1_440),
            Some(at(2026, 2, 16, 0)),
            Some("resets daily".to_string()),
        );
        assert!(!amp.supports_reset_window_pace(&free, now));
    }

    #[test]
    fn grok_opts_in_weekly_windows_inside_their_length() {
        let grok = ProviderId::Grok.pace_capability();
        let now = at(2026, 2, 15, 0);
        // Explicit weekly duration, reset 3 days away.
        assert!(
            grok.supports_reset_window_pace(&window(10.0, Some(10_080), at(2026, 2, 18, 0)), now)
        );
        // Monthly duration is labelled "Monthly", not "Weekly".
        assert!(
            !grok.supports_reset_window_pace(&window(10.0, Some(43_200), at(2026, 2, 18, 0)), now)
        );
        // No duration: the label comes from the distance to the reset.
        // 6 days away rounds to "Weekly" and fits the 7-day fallback.
        assert!(grok.supports_reset_window_pace(&window(10.0, None, at(2026, 2, 21, 0)), now));
        // 3 days away rounds to 3 days, which has no label.
        assert!(!grok.supports_reset_window_pace(&window(10.0, None, at(2026, 2, 18, 0)), now));
        // Reset in the past.
        assert!(
            !grok.supports_reset_window_pace(&window(10.0, Some(10_080), at(2026, 2, 14, 0)), now)
        );
        // Weekly label but the reset is further away than the window.
        assert!(
            !grok.supports_reset_window_pace(&window(10.0, Some(7_200), at(2026, 2, 22, 0)), now)
        );
    }
}
