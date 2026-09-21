import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { QuotaWindowHistory } from "./QuotaWindowHistory";
import type { QuotaWindowHistoryBridge } from "../types/bridge";

const history: QuotaWindowHistoryBridge = {
  providerId: "claude",
  accountScope: "account@example.com",
  historyCoverageEstablished: true,
  windows: [
    {
      offset: 0,
      start: "2026-09-14T12:00:00Z",
      end: "2026-09-21T12:00:00Z",
      totalTokens: 1234,
      totalCostUsd: 1.25,
      tokensAreComplete: true,
      costIsComplete: true,
      entryCount: 2,
      boundariesAreEstimated: true,
    },
  ],
};

describe("QuotaWindowHistory", () => {
  it("marks estimated boundaries instead of presenting exact totals", () => {
    render(<QuotaWindowHistory history={history} t={(key) => key} />);

    expect(screen.getByRole("row", { name: /1,234/ })).toHaveAttribute(
      "data-boundaries-estimated",
      "true",
    );
    expect(screen.getByText("~1,234")).toBeInTheDocument();
    expect(screen.getByText("~$1.25")).toBeInTheDocument();
    expect(screen.getByRole("table")).toHaveAttribute("data-history-coverage", "complete");
  });
});
