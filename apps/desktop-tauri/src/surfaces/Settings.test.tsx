import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { BootstrapState } from "../types/bridge";

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    setSize: () => Promise.resolve(),
    setPosition: () => Promise.resolve(),
    minimize: () => Promise.resolve(),
  }),
  LogicalPosition: class {},
  LogicalSize: class {},
}));
vi.mock("@tauri-apps/api/webviewWindow", () => ({
  getCurrentWebviewWindow: () => ({ label: "settings" }),
}));
vi.mock("../lib/tauri", () => ({
  closeSettingsWindow: vi.fn(),
  getWorkAreaRect: () => Promise.resolve(null),
  setSurfaceMode: vi.fn(),
}));
vi.mock("../hooks/useSettings", () => ({
  useSettings: (settings: unknown) => ({
    settings,
    saving: false,
    error: null,
    update: vi.fn(),
  }),
}));
vi.mock("../hooks/useSurfaceMode", () => ({ useSurfaceTarget: () => null }));
vi.mock("../hooks/useLocale", () => ({ useLocale: () => ({ t: (key: string) => key }) }));
vi.mock("./settings/tabs/GeneralTab", () => ({
  default: ({ mode }: { mode: string }) => <div data-testid="tab">general:{mode}</div>,
}));
vi.mock("./settings/tabs/DisplayTab", () => ({
  default: ({ mode }: { mode: string }) => <div data-testid="tab">display:{mode}</div>,
}));
vi.mock("./settings/tabs/AdvancedTab", () => ({
  default: () => <div data-testid="tab">advanced</div>,
}));
vi.mock("./settings/tabs/AboutTab", () => ({
  default: () => <div data-testid="tab">about</div>,
}));
vi.mock("./settings/tabs/UsageSpendTab", () => ({
  default: () => <div data-testid="tab">usageSpend</div>,
}));
vi.mock("./settings/tabs/ProvidersTab", () => ({
  default: ({
    focusProviderId,
    focusRequest,
  }: {
    focusProviderId: string | null;
    focusRequest: number;
  }) => (
    <div data-testid="tab">
      providers:{String(focusProviderId)}:{focusRequest}
    </div>
  ),
}));

import Settings from "./Settings";

const state = { settings: {}, providers: [] } as unknown as BootstrapState;

describe("Settings deep links", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("opens cold on Providers with the requested provider focused", () => {
    render(<Settings state={state} initialTab="providers:claude" tabRequestId={1} />);
    expect(screen.getByTestId("tab").textContent).toBe("providers:claude:1");
  });

  it("switches from another tab to Providers and refocuses on a repeated request", () => {
    const { rerender } = render(<Settings state={state} initialTab="general" tabRequestId={0} />);
    expect(screen.getByTestId("tab").textContent).toBe("general:general");

    rerender(<Settings state={state} initialTab="providers:codex" tabRequestId={1} />);
    expect(screen.getByTestId("tab").textContent).toBe("providers:codex:1");

    rerender(<Settings state={state} initialTab="providers:codex" tabRequestId={2} />);
    expect(screen.getByTestId("tab").textContent).toBe("providers:codex:2");
  });

  it("keeps the focused provider when a plain tab request arrives", () => {
    const { rerender } = render(
      <Settings state={state} initialTab="providers:grok" tabRequestId={1} />,
    );
    rerender(<Settings state={state} initialTab="about" tabRequestId={2} />);
    expect(screen.getByTestId("tab").textContent).toBe("about");

    rerender(<Settings state={state} initialTab="providers" tabRequestId={3} />);
    expect(screen.getByTestId("tab").textContent).toBe("providers:grok:1");
  });
});
