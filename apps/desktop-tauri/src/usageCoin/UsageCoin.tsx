import { useCallback, useEffect, useState, type MouseEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useProviders } from "../hooks/useProviders";
import { refreshProvidersIfStale } from "../lib/tauri";
import type { RateWindowSnapshot } from "../types/bridge";
import {
  actualPath,
  chartPoint,
  idealUsedPercent,
  projectedPath,
  readWeeklySamples,
  recordWeeklySample,
  weeklyResetMs,
  type UsageSample,
} from "./trend";
import "./UsageCoin.css";

export function countdown(resetsAt: string | null, now: number): string {
  if (!resetsAt) return "Reset unknown";
  const remaining = Date.parse(resetsAt) - now;
  if (!Number.isFinite(remaining)) return "Reset unknown";
  if (remaining <= 0) return "Reset due";
  const totalHours = Math.ceil(remaining / 3_600_000);
  const days = Math.floor(totalHours / 24);
  const hours = totalHours % 24;
  return days > 0 ? `${days}d ${hours}h` : `${hours}h`;
}

function percentage(window: RateWindowSnapshot | null): string {
  if (!window || !Number.isFinite(window.remainingPercent)) return "--%";
  return `${Math.round(Math.max(0, Math.min(100, window.remainingPercent)))}%`;
}

export default function UsageCoin() {
  const { providers } = useProviders({ refreshOnMount: false });
  const codex = providers.find((provider) => provider.providerId === "codex");
  const weekly = codex?.secondary ?? null;
  const [now, setNow] = useState(Date.now());
  const [topmost, setTopmost] = useState(true);
  const [notice, setNotice] = useState("");
  const [samples, setSamples] = useState<UsageSample[]>([]);

  useEffect(() => {
    const reset = weeklyResetMs(weekly);
    const observedAt = Date.parse(codex?.updatedAt ?? "");
    if (!codex || !weekly || reset === null) {
      setSamples([]);
      return;
    }
    const account = codex.accountEmail
      ?? codex.cost?.accountId
      ?? codex.accountOrganization
      ?? "default";
    try {
      const saved = readWeeklySamples(window.localStorage, account, reset);
      if (codex.errorState === "ready" && Number.isFinite(observedAt)) {
        const sample = { at: observedAt, usedPercent: weekly.usedPercent };
        const recorded = recordWeeklySample(window.localStorage, account, reset, sample);
        setSamples(recorded.length > 0 ? recorded : saved);
      } else {
        setSamples(saved);
      }
    } catch {
      setSamples(Number.isFinite(observedAt)
        ? [{ at: observedAt, usedPercent: weekly.usedPercent }]
        : []);
    }
  }, [
    codex?.updatedAt,
    codex?.errorState,
    codex?.accountEmail,
    codex?.cost?.accountId,
    codex?.accountOrganization,
    weekly?.resetsAt,
    weekly?.usedPercent,
  ]);

  useEffect(() => {
    document.body.classList.add("usage-coin-window");
    void invoke<boolean>("get_usage_coin_topmost").then(setTopmost).catch(() => {});
    void refreshProvidersIfStale().catch(() => {});
    const timer = window.setInterval(() => setNow(Date.now()), 60_000);
    return () => {
      document.body.classList.remove("usage-coin-window");
      window.clearInterval(timer);
    };
  }, []);

  useEffect(() => {
    if (!notice) return;
    const timer = window.setTimeout(() => setNotice(""), 1600);
    return () => window.clearTimeout(timer);
  }, [notice]);

  const drag = useCallback((event: MouseEvent<HTMLDivElement>) => {
    if (event.button === 0) {
      void getCurrentWindow().startDragging().catch(() => {});
    }
  }, []);

  const toggleTopmost = useCallback(() => {
    void invoke<boolean>("toggle_usage_coin_topmost")
      .then((enabled) => {
        setTopmost(enabled);
        setNotice(enabled ? "Always on top" : "Normal window");
      })
      .catch(() => setNotice("Could not change pin"));
  }, []);

  const reset = weekly
    ? countdown(weekly.resetsAt, now)
    : codex?.errorState === "needsAuthentication" || codex?.errorState === "expiredSession"
      ? "Sign in to Codex"
      : "Waiting for Codex";
  const resetMs = weeklyResetMs(weekly);
  const aheadOfPace = resetMs !== null && weekly !== null
    && weekly.usedPercent > idealUsedPercent(now, resetMs);
  const recordedSamples = resetMs === null
    ? []
    : samples.filter((sample) => sample.at <= now);
  const latestPoint = resetMs !== null && recordedSamples.length > 0
    ? chartPoint(recordedSamples[recordedSamples.length - 1], resetMs)
    : null;
  const forecast = resetMs !== null && recordedSamples.length > 0
    ? projectedPath(recordedSamples[recordedSamples.length - 1], resetMs)
    : "";
  const label = weekly
    ? `Codex weekly usage: ${percentage(weekly)} remaining, ${reset} until reset. Green is ideal usage; ${aheadOfPace ? "red" : "blue"} is actual usage with a dashed projection. Right-click to ${topmost ? "turn off" : "turn on"} always on top.`
    : "Codex weekly usage unavailable. Right-click to toggle always on top.";

  return (
    <div
      className="usage-coin"
      role="button"
      tabIndex={0}
      aria-label={label}
      title={label}
      onMouseDown={drag}
      onContextMenu={(event) => {
        event.preventDefault();
        toggleTopmost();
      }}
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          toggleTopmost();
        }
      }}
    >
      <div className="usage-coin__main">
        {resetMs !== null && (
          <svg className="usage-coin__trend" viewBox="0 0 76 52" aria-hidden="true">
            <path className="usage-coin__trend-ideal" d="M7 44 L69 26" />
            {recordedSamples.length > 1 && (
              <path
                className={`usage-coin__trend-actual${aheadOfPace ? " usage-coin__trend-actual--ahead" : ""}`}
                d={actualPath(recordedSamples, resetMs)}
              />
            )}
            {forecast && (
              <path
                className={`usage-coin__trend-projection${aheadOfPace ? " usage-coin__trend-projection--ahead" : ""}`}
                d={forecast}
              />
            )}
            {latestPoint && (
              <circle
                className={`usage-coin__trend-dot${aheadOfPace ? " usage-coin__trend-dot--ahead" : ""}`}
                cx={latestPoint.x}
                cy={latestPoint.y}
                r="1.8"
              />
            )}
          </svg>
        )}
        <span className="usage-coin__percent">{percentage(weekly)}</span>
      </div>
      <div className={`usage-coin__footer${weekly ? "" : " usage-coin__footer--status"}`}>
        {reset}
      </div>
      {notice && <div className="usage-coin__notice" role="status">{notice}</div>}
    </div>
  );
}
