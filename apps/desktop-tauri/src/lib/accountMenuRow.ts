import type { LocaleKey } from "../i18n/keys";
import type { ProviderUsageSnapshot } from "../types/bridge";
import { providersTabRequest } from "../surfaces/settings/settingsTabs";
import { hasSuccessfulClaudeCliQuota } from "./claudeAccountActions";

/** Providers whose account manager lives in Settings > Providers > that provider. */
const ACCOUNT_MANAGER_PROVIDERS = new Set(["codex", "claude", "grok"]);

export interface AccountMenuAction {
  labelKey: Extract<LocaleKey, "TrayMenuSwitchAccount" | "AddAccountEllipsis">;
  /** `open_settings_window` argument that selects the provider. */
  settingsTab: string;
}

/**
 * The tray footer account row (Mac `MenuDescriptor` "Switch Account..." or
 * "Add Account..."). It replaces the account menus that used to sit inside
 * the card, and deep-links to the provider's existing Settings account UI.
 */
export function accountMenuAction(
  provider: ProviderUsageSnapshot | null,
): AccountMenuAction | null {
  if (!provider || !ACCOUNT_MANAGER_PROVIDERS.has(provider.providerId)) return null;
  const hasAccount =
    Boolean(provider.accountEmail?.trim()) || hasSuccessfulClaudeCliQuota(provider);
  return {
    labelKey: hasAccount ? "TrayMenuSwitchAccount" : "AddAccountEllipsis",
    settingsTab: providersTabRequest(provider.providerId),
  };
}
