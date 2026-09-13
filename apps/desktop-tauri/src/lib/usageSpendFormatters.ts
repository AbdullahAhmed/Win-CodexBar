const currencyFormatters = new Map<string, Intl.NumberFormat>();

export function formatUsageSpendUsd(
  value: number | null | undefined,
  currency: string,
): string {
  if (value == null || !Number.isFinite(value)) return "—";
  const code = currency || "USD";
  try {
    let formatter = currencyFormatters.get(code);
    if (!formatter) {
      formatter = new Intl.NumberFormat(undefined, {
        style: "currency",
        currency: code,
        maximumFractionDigits: 2,
      });
      currencyFormatters.set(code, formatter);
    }
    return formatter.format(value);
  } catch {
    return `$${value.toFixed(2)}`;
  }
}

export function formatUsageSpendMetric(
  cost: number | null | undefined,
  tokens: number | null | undefined,
  currency: string,
  tokenLabel: string,
): string {
  const parts: string[] = [];
  if (cost != null && Number.isFinite(cost)) {
    parts.push(formatUsageSpendUsd(cost, currency));
  }
  if (tokens != null && Number.isFinite(tokens)) {
    parts.push(`${Math.max(0, tokens).toLocaleString()} ${tokenLabel}`);
  }
  return parts.length > 0 ? parts.join(" · ") : "—";
}

export function formatUsageSpendTokens(
  value: number | null | undefined,
  tokenLabel: string,
  unknownLabel: string,
): string {
  if (value == null || !Number.isFinite(value) || value < 0) {
    return unknownLabel;
  }
  return `${value.toLocaleString()} ${tokenLabel}`;
}
