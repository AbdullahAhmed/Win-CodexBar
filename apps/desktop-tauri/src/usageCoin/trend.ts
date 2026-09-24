import type { RateWindowSnapshot } from "../types/bridge";

const WEEK_MS = 7 * 24 * 60 * 60 * 1000;
const STORAGE_KEY = "codexbar.usage-coin.weekly-history.v1";
const MAX_WINDOWS = 8;
const MAX_SAMPLES = 2048;

export interface UsageSample {
  at: number;
  usedPercent: number;
}

interface StoredWindow {
  account: string;
  resetsAt: number;
  samples: UsageSample[];
}

type TrendStorage = Pick<Storage, "getItem" | "setItem">;

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

export function weeklyResetMs(window: RateWindowSnapshot | null): number | null {
  if (!window?.resetsAt || !Number.isFinite(window.usedPercent)) return null;
  const reset = Date.parse(window.resetsAt);
  return Number.isFinite(reset) ? reset : null;
}

export function idealUsedPercent(at: number, reset: number): number {
  return clamp(((at - (reset - WEEK_MS)) / WEEK_MS) * 100, 0, 100);
}

function validSample(value: unknown, reset: number): value is UsageSample {
  if (!value || typeof value !== "object") return false;
  const sample = value as UsageSample;
  return Number.isFinite(sample.at)
    && sample.at >= reset - WEEK_MS
    && sample.at <= reset
    && Number.isFinite(sample.usedPercent)
    && sample.usedPercent >= 0
    && sample.usedPercent <= 100;
}

function readWindows(storage: TrendStorage): StoredWindow[] {
  try {
    const parsed: unknown = JSON.parse(storage.getItem(STORAGE_KEY) ?? "[]");
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((value): value is StoredWindow => {
      if (!value || typeof value !== "object") return false;
      const entry = value as StoredWindow;
      return typeof entry.account === "string"
        && Number.isFinite(entry.resetsAt)
        && Array.isArray(entry.samples);
    });
  } catch {
    return [];
  }
}

export function readWeeklySamples(
  storage: TrendStorage,
  account: string,
  reset: number,
): UsageSample[] {
  return readWindows(storage)
    .find((entry) => entry.account === account && entry.resetsAt === reset)
    ?.samples.filter((sample) => validSample(sample, reset))
    .sort((left, right) => left.at - right.at) ?? [];
}

/** Records only observed quota percentages; no prior usage is inferred. */
export function recordWeeklySample(
  storage: TrendStorage,
  account: string,
  reset: number,
  sample: UsageSample,
): UsageSample[] {
  if (!validSample(sample, reset)) return [];
  const windows = readWindows(storage)
    .filter((entry) => entry.resetsAt >= reset - MAX_WINDOWS * WEEK_MS);
  let current = windows.find((entry) => entry.account === account && entry.resetsAt === reset);
  if (!current) {
    current = { account, resetsAt: reset, samples: [] };
    windows.push(current);
  }
  current.samples = current.samples
    .filter((point) => validSample(point, reset) && point.at !== sample.at)
    .concat(sample)
    .sort((left, right) => left.at - right.at)
    .slice(-MAX_SAMPLES);
  try {
    storage.setItem(
      STORAGE_KEY,
      JSON.stringify(windows.sort((left, right) => left.resetsAt - right.resetsAt).slice(-MAX_WINDOWS)),
    );
  } catch {
    // Keep the in-memory samples even if WebView storage is unavailable.
  }
  return current.samples;
}

export function chartPoint(sample: UsageSample, reset: number): { x: number; y: number } {
  const elapsed = idealUsedPercent(sample.at, reset) / 100;
  return {
    x: 5 + elapsed * 66,
    y: 25 - sample.usedPercent * 0.21,
  };
}

export function actualPath(samples: UsageSample[], reset: number): string {
  const observed = samples
    .filter((sample) => validSample(sample, reset))
    .sort((left, right) => left.at - right.at);
  if (observed.length === 0) return "";
  return ["M5 25", ...observed.map((sample) => {
      const { x, y } = chartPoint(sample, reset);
      return `L${x.toFixed(1)} ${y.toFixed(1)}`;
    })].join(" ");
}

/** Extend the average burn rate from the latest observation, capped at exhaustion. */
export function projectedPath(sample: UsageSample | null, reset: number): string {
  if (!validSample(sample, reset)) return "";
  const elapsed = idealUsedPercent(sample.at, reset) / 100;
  if (elapsed <= 0 || elapsed >= 1) return "";
  const projectedUsed = sample.usedPercent / elapsed;
  const endpoint = projectedUsed >= 100 && sample.usedPercent > 0
    ? { at: reset - WEEK_MS + WEEK_MS * (elapsed * 100 / sample.usedPercent), usedPercent: 100 }
    : { at: reset, usedPercent: clamp(projectedUsed, 0, 100) };
  const from = chartPoint(sample, reset);
  const to = chartPoint(endpoint, reset);
  return `M${from.x.toFixed(1)} ${from.y.toFixed(1)} L${to.x.toFixed(1)} ${to.y.toFixed(1)}`;
}
