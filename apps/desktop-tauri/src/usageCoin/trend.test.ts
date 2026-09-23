import { describe, expect, it } from "vitest";
import {
  actualPath,
  chartPoint,
  idealUsedPercent,
  projectedPath,
  readWeeklySamples,
  recordWeeklySample,
} from "./trend";

const DAY = 24 * 60 * 60 * 1000;
const RESET = Date.parse("2026-10-01T00:00:00Z");
const START = RESET - 7 * DAY;

function memoryStorage() {
  const values = new Map<string, string>();
  return {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => { values.set(key, value); },
  };
}

describe("weekly usage trend", () => {
  it("maps day one zero usage to the lower left and day seven 100% to the right middle", () => {
    expect(chartPoint({ at: START, usedPercent: 0 }, RESET)).toEqual({ x: 7, y: 44 });
    expect(chartPoint({ at: RESET, usedPercent: 100 }, RESET)).toEqual({ x: 69, y: 26 });
    expect(idealUsedPercent(START + 3.5 * DAY, RESET)).toBe(50);
    expect(60 > idealUsedPercent(START + 3.5 * DAY, RESET)).toBe(true);
  });

  it("keeps real observations across reopen and separates accounts and reset weeks", () => {
    const storage = memoryStorage();
    const first = { at: START + DAY, usedPercent: 12 };
    const second = { at: START + 2 * DAY, usedPercent: 25 };
    expect(recordWeeklySample(storage, "alice", RESET, first)).toEqual([first]);
    expect(recordWeeklySample(storage, "alice", RESET, second)).toEqual([first, second]);
    expect(recordWeeklySample(storage, "bob", RESET, { at: second.at, usedPercent: 70 }))
      .toHaveLength(1);
    expect(readWeeklySamples(storage, "alice", RESET)).toEqual([first, second]);
    expect(recordWeeklySample(storage, "alice", RESET, second)).toEqual([first, second]);
    expect(recordWeeklySample(storage, "alice", RESET + 7 * DAY, {
      at: RESET + DAY,
      usedPercent: 5,
    })).toHaveLength(1);
  });

  it("draws only observed points and rejects samples outside this quota week", () => {
    const storage = memoryStorage();
    const first = { at: START + DAY, usedPercent: 12 };
    const second = { at: START + 2 * DAY, usedPercent: 25 };
    expect(recordWeeklySample(storage, "alice", RESET, { at: START - 1, usedPercent: 99 }))
      .toEqual([]);
    expect(actualPath([first, second], RESET)).toBe("M15.9 41.8 L24.7 39.5");
    expect(actualPath([first, { at: RESET + 1, usedPercent: 100 }], RESET))
      .toBe("M15.9 41.8");
  });

  it("projects a fast burn to early exhaustion and a slow burn to day seven", () => {
    const at = START + 3.5 * DAY;
    expect(projectedPath({ at, usedPercent: 60 }, RESET))
      .toBe("M38.0 33.2 L58.7 26.0");
    expect(projectedPath({ at, usedPercent: 40 }, RESET))
      .toBe("M38.0 36.8 L69.0 29.6");
  });
});
