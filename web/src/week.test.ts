import { describe, expect, it } from "vitest";
import { addWeeks, mondayOf, weekLabel, weekOf } from "./week";

describe("weeks", () => {
  it("finds the Monday of any day", () => {
    expect(mondayOf(new Date(2026, 10, 2))).toBe("2026-11-02"); // a Monday
    expect(mondayOf(new Date(2026, 10, 8))).toBe("2026-11-02"); // a Sunday
    expect(mondayOf(new Date(2026, 0, 1))).toBe("2025-12-29"); // across a year
    expect(weekOf("2026-11-05")).toBe("2026-11-02");
    expect(weekOf("2026-11-02")).toBe("2026-11-02");
  });

  it("moves by whole weeks across months", () => {
    expect(addWeeks("2026-10-26", 1)).toBe("2026-11-02");
    expect(addWeeks("2026-11-02", -1)).toBe("2026-10-26");
  });

  it("names nearby weeks and dates the rest", () => {
    const today = new Date(2026, 10, 4);
    expect(weekLabel("2026-11-02", today)).toBe("This week");
    expect(weekLabel("2026-11-09", today)).toBe("Next week");
    expect(weekLabel("2026-10-26", today)).toBe("Last week");
    expect(weekLabel("2026-11-16", today)).toBe("Nov 16 – 22");
    expect(weekLabel("2026-11-30", today)).toBe("Nov 30 – Dec 6");
  });
});
