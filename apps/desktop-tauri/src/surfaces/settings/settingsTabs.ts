import type { BootstrapState, SettingsTabId, SettingsUpdate } from "../../types/bridge";
import type { LocaleKey } from "../../i18n/keys";

export const TAB_META: { id: SettingsTabId; labelKey: LocaleKey }[] = [
  { id: "general", labelKey: "TabGeneral" },
  { id: "providers", labelKey: "TabProviders" },
  { id: "notifications", labelKey: "TabNotifications" },
  { id: "menuBar", labelKey: "TabMenuBar" },
  { id: "menu", labelKey: "TabMenu" },
  { id: "usageSpend", labelKey: "TabUsageSpend" },
  { id: "advanced", labelKey: "TabAdvanced" },
  { id: "about", labelKey: "TabAbout" },
];

export interface TabProps {
  settings: BootstrapState["settings"];
  set: (p: SettingsUpdate) => void;
  saving: boolean;
}

export function isSettingsTab(value: string): value is SettingsTabId {
  return TAB_META.some((t) => t.id === value);
}

/** A Settings window request: a tab plus, on Providers, the provider to select. */
export interface SettingsTabRequest {
  tab: SettingsTabId;
  providerId: string | null;
}

/**
 * Parse a raw `open_settings_window` tab argument. `providers:<id>` selects a
 * provider on the Providers tab; the tab id still has to be one of `TAB_META`.
 * Unknown tabs return null so callers keep their own fallback.
 */
export function parseSettingsTabRequest(raw: string | null | undefined): SettingsTabRequest | null {
  if (!raw) return null;
  const separator = raw.indexOf(":");
  const tab = separator < 0 ? raw : raw.slice(0, separator);
  if (!isSettingsTab(tab)) return null;
  const providerId = separator < 0 ? "" : raw.slice(separator + 1).trim();
  return { tab, providerId: tab === "providers" && providerId ? providerId : null };
}

/** The `open_settings_window` argument that opens Settings > Providers on one provider. */
export function providersTabRequest(providerId: string): string {
  return `providers:${providerId}`;
}
