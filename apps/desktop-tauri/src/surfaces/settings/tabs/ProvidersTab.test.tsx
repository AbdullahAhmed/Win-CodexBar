import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type {
  ProviderCatalogEntry,
  ProviderUsageSnapshot,
  RateWindowSnapshot,
  SettingsSnapshot,
} from "../../../types/bridge";

const hookMocks = vi.hoisted(() => ({
  useProviders: vi.fn(),
}));

vi.mock("../../../hooks/useProviders", () => hookMocks);
vi.mock("../../../hooks/useLocale", () => ({
  useLocale: () => ({ t: (key: string) => key }),
}));
vi.mock("../../../lib/tauri", () => ({
  reorderProviders: vi.fn().mockResolvedValue(undefined),
}));
vi.mock("../providers/ProviderDetailPane", () => ({
  ProviderDetailPane: ({
    providerId,
    liveSnapshot,
  }: {
    providerId: string | null;
    liveSnapshot?: { providerId: string } | null;
  }) => (
    <div data-testid="detail-pane" data-live={liveSnapshot?.providerId ?? ""}>
      {providerId}
    </div>
  ),
}));

import ProvidersTab from "./ProvidersTab";

function rateWindow(
  usedPercent: number,
  isInformational?: boolean,
): RateWindowSnapshot {
  return {
    usedPercent,
    remainingPercent: 100 - usedPercent,
    windowMinutes: null,
    resetsAt: null,
    resetDescription: null,
    isExhausted: false,
    isInformational,
    reservePercent: null,
    reserveDescription: null,
  };
}

const provider: ProviderCatalogEntry = {
  id: "codex",
  displayName: "Codex",
  cookieDomain: null,
};

const settings = {
  enabledProviders: [provider.id],
  resetTimeRelative: true,
  providerMetrics: {},
} as SettingsSnapshot;

describe("ProvidersTab", () => {
  it("shows the real secondary percentage when the primary is informational", () => {
    const snapshot: ProviderUsageSnapshot = {
      providerId: provider.id,
      displayName: provider.displayName,
      primary: rateWindow(0, true),
      selectedMetric: rateWindow(42),
      secondary: rateWindow(42),
      modelSpecific: null,
      tertiary: null,
      extraRateWindows: [],
      cost: null,
      planName: null,
      accountEmail: null,
      sourceLabel: "auto",
      updatedAt: new Date().toISOString(),
      error: null,
      errorState: "ready",
      pace: null,
      accountOrganization: null,
      trayStatusLabel: null,
    };
    hookMocks.useProviders.mockReturnValue({ providers: [snapshot] });

    render(
      <ProvidersTab
        settings={settings}
        providers={[provider]}
        set={vi.fn()}
        saving={false}
      />,
    );

    expect(screen.getByText("42%")).toBeInTheDocument();
  });

  const claude: ProviderCatalogEntry = { id: "claude", displayName: "Claude", cookieDomain: null };
  const twoProviderSettings = {
    ...settings,
    enabledProviders: [provider.id, claude.id],
  } as SettingsSnapshot;

  it("selects the deep-linked provider and follows a later request", () => {
    hookMocks.useProviders.mockReturnValue({ providers: [] });
    const { rerender } = render(
      <ProvidersTab
        settings={twoProviderSettings}
        providers={[provider, claude]}
        set={vi.fn()}
        saving={false}
        focusProviderId="claude"
        focusRequest={1}
      />,
    );
    expect(screen.getByTestId("detail-pane")).toHaveTextContent("claude");

    rerender(
      <ProvidersTab
        settings={twoProviderSettings}
        providers={[provider, claude]}
        set={vi.fn()}
        saving={false}
        focusProviderId="codex"
        focusRequest={2}
      />,
    );
    expect(screen.getByTestId("detail-pane")).toHaveTextContent("codex");
  });

  it("falls back to the first provider for an unknown deep link", () => {
    hookMocks.useProviders.mockReturnValue({ providers: [] });
    render(
      <ProvidersTab
        settings={twoProviderSettings}
        providers={[provider, claude]}
        set={vi.fn()}
        saving={false}
        focusProviderId="nope"
        focusRequest={1}
      />,
    );
    expect(screen.getByTestId("detail-pane")).toHaveTextContent("codex");
  });

  it("hands the pane the selected provider's live snapshot only", () => {
    const live = (providerId: string) =>
      ({
        providerId,
        displayName: providerId,
        primary: rateWindow(10),
        selectedMetric: rateWindow(10),
        secondary: null,
        modelSpecific: null,
        tertiary: null,
        extraRateWindows: [],
        error: null,
      }) as unknown as ProviderUsageSnapshot;
    const codexSnapshot = live("codex");
    const claudeSnapshot = live("claude");
    hookMocks.useProviders.mockReturnValue({ providers: [codexSnapshot, claudeSnapshot] });
    render(
      <ProvidersTab
        settings={twoProviderSettings}
        providers={[provider, claude]}
        set={vi.fn()}
        saving={false}
        focusProviderId="claude"
        focusRequest={1}
      />,
    );
    expect(screen.getByTestId("detail-pane")).toHaveAttribute("data-live", "claude");
  });
});
