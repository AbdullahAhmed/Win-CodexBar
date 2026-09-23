import { describe, expect, it } from "vitest";
import { countdown } from "./UsageCoin";

describe("usage coin reset countdown", () => {
  it("formats the weekly reset shown in the coin", () => {
    const now = Date.parse("2026-09-23T10:00:00Z");
    expect(countdown("2026-09-29T20:00:00Z", now)).toBe("6d 10h");
  });

  it("does not invent a reset when it is missing or elapsed", () => {
    const now = Date.parse("2026-09-23T10:00:00Z");
    expect(countdown(null, now)).toBe("Reset unknown");
    expect(countdown("2026-09-23T09:00:00Z", now)).toBe("Reset due");
  });
});
