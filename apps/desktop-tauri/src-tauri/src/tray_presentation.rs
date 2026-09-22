//! Pure tray presentation policy shared by the native tray surfaces.

use crate::commands::{ProviderUsageSnapshot, RateWindowSnapshot};
use codexbar::settings::{Language, MetricPreference, Settings, TrayIconMode};
use codexbar::tray::{
    render_bar_icon_rgba, render_percent_icon_rgba, render_stacked_bar_icon_rgba,
};

#[derive(Debug, Clone, Copy, PartialEq)]
enum TrayIconPlan {
    Bars {
        primary_percent: f64,
        secondary_percent: Option<f64>,
        has_error: bool,
    },
    Percent {
        percent: f64,
        has_error: bool,
    },
    Stacked {
        top_percent: f64,
        bottom_percent: f64,
        has_error: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayStatusKey {
    Summary,
    Provider,
}

#[derive(Debug, Clone, Copy)]
struct TrayStatusRow<'a> {
    key: TrayStatusKey,
    snapshot: &'a ProviderUsageSnapshot,
}

/// Fully resolved tray presentation, independent of Tauri and operating-system state.
///
/// The plan is the single policy boundary for provider ordering, mode-specific
/// selection, metric selection, status rows, and icon renderer choice.
pub(crate) struct TrayPresentationPlan<'a> {
    settings: &'a Settings,
    icon: TrayIconPlan,
    status_rows: Vec<TrayStatusRow<'a>>,
}

impl<'a> TrayPresentationPlan<'a> {
    pub(crate) fn resolve(settings: &'a Settings, snapshots: &'a [ProviderUsageSnapshot]) -> Self {
        let ordered = ordered_snapshot_refs(settings, snapshots);
        let healthy = ordered
            .into_iter()
            .filter(|snapshot| snapshot.error.is_none())
            .collect::<Vec<_>>();
        let has_error = healthy.is_empty() && !snapshots.is_empty();
        let prefer_highest =
            settings.menu_bar_shows_highest_usage || settings.menu_bar_display_mode == "minimal";
        let selected = pick_tray_provider(&healthy, prefer_highest);

        let (primary_percent, secondary_percent, status_rows) = match settings.tray_icon_mode {
            TrayIconMode::Stacked => {
                if let Some((top, bottom)) = pick_stacked_tray_providers(&healthy, settings) {
                    (
                        selected_tray_percents(top, settings).0,
                        Some(selected_tray_percents(bottom, settings).0),
                        vec![
                            TrayStatusRow {
                                key: TrayStatusKey::Provider,
                                snapshot: top,
                            },
                            TrayStatusRow {
                                key: TrayStatusKey::Provider,
                                snapshot: bottom,
                            },
                        ],
                    )
                } else {
                    let percents = selected
                        .map(|snapshot| selected_tray_percents(snapshot, settings))
                        .unwrap_or((0.0, None));
                    let rows = healthy
                        .first()
                        .map(|snapshot| TrayStatusRow {
                            key: TrayStatusKey::Provider,
                            snapshot,
                        })
                        .into_iter()
                        .collect();
                    (percents.0, percents.1, rows)
                }
            }
            TrayIconMode::PerProvider => {
                let percents = selected
                    .map(|snapshot| selected_tray_percents(snapshot, settings))
                    .unwrap_or_else(|| fallback_percents(&healthy, settings));
                let rows = healthy
                    .iter()
                    .copied()
                    .map(|snapshot| TrayStatusRow {
                        key: TrayStatusKey::Provider,
                        snapshot,
                    })
                    .collect();
                (percents.0, percents.1, rows)
            }
            TrayIconMode::Single => {
                let percents = selected
                    .map(|snapshot| selected_tray_percents(snapshot, settings))
                    .unwrap_or_else(|| fallback_percents(&healthy, settings));
                let rows = selected
                    .map(|snapshot| TrayStatusRow {
                        key: TrayStatusKey::Summary,
                        snapshot,
                    })
                    .into_iter()
                    .collect();
                (percents.0, percents.1, rows)
            }
        };

        let icon = resolve_icon_plan(settings, primary_percent, secondary_percent, has_error);

        Self {
            settings,
            icon,
            status_rows,
        }
    }

    pub(crate) fn render_icon(&self) -> (Vec<u8>, u32, u32) {
        match self.icon {
            TrayIconPlan::Bars {
                primary_percent,
                secondary_percent,
                has_error,
            } => render_bar_icon_rgba(primary_percent, secondary_percent, has_error),
            TrayIconPlan::Percent { percent, has_error } => {
                render_percent_icon_rgba(percent, has_error)
            }
            TrayIconPlan::Stacked {
                top_percent,
                bottom_percent,
                has_error,
            } => render_stacked_bar_icon_rgba(top_percent, bottom_percent, has_error),
        }
    }

    pub(crate) fn status_labels(&self, language: Language) -> Vec<(String, String)> {
        self.status_rows
            .iter()
            .map(|row| {
                let (_, label) = provider_status_label(row.snapshot, self.settings, language);
                let key = match row.key {
                    TrayStatusKey::Summary => "status_summary".to_string(),
                    TrayStatusKey::Provider => row.snapshot.provider_id.clone(),
                };
                (key, label)
            })
            .collect()
    }
}

fn resolve_icon_plan(
    settings: &Settings,
    primary_percent: f64,
    secondary_percent: Option<f64>,
    has_error: bool,
) -> TrayIconPlan {
    if settings.tray_icon_mode == TrayIconMode::Stacked
        && let Some(bottom_percent) = secondary_percent
    {
        TrayIconPlan::Stacked {
            top_percent: primary_percent,
            bottom_percent,
            has_error,
        }
    } else if settings.menu_bar_shows_percent {
        TrayIconPlan::Percent {
            percent: primary_percent,
            has_error,
        }
    } else {
        TrayIconPlan::Bars {
            primary_percent,
            secondary_percent,
            has_error,
        }
    }
}

fn fallback_percents(
    healthy: &[&ProviderUsageSnapshot],
    settings: &Settings,
) -> (f64, Option<f64>) {
    (
        healthy
            .iter()
            .map(|snapshot| selected_tray_percents(snapshot, settings).0)
            .fold(0.0_f64, f64::max),
        None,
    )
}

fn ordered_snapshot_refs<'a>(
    settings: &Settings,
    snapshots: &'a [ProviderUsageSnapshot],
) -> Vec<&'a ProviderUsageSnapshot> {
    let order = settings
        .provider_display_order_names()
        .into_iter()
        .enumerate()
        .map(|(index, provider_id)| (provider_id, index))
        .collect::<std::collections::HashMap<_, _>>();
    let mut ordered = snapshots.iter().collect::<Vec<_>>();
    ordered.sort_by(|a, b| {
        let a_order = order.get(&a.provider_id);
        let b_order = order.get(&b.provider_id);
        match (a_order, b_order) {
            (Some(a_order), Some(b_order)) if a_order != b_order => a_order.cmp(b_order),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            _ => a.display_name.cmp(&b.display_name),
        }
    });
    ordered
}

fn provider_status_label(
    snapshot: &ProviderUsageSnapshot,
    settings: &Settings,
    language: Language,
) -> (String, String) {
    let provider = codexbar::core::ProviderId::from_cli_name(&snapshot.provider_id);
    let preference = provider
        .map(|id| settings.get_provider_metric(id))
        .unwrap_or_default();
    if preference == MetricPreference::MonthlyPlan
        && let Some(cost) = snapshot.cost.as_ref()
    {
        let amount = if !cost.formatted_used.is_empty() {
            cost.formatted_used.clone()
        } else {
            crate::commands::format_cost_amount(cost)
        };
        return (
            snapshot.provider_id.clone(),
            format!("{} {}", snapshot.display_name, amount),
        );
    }

    let label = crate::commands::compact_tray_status_label(headline_window(snapshot), language);
    (
        snapshot.provider_id.clone(),
        format!("{} {}", snapshot.display_name, label),
    )
}

/// Window that headline tray surfaces should label for a provider.
pub(crate) fn headline_window(snapshot: &ProviderUsageSnapshot) -> &RateWindowSnapshot {
    if snapshot.provider_id == "codex" {
        codex_lane_headline_window(snapshot)
    } else {
        &snapshot.primary
    }
}

/// Pick the first non-informational Codex lane in session, weekly, monthly order.
pub(crate) fn codex_lane_headline_window(snapshot: &ProviderUsageSnapshot) -> &RateWindowSnapshot {
    if !snapshot.primary.is_informational {
        return &snapshot.primary;
    }
    if let Some(ref secondary) = snapshot.secondary
        && !secondary.is_informational
    {
        return secondary;
    }
    if let Some(ref tertiary) = snapshot.tertiary
        && !tertiary.is_informational
    {
        return tertiary;
    }
    &snapshot.primary
}

/// Resolve a stable top/bottom pair while retaining stale saved preferences.
fn pick_stacked_tray_providers<'a>(
    healthy: &'a [&'a ProviderUsageSnapshot],
    settings: &Settings,
) -> Option<(&'a ProviderUsageSnapshot, &'a ProviderUsageSnapshot)> {
    if healthy.len() < 2 {
        return None;
    }

    let preferred = |provider_id: Option<&str>| {
        provider_id.and_then(|id| {
            healthy
                .iter()
                .copied()
                .find(|snapshot| snapshot.provider_id == id)
        })
    };
    let preferred_bottom = preferred(settings.stacked_tray_bottom_provider.as_deref());
    let top = preferred(settings.stacked_tray_top_provider.as_deref()).or_else(|| {
        healthy.iter().copied().find(|snapshot| {
            preferred_bottom.map(|bottom| bottom.provider_id.as_str())
                != Some(snapshot.provider_id.as_str())
        })
    })?;
    let bottom = preferred_bottom
        .filter(|snapshot| snapshot.provider_id != top.provider_id)
        .or_else(|| {
            healthy
                .iter()
                .copied()
                .find(|snapshot| snapshot.provider_id != top.provider_id)
        })?;

    Some((top, bottom))
}

fn pick_tray_provider<'a>(
    healthy: &'a [&'a ProviderUsageSnapshot],
    prefer_highest: bool,
) -> Option<&'a ProviderUsageSnapshot> {
    if prefer_highest {
        healthy.iter().copied().max_by(|a, b| {
            a.primary
                .used_percent
                .partial_cmp(&b.primary.used_percent)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    } else {
        healthy.first().copied()
    }
}

fn selected_tray_percents(
    snapshot: &ProviderUsageSnapshot,
    settings: &Settings,
) -> (f64, Option<f64>) {
    let (selected, companion) =
        crate::usage_metric::selected_usage_icon_windows(snapshot, settings);
    (
        display_metric_percent(&selected, settings.show_as_used),
        companion
            .as_ref()
            .map(|window| display_metric_percent(window, settings.show_as_used)),
    )
}

fn display_metric_percent(window: &RateWindowSnapshot, show_as_used: bool) -> f64 {
    if window.is_informational {
        return 0.0;
    }
    if window.is_exhausted || window.used_percent >= 100.0 {
        return if show_as_used { 100.0 } else { 0.0 };
    }

    let used = window.used_percent.clamp(0.0, 100.0);
    if show_as_used { used } else { 100.0 - used }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codexbar::core::{ProviderId, ProviderStateKind};

    fn fake_snapshot(id: &str, display_name: &str, used_percent: f64) -> ProviderUsageSnapshot {
        fake_snapshot_with(id, display_name, used_percent, None, None, None)
    }

    fn fake_snapshot_with(
        id: &str,
        display_name: &str,
        used_percent: f64,
        secondary_percent: Option<f64>,
        tertiary_percent: Option<f64>,
        cost: Option<(f64, f64)>,
    ) -> ProviderUsageSnapshot {
        let window = |percent: f64| RateWindowSnapshot {
            used_percent: percent,
            remaining_percent: 100.0 - percent,
            window_minutes: None,
            resets_at: None,
            reset_description: None,
            is_exhausted: false,
            is_informational: false,
            reserve_percent: None,
            reserve_description: None,
            reserve_will_last_to_reset: false,
            reserve_eta_seconds: None,
        };

        ProviderUsageSnapshot {
            provider_id: id.into(),
            display_name: display_name.into(),
            primary: window(used_percent),
            primary_label: None,
            secondary: secondary_percent.map(window),
            secondary_label: None,
            model_specific: None,
            tertiary: tertiary_percent.map(window),
            tertiary_label: None,
            extra_rate_windows: Vec::new(),
            inventory: Vec::new(),
            display_details: Vec::new(),
            cost: cost.map(|(used, limit)| crate::commands::CostSnapshotBridge {
                used,
                limit: Some(limit),
                remaining: Some((limit - used).max(0.0)),
                currency_code: "USD".to_string(),
                currency_symbol: None,
                period: "monthly".to_string(),
                resets_at: None,
                formatted_used: format!("${used:.2}"),
                formatted_limit: Some(format!("${limit:.2}")),
                balance: None,
                formatted_balance: None,
                balance_updated_at: None,
                account_id: None,
                daily: Vec::new(),
                always_visible: false,
            }),
            plan_name: None,
            account_email: None,
            subscription: None,
            source_label: String::new(),
            has_successful_claude_cli_quota: false,
            updated_at: "2025-01-01T00:00:00Z".into(),
            error: None,
            error_state: ProviderStateKind::Ready,
            pace: None,
            account_organization: None,
            tray_status_label: None,
            fetch_duration_ms: None,
            wayfinder_usage: None,
            session_equivalent_forecast: None,
        }
    }

    #[test]
    fn single_plan_uses_highest_provider_for_icon_and_summary() {
        let settings = Settings {
            tray_icon_mode: TrayIconMode::Single,
            menu_bar_shows_highest_usage: true,
            ..Settings::default()
        };
        let snapshots = vec![
            fake_snapshot("codex", "Codex", 30.0),
            fake_snapshot("claude", "Claude", 72.0),
        ];

        let plan = TrayPresentationPlan::resolve(&settings, &snapshots);

        assert_eq!(
            plan.icon,
            TrayIconPlan::Bars {
                primary_percent: 72.0,
                secondary_percent: None,
                has_error: false,
            }
        );
        assert_eq!(
            plan.status_labels(Language::English),
            vec![("status_summary".to_string(), "Claude 72%".to_string())]
        );
    }

    #[test]
    fn per_provider_plan_preserves_configured_order_for_status_rows() {
        let settings = Settings {
            tray_icon_mode: TrayIconMode::PerProvider,
            provider_order: codexbar::settings::normalize_provider_order(&[
                "claude".to_string(),
                "codex".to_string(),
            ]),
            ..Settings::default()
        };
        let snapshots = vec![
            fake_snapshot("codex", "Codex", 30.0),
            fake_snapshot("claude", "Claude", 72.0),
        ];

        let labels =
            TrayPresentationPlan::resolve(&settings, &snapshots).status_labels(Language::English);

        assert_eq!(
            labels,
            vec![
                ("claude".to_string(), "Claude 72%".to_string()),
                ("codex".to_string(), "Codex 30%".to_string()),
            ]
        );
    }

    #[test]
    fn stacked_plan_resolves_distinct_preferences_once() {
        let settings = Settings {
            tray_icon_mode: TrayIconMode::Stacked,
            stacked_tray_top_provider: Some("claude".to_string()),
            stacked_tray_bottom_provider: Some("codex".to_string()),
            ..Settings::default()
        };
        let snapshots = vec![
            fake_snapshot("codex", "Codex", 30.0),
            fake_snapshot("claude", "Claude", 72.0),
            fake_snapshot("gemini", "Gemini", 44.0),
        ];

        let plan = TrayPresentationPlan::resolve(&settings, &snapshots);

        assert_eq!(
            plan.icon,
            TrayIconPlan::Stacked {
                top_percent: 72.0,
                bottom_percent: 30.0,
                has_error: false,
            }
        );
        assert_eq!(
            plan.status_labels(Language::English),
            vec![
                ("claude".to_string(), "Claude 72%".to_string()),
                ("codex".to_string(), "Codex 30%".to_string()),
            ]
        );
    }

    #[test]
    fn stacked_plan_falls_back_around_stale_and_duplicate_preferences() {
        let settings = Settings {
            tray_icon_mode: TrayIconMode::Stacked,
            stacked_tray_top_provider: Some("missing".to_string()),
            stacked_tray_bottom_provider: Some("claude".to_string()),
            ..Settings::default()
        };
        let snapshots = vec![
            fake_snapshot("codex", "Codex", 30.0),
            fake_snapshot("claude", "Claude", 72.0),
        ];

        let plan = TrayPresentationPlan::resolve(&settings, &snapshots);

        assert_eq!(
            plan.icon,
            TrayIconPlan::Stacked {
                top_percent: 30.0,
                bottom_percent: 72.0,
                has_error: false,
            }
        );
        assert_eq!(plan.status_rows[0].snapshot.provider_id, "codex");
        assert_eq!(plan.status_rows[1].snapshot.provider_id, "claude");
    }

    #[test]
    fn one_provider_stacked_plan_preserves_secondary_window_fallback() {
        let settings = Settings {
            tray_icon_mode: TrayIconMode::Stacked,
            ..Settings::default()
        };
        let snapshots = vec![fake_snapshot_with(
            "codex",
            "Codex",
            30.0,
            Some(65.0),
            None,
            None,
        )];

        let plan = TrayPresentationPlan::resolve(&settings, &snapshots);

        assert_eq!(
            plan.icon,
            TrayIconPlan::Stacked {
                top_percent: 65.0,
                bottom_percent: 30.0,
                has_error: false,
            }
        );
        assert_eq!(plan.status_rows.len(), 1);
    }

    #[test]
    fn all_errors_produce_error_styled_zero_percent_plan() {
        let settings = Settings {
            menu_bar_shows_percent: true,
            ..Settings::default()
        };
        let mut snapshot = fake_snapshot("codex", "Codex", 30.0);
        snapshot.error = Some("offline".to_string());
        let snapshots = vec![snapshot];

        let plan = TrayPresentationPlan::resolve(&settings, &snapshots);

        assert_eq!(
            plan.icon,
            TrayIconPlan::Percent {
                percent: 0.0,
                has_error: true,
            }
        );
        assert!(plan.status_rows.is_empty());
    }

    #[test]
    fn plan_uses_selected_metric_and_remaining_display_mode() {
        let mut settings = Settings {
            show_as_used: false,
            ..Settings::default()
        };
        settings.set_provider_metric(ProviderId::Cursor, MetricPreference::ExtraUsage);
        let snapshots = vec![fake_snapshot_with(
            "cursor",
            "Cursor",
            10.0,
            Some(20.0),
            Some(72.0),
            Some((15.0, 100.0)),
        )];

        let plan = TrayPresentationPlan::resolve(&settings, &snapshots);

        assert_eq!(
            plan.icon,
            TrayIconPlan::Bars {
                primary_percent: 85.0,
                secondary_percent: Some(80.0),
                has_error: false,
            }
        );
    }

    #[test]
    fn render_icon_delegates_to_resolved_stacked_renderer() {
        let settings = Settings {
            tray_icon_mode: TrayIconMode::Stacked,
            stacked_tray_top_provider: Some("claude".to_string()),
            stacked_tray_bottom_provider: Some("codex".to_string()),
            ..Settings::default()
        };
        let snapshots = vec![
            fake_snapshot("codex", "Codex", 40.0),
            fake_snapshot("claude", "Claude", 72.0),
        ];
        let plan = TrayPresentationPlan::resolve(&settings, &snapshots);

        assert_eq!(
            plan.render_icon(),
            render_stacked_bar_icon_rgba(72.0, 40.0, false)
        );
    }

    #[test]
    fn codex_headline_skips_informational_primary() {
        let mut snapshot = fake_snapshot_with("codex", "Codex", 0.0, Some(25.0), Some(30.0), None);
        snapshot.primary.is_informational = true;

        assert_eq!(codex_lane_headline_window(&snapshot).used_percent, 25.0);
    }
    fn fake_extra_window(percent: f64) -> crate::commands::NamedRateWindowSnapshot {
        crate::commands::NamedRateWindowSnapshot {
            id: "additional_budget".to_string(),
            title: "Additional Budget".to_string(),
            fallback_lane: false,
            window: crate::commands::RateWindowSnapshot {
                used_percent: percent,
                remaining_percent: 100.0 - percent,
                window_minutes: None,
                resets_at: None,
                reset_description: None,
                is_exhausted: false,
                is_informational: false,
                reserve_percent: None,
                reserve_description: None,
                reserve_will_last_to_reset: false,
                reserve_eta_seconds: None,
            },
        }
    }

    #[test]
    fn selected_tray_percent_uses_cursor_extra_usage_cost() {
        let mut settings = Settings::default();
        settings.set_provider_metric(ProviderId::Cursor, MetricPreference::ExtraUsage);
        let snapshot = fake_snapshot_with(
            "cursor",
            "Cursor",
            10.0,
            Some(20.0),
            Some(72.0),
            Some((15.0, 100.0)),
        );

        let (primary, secondary) = selected_tray_percents(&snapshot, &settings);

        assert_eq!(primary, 15.0);
        assert_eq!(secondary, Some(20.0));
    }

    #[test]
    fn selected_tray_percent_tracks_extra_rate_window() {
        let mut settings = Settings::default();
        settings.set_provider_metric(ProviderId::Copilot, MetricPreference::ExtraUsage);
        let mut snapshot = fake_snapshot("copilot", "Copilot", 20.0);
        snapshot.extra_rate_windows.push(fake_extra_window(42.0));

        let (primary, secondary) = selected_tray_percents(&snapshot, &settings);

        assert_eq!(primary, 42.0);
        assert_eq!(secondary, None);
    }

    #[test]
    fn copilot_automatic_tracks_highest_extra_rate_window() {
        let settings = Settings::default();
        let mut snapshot = fake_snapshot("copilot", "Copilot", 20.0);
        snapshot.extra_rate_windows.push(fake_extra_window(42.0));

        let (primary, _) = selected_tray_percents(&snapshot, &settings);

        assert_eq!(primary, 42.0);
    }

    #[test]
    fn selected_tray_percent_respects_remaining_display_mode() {
        let mut settings = Settings {
            show_as_used: false,
            ..Settings::default()
        };
        settings.set_provider_metric(ProviderId::Cursor, MetricPreference::ExtraUsage);
        let snapshot = fake_snapshot_with(
            "cursor",
            "Cursor",
            10.0,
            Some(20.0),
            Some(72.0),
            Some((15.0, 100.0)),
        );

        let (primary, secondary) = selected_tray_percents(&snapshot, &settings);

        assert_eq!(primary, 85.0);
        assert_eq!(secondary, Some(80.0));
    }

    #[test]
    fn exhausted_automatic_window_never_renders_as_remaining_progress() {
        let mut settings = Settings {
            show_as_used: false,
            ..Settings::default()
        };
        let mut snapshot = fake_snapshot_with(
            "opencodego",
            "OpenCode Go",
            20.0,
            Some(60.0),
            Some(40.0),
            None,
        );
        snapshot
            .tertiary
            .as_mut()
            .expect("monthly quota")
            .is_exhausted = true;

        let (remaining, _) = selected_tray_percents(&snapshot, &settings);
        assert_eq!(remaining, 0.0);

        settings.show_as_used = true;
        let (used, _) = selected_tray_percents(&snapshot, &settings);
        assert_eq!(used, 100.0);
    }

    #[test]
    fn full_automatic_window_without_exhausted_flag_has_zero_remaining_progress() {
        let mut settings = Settings {
            show_as_used: false,
            ..Settings::default()
        };
        let mut snapshot = fake_snapshot_with(
            "opencodego",
            "OpenCode Go",
            20.0,
            Some(60.0),
            Some(100.0),
            None,
        );
        snapshot
            .tertiary
            .as_mut()
            .expect("monthly quota")
            .is_exhausted = false;

        let (remaining, _) = selected_tray_percents(&snapshot, &settings);
        assert_eq!(remaining, 0.0);

        settings.show_as_used = true;
        let (used, _) = selected_tray_percents(&snapshot, &settings);
        assert_eq!(used, 100.0);
    }

    #[test]
    fn missing_automatic_window_does_not_look_like_available_remaining_progress() {
        let settings = Settings {
            show_as_used: false,
            ..Settings::default()
        };
        let mut snapshot = fake_snapshot_with("opencodego", "OpenCode Go", 0.0, None, None, None);
        snapshot.primary.is_informational = true;

        let (remaining, _) = selected_tray_percents(&snapshot, &settings);

        assert_eq!(remaining, 0.0);
    }

    #[test]
    fn selected_tray_percent_falls_back_when_extra_usage_missing() {
        let mut settings = Settings::default();
        settings.set_provider_metric(ProviderId::Cursor, MetricPreference::ExtraUsage);
        let snapshot = fake_snapshot_with("cursor", "Cursor", 10.0, Some(72.0), None, None);

        let (primary, _) = selected_tray_percents(&snapshot, &settings);

        assert_eq!(primary, 72.0);
    }

    #[test]
    fn single_meaningful_secondary_quota_uses_full_single_meter() {
        let settings = Settings::default();
        let mut snapshot = fake_snapshot_with("claude", "Claude", 0.0, Some(42.0), None, None);
        snapshot.primary.is_informational = true;

        let (primary, secondary) = selected_tray_percents(&snapshot, &settings);

        assert_eq!(primary, 42.0);
        assert_eq!(secondary, None);
    }

    #[test]
    fn selected_secondary_quota_is_not_duplicated_when_tertiary_is_meaningful() {
        let settings = Settings::default();
        let mut snapshot =
            fake_snapshot_with("claude", "Claude", 0.0, Some(42.0), Some(30.0), None);
        snapshot.primary.is_informational = true;

        let (primary, secondary) = selected_tray_percents(&snapshot, &settings);

        assert_eq!(primary, 42.0);
        assert_eq!(secondary, Some(30.0));
    }

    #[test]
    fn two_meaningful_quotas_keep_two_meter_layout() {
        let mut settings = Settings::default();
        settings.set_provider_metric(ProviderId::Cursor, MetricPreference::Session);
        let snapshot = fake_snapshot_with("cursor", "Cursor", 15.0, Some(40.0), None, None);

        let (primary, secondary) = selected_tray_percents(&snapshot, &settings);

        assert_eq!(primary, 15.0);
        assert_eq!(secondary, Some(40.0));
    }

    #[test]
    fn informational_primary_skips_session_and_automatic_phantom_zero() {
        let mut settings = Settings::default();
        settings.set_provider_metric(ProviderId::Claude, MetricPreference::Session);
        let mut snapshot = fake_snapshot_with("claude", "Claude", 0.0, Some(42.0), None, None);
        snapshot.primary.is_informational = true;

        // Session preference must not paint the synthetic 0% primary;
        // it falls through to Automatic which prefers weekly (42%).
        let (primary, _) = selected_tray_percents(&snapshot, &settings);
        assert_eq!(primary, 42.0);
        assert_ne!(primary, 0.0);

        // Automatic also prefers weekly over informational primary.
        settings.set_provider_metric(ProviderId::Claude, MetricPreference::Automatic);
        let (primary, _) = selected_tray_percents(&snapshot, &settings);
        assert_eq!(primary, 42.0);
    }

    #[test]
    fn claude_automatic_prefers_weekly_when_model_exhausted() {
        let settings = Settings::default();
        let mut snapshot = fake_snapshot_with("claude", "Claude", 40.0, Some(22.0), None, None);
        snapshot.model_specific = Some(crate::commands::RateWindowSnapshot {
            used_percent: 100.0,
            remaining_percent: 0.0,
            window_minutes: Some(10080),
            resets_at: None,
            reset_description: None,
            is_exhausted: true,
            is_informational: false,
            reserve_percent: None,
            reserve_description: None,
            reserve_will_last_to_reset: false,
            reserve_eta_seconds: None,
        });

        let (primary, _) = selected_tray_percents(&snapshot, &settings);
        assert_eq!(primary, 22.0);

        // Explicit model override is untouched.
        let mut overridden = settings.clone();
        overridden.set_provider_metric(ProviderId::Claude, MetricPreference::Model);
        let (primary, _) = selected_tray_percents(&snapshot, &overridden);
        assert_eq!(primary, 100.0);
    }

    #[test]
    fn automatic_prefers_exhausted_weekly_over_low_session() {
        let settings = Settings::default();
        let snapshot = fake_snapshot_with("codex", "Codex", 20.0, Some(100.0), None, None);

        let (primary, _) = selected_tray_percents(&snapshot, &settings);
        assert_eq!(primary, 100.0);

        // Explicit session override still wins.
        let mut overridden = settings.clone();
        overridden.set_provider_metric(ProviderId::Codex, MetricPreference::Session);
        let (primary, _) = selected_tray_percents(&snapshot, &overridden);
        assert_eq!(primary, 20.0);
    }

    #[test]
    fn automatic_picks_highest_among_model_and_extra_windows() {
        let settings = Settings::default();
        let mut snapshot =
            fake_snapshot_with("gemini", "Gemini", 10.0, Some(30.0), Some(40.0), None);
        snapshot.model_specific = Some(crate::commands::RateWindowSnapshot {
            used_percent: 55.0,
            remaining_percent: 45.0,
            window_minutes: None,
            resets_at: None,
            reset_description: None,
            is_exhausted: false,
            is_informational: false,
            reserve_percent: None,
            reserve_description: None,
            reserve_will_last_to_reset: false,
            reserve_eta_seconds: None,
        });
        snapshot.extra_rate_windows.push(fake_extra_window(90.0));

        let (primary, _) = selected_tray_percents(&snapshot, &settings);
        assert_eq!(primary, 90.0);
    }

    #[test]
    fn f5_headline_prefers_non_informational_primary() {
        let snapshot = fake_snapshot_with("codex", "Codex", 50.0, Some(20.0), Some(30.0), None);
        let headline = codex_lane_headline_window(&snapshot);
        assert!((headline.used_percent - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn f5_headline_falls_back_to_secondary_when_primary_informational() {
        let mut snapshot = fake_snapshot_with("codex", "Codex", 0.0, Some(25.0), Some(30.0), None);
        snapshot.primary.is_informational = true;
        let headline = codex_lane_headline_window(&snapshot);
        assert!((headline.used_percent - 25.0).abs() < f64::EPSILON);
    }

    #[test]
    fn f5_headline_falls_back_to_tertiary_when_primary_and_secondary_informational() {
        let mut snapshot = fake_snapshot_with("codex", "Codex", 0.0, Some(0.0), Some(35.0), None);
        snapshot.primary.is_informational = true;
        snapshot.secondary.as_mut().unwrap().is_informational = true;
        let headline = codex_lane_headline_window(&snapshot);
        assert!((headline.used_percent - 35.0).abs() < f64::EPSILON);
    }

    #[test]
    fn f5_headline_returns_primary_when_all_informational() {
        let mut snapshot = fake_snapshot_with("codex", "Codex", 0.0, Some(0.0), Some(0.0), None);
        snapshot.primary.is_informational = true;
        if let Some(sec) = &mut snapshot.secondary {
            sec.is_informational = true;
        }
        if let Some(ter) = &mut snapshot.tertiary {
            ter.is_informational = true;
        }
        let headline = codex_lane_headline_window(&snapshot);
        // Falls back to primary (the placeholder) when all are informational.
        assert!(headline.is_informational);
    }
}
