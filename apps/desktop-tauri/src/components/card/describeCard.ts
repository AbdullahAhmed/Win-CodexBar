import type {
  CostSummaryDisplayStyle,
  ProviderChartData,
  ProviderUsageSnapshot,
} from "../../types/bridge";
import type { MetricEntry } from "./CardMetricRow";

export interface MenuCardPresence {
  hasMetrics: boolean;
  /** Inventory rows or provider key-value details. */
  hasDetailSections: boolean;
  hasCost: boolean;
  localUsage: ProviderChartData["localUsage"] | null;
  wayfinderUsage: ProviderUsageSnapshot["wayfinderUsage"] | null;
  hasDetails: boolean;
}

/**
 * Which card sections exist. A card with an error, or whose only content the
 * compact Overview drops, renders its header alone with no divider.
 */
export function describeCard(
  provider: ProviderUsageSnapshot,
  chartData: ProviderChartData | null,
  visibleMetrics: MetricEntry[],
  costSummaryDisplayStyle: CostSummaryDisplayStyle = "detailed",
  compactOverview = false,
): MenuCardPresence {
  const hasError = !!provider.error;
  const hasMetrics = !hasError && visibleMetrics.length > 0;
  const hasDetailSections =
    !hasError &&
    ((provider.inventory?.length ?? 0) > 0 || (provider.displayDetails?.length ?? 0) > 0);
  const hasCost =
    !hasError &&
    !!provider.cost &&
    (costSummaryDisplayStyle !== "hidden" || provider.cost.alwaysVisible === true);
  const localUsage = hasError ? null : chartData?.localUsage ?? null;
  const wayfinderUsage =
    !hasError && provider.providerId === "wayfinder" ? provider.wayfinderUsage ?? null : null;
  const hasDetails = compactOverview
    ? hasMetrics || !!wayfinderUsage
    : hasMetrics || hasDetailSections || hasCost || !!localUsage || !!wayfinderUsage;
  return { hasMetrics, hasDetailSections, hasCost, localUsage, wayfinderUsage, hasDetails };
}
