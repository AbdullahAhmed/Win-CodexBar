import { describe, expect, it } from "vitest";
import type { ProviderUsageSnapshot, RateWindowSnapshot } from "../types/bridge";
import { accountMenuAction } from "./accountMenuRow";

const window: RateWindowSnapshot = {
  usedPercent: 25,
  remainingPercent: 75,
  windowMinutes: 300,
  resetsAt: null,
  resetDescription: null,
  isExhausted: false,
  reservePercent: null,
  reserveDescription: null,
  isInformational: false,
};

function snapshot(overrides: Partial<ProviderUsageSnapshot>): ProviderUsageSnapshot {
  return {
    providerId: "codex",
    displayName: "Codex",
    primary: window,
    selectedMetric: window,
    secondary: null,
    modelSpecific: null,
    tertiary: null,
    extraRateWindows: [],
    cost: null,
    planName: null,
    accountEmail: null,
    sourceLabel: "auto",
    updatedAt: "2026-10-11T00:00:00Z",
    error: null,
    errorState: "ready",
    pace: null,
    accountOrganization: null,
    trayStatusLabel: null,
    ...overrides,
  };
}

describe("accountMenuAction", () => {
  it.each(["codex", "claude", "grok"])("offers Switch Account... for a signed-in %s", (providerId) => {
    expect(accountMenuAction(snapshot({ providerId, accountEmail: "a@example.com" }))).toEqual({
      labelKey: "TrayMenuSwitchAccount",
      settingsTab: `providers:${providerId}`,
    });
  });

  it.each(["codex", "claude", "grok"])("offers Add Account... when %s has no account", (providerId) => {
    expect(accountMenuAction(snapshot({ providerId }))).toEqual({
      labelKey: "AddAccountEllipsis",
      settingsTab: `providers:${providerId}`,
    });
  });

  it("treats a live Claude CLI quota as a signed-in account without an email", () => {
    expect(
      accountMenuAction(
        snapshot({ providerId: "claude", sourceLabel: "Claude CLI", hasSuccessfulClaudeCliQuota: true }),
      )?.labelKey,
    ).toBe("TrayMenuSwitchAccount");
  });

  it("ignores a blank email", () => {
    expect(accountMenuAction(snapshot({ accountEmail: "  " }))?.labelKey).toBe("AddAccountEllipsis");
  });

  it("offers nothing for providers without an account manager", () => {
    expect(accountMenuAction(snapshot({ providerId: "cursor", accountEmail: "a@example.com" }))).toBeNull();
    expect(accountMenuAction(null)).toBeNull();
  });
});
