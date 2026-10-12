import { describe, expect, it } from "vitest";
import type { LocaleKey } from "../i18n/keys";
import { localizeProviderLabel, localizeWindowLabel } from "./windowLabels";

const t = (key: LocaleKey) => `t:${key}`;

describe("localizeWindowLabel", () => {
  it("translates the generic provider window labels", () => {
    expect(localizeWindowLabel("Session", t)).toBe("t:ProviderSessionLabel");
    expect(localizeWindowLabel("Weekly", t)).toBe("t:ProviderWeeklyLabel");
    expect(localizeWindowLabel("Monthly", t)).toBe("t:ProviderMonthly");
  });

  it("translates the generic labels providers declare themselves", () => {
    expect(localizeWindowLabel("Credits", t)).toBe("t:CreditsLabel");
    expect(localizeWindowLabel("Personal budget", t)).toBe("t:WindowLabelPersonalBudget");
    expect(localizeWindowLabel("5-hour", t)).toBe("t:WindowLabelHours".replace("{}", "5"));
    expect(localizeProviderLabel("API key limit", t)).toBe("t:WindowLabelApiKeyLimit");
  });

  it("fills the duration templates", () => {
    const ru = (key: LocaleKey) =>
      ({ WindowLabelHours: "{} ч", WindowLabelHourQuota: "Квота на {} ч", WindowLabelDays: "{} дн.", ProviderSessionLabel: "Сессия" })[
        key as string
      ] ?? key;
    expect(localizeProviderLabel("5-Hour", ru)).toBe("5 ч");
    expect(localizeProviderLabel("5 hours", ru)).toBe("5 ч");
    expect(localizeProviderLabel("4-hour quota", ru)).toBe("Квота на 4 ч");
    expect(localizeProviderLabel("7-Day", ru)).toBe("7 дн.");
    expect(localizeProviderLabel("Session (5h)", ru)).toBe("Сессия (5h)");
  });

  it("keeps brand, model and plan names as declared", () => {
    expect(localizeWindowLabel("Gemini Pro", t)).toBe("Gemini Pro");
    expect(localizeWindowLabel("gpt-5.4", t)).toBe("gpt-5.4");
    expect(localizeWindowLabel(undefined, t)).toBe("");
  });
});
