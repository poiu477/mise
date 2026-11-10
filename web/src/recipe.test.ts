import { describe, expect, it } from "vitest";
import type { Variant } from "./api/client";
import { allergens, difficulty, minutes, servingsText } from "./recipe";

describe("minutes", () => {
  it("reads hours and minutes", () => {
    expect(minutes("PT25M")).toBe(25);
    expect(minutes("PT1H5M")).toBe(65);
    expect(minutes("PT2H")).toBe(120);
  });
  it("has nothing for a missing or empty duration", () => {
    expect(minutes(undefined)).toBeUndefined();
    expect(minutes("PT0M")).toBeUndefined();
    expect(minutes("soon")).toBeUndefined();
  });
});

describe("difficulty", () => {
  it("names the published levels and nothing else", () => {
    expect(difficulty(1)).toBe("Easy");
    expect(difficulty(3)).toBe("Hard");
    expect(difficulty(7)).toBe("");
    expect(difficulty("2")).toBe("");
  });
});

describe("allergens", () => {
  it("lists each allergen once, sorted", () => {
    const v = {
      ingredients: [
        { name: "Butter", allergens: ["Milk"], scaled: false },
        { name: "Bread", allergens: ["Wheat", "Milk"], scaled: false },
      ],
    } as Variant;
    expect(allergens(v)).toEqual(["Milk", "Wheat"]);
  });
});

describe("servingsText", () => {
  it("counts servings in words", () => {
    expect(servingsText(1)).toBe("1 serving");
    expect(servingsText(6)).toBe("6 servings");
  });
});
