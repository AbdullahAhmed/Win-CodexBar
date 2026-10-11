import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const tauriMocks = vi.hoisted(() => ({
  getSettingsSnapshot: vi.fn(),
  getUsageSpendSummary: vi.fn(),
  updateSettings: vi.fn(),
  writeUsageSpendExport: vi.fn(),
}));

vi.mock("../../../lib/tauri", () => tauriMocks);
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

const EN: Record<string, string> = {
  CostPeriodMonthToDate: "Month to date",
  CostPeriodAll: "All available history",
  CostPeriodToday: "Today",
  CostPeriodLastDays: "Last {} days",
  CostPeriodCustom: "Custom days",
  UsageSpendModelsPeriodCaption: "Models for: {}",
  UsageSpendProjectsPeriodCaption: "Projects for: {}",
};
vi.mock("../../../hooks/useLocale", () => ({
  useLocale: () => ({ t: (key: string) => EN[key] ?? key }),
}));

import UsageSpendTab from "./UsageSpendTab";
import type { TabProps } from "../settingsTabs";
import type { SpendContract, UsageSpendSummary } from "../../../types/bridge";

function contract(reportingPeriod: string): SpendContract {
  return {
    providerId: "codex",
    historyDays: 30,
    reportingPeriod,
    knownCostUsd: 12,
    knownZero: false,
    provenance: "listPriceEstimate",
    priceCoverage: { priced: 1, unpriced: 0 },
    priceCoverageRatio: 1,
    historyCoverageEstablished: true,
    tokenMix: {},
    conversationCount: 1,
    models: [],
    projects: [],
    conversations: [],
    daily: [],
    hourlyActivity: [],
    projectSourceStatus: null,
    imports: [],
    customPricingActive: false,
  } as unknown as SpendContract;
}

function summary(reportingPeriod: string): UsageSpendSummary {
  return {
    rows: [
      {
        providerId: "codex",
        displayName: "Codex",
        sevenDay: 1,
        thirtyDay: 2,
        periodCost: 9,
        periodTokens: 900,
        currency: "USD",
        source: "local",
        includedInOverview: true,
      },
    ],
    contract: contract(reportingPeriod),
    reportingPeriod,
    reportingDay: "2026-09-29",
    dashboardTimezone: "UTC",
  } as unknown as UsageSpendSummary;
}

const props = {} as TabProps;

function lastScanPeriod(): string | undefined {
  const calls = tauriMocks.getUsageSpendSummary.mock.calls;
  return calls[calls.length - 1]?.[0]?.period;
}

describe("UsageSpendTab History window", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    tauriMocks.getSettingsSnapshot.mockResolvedValue({
      costReportingPeriod: "month-to-date",
      costSummaryDisplayStyle: "compact",
    });
    tauriMocks.updateSettings.mockResolvedValue({});
    tauriMocks.getUsageSpendSummary.mockImplementation(
      async (options?: { period?: string }) => summary(options?.period ?? "rolling:30"),
    );
  });

  it("scans once with the saved window and labels the period with it", async () => {
    render(<UsageSpendTab {...props} />);

    await waitFor(() => expect(tauriMocks.getUsageSpendSummary).toHaveBeenCalledTimes(1));
    expect(tauriMocks.getUsageSpendSummary).toHaveBeenCalledWith({
      period: "month-to-date",
      forceRefresh: false,
    });
    // The period column is labelled with the window and shows its numbers.
    expect(await screen.findByRole("columnheader", { name: "Month to date" })).toBeTruthy();
    expect(screen.getByText(/900/)).toBeTruthy();
    expect(screen.getByText("Models for: Month to date")).toBeTruthy();
  });

  it("migrates a settings payload without a saved window to 30 days", async () => {
    tauriMocks.getSettingsSnapshot.mockResolvedValue({ costSummaryDisplayStyle: "compact" });
    render(<UsageSpendTab {...props} />);

    await waitFor(() => expect(tauriMocks.getUsageSpendSummary).toHaveBeenCalled());
    expect(lastScanPeriod()).toBe("rolling:30");
  });

  it("falls back to 30 days when the saved window is unreadable", async () => {
    tauriMocks.getSettingsSnapshot.mockResolvedValue({
      costReportingPeriod: "rolling:0",
      costSummaryDisplayStyle: "compact",
    });
    render(<UsageSpendTab {...props} />);

    await waitFor(() => expect(tauriMocks.getUsageSpendSummary).toHaveBeenCalled());
    expect(lastScanPeriod()).toBe("rolling:30");
  });

  it("leaves the History window and Cost summary pickers to Settings > Menu", async () => {
    render(<UsageSpendTab {...props} />);
    await waitFor(() => expect(tauriMocks.getUsageSpendSummary).toHaveBeenCalledTimes(1));

    expect(screen.queryByLabelText("CostPeriodHistoryWindow")).toBeNull();
    expect(screen.queryByText("CostSummaryDisplayStyle")).toBeNull();
    expect(tauriMocks.updateSettings).not.toHaveBeenCalled();
  });
});
