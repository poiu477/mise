import { describe, expect, it } from "vitest";
import type { Recipe, RecipeLine } from "./api/client";
import { amountText, draftOf, inputOf, linesOf, measureText, readingText } from "./editor";

const line = (over: Partial<RecipeLine>): RecipeLine => ({
  text: "",
  name: "",
  amount: null,
  unit: null,
  note: null,
  food: null,
  ...over,
});

const garlic = { id: "0190f4a2-0000-7000-8000-000000000001", name: "Garlic" };
const parmesan = { id: "0190f4a2-0000-7000-8000-000000000002", name: "Parmesan" };

const stored: Recipe = {
  id: "0190f4a2-0000-7000-8000-00000000000a",
  dish_slug: "garlic-bread",
  visibility: "partners",
  title: "Garlic Bread",
  subtitle: null,
  description: "Crisp outside, soft inside.",
  servings: 4,
  ingredients: [
    line({
      text: "4 cloves garlic, minced",
      name: "garlic",
      amount: 4,
      unit: "clove",
      food: garlic,
    }),
    line({ text: "a handful of hard cheese", name: "a handful of hard cheese", food: parmesan }),
  ],
  steps: ["Mix the garlic\ninto the butter.", "Bake until golden."],
  image: null,
};

describe("linesOf", () => {
  it("gives one trimmed line for each line with something on it", () => {
    expect(linesOf("  2 eggs \n\n1 cup flour\n   \n")).toEqual(["2 eggs", "1 cup flour"]);
    expect(linesOf("")).toEqual([]);
  });
});

describe("draftOf", () => {
  it("starts a new recipe with empty boxes, written for 4", () => {
    const draft = draftOf(null);
    expect(draft).toMatchObject({ title: "", servings: "4", ingredients: "", steps: "" });
    expect(draft.picks).toEqual({});
  });

  it("shows a stored recipe as typed, one ingredient and one step to a line", () => {
    const draft = draftOf(stored);
    expect(draft.ingredients).toBe("4 cloves garlic, minced\na handful of hard cheese");
    expect(draft.steps).toBe("Mix the garlic into the butter.\nBake until golden.");
    expect(draft.subtitle).toBe("");
    expect(draft.description).toBe("Crisp outside, soft inside.");
  });

  it("keeps each stored line's food as its pick", () => {
    expect(draftOf(stored).picks).toEqual({
      "4 cloves garlic, minced": garlic,
      "a handful of hard cheese": parmesan,
    });
  });
});

describe("inputOf", () => {
  it("sends a stored recipe back unchanged, foods included", () => {
    expect(inputOf(draftOf(stored))).toEqual({
      title: "Garlic Bread",
      subtitle: null,
      description: "Crisp outside, soft inside.",
      servings: 4,
      ingredients: [
        { text: "4 cloves garlic, minced", food_id: garlic.id },
        { text: "a handful of hard cheese", food_id: parmesan.id },
      ],
      steps: ["Mix the garlic into the butter.", "Bake until golden."],
      visibility: "partners",
    });
  });

  it("leaves a new recipe's visibility to the household's default", () => {
    expect(draftOf(null).visibility).toBeNull();
    expect(inputOf(draftOf(null)).visibility).toBeNull();
  });

  it("drops a pick when its line's text changes, so the line is matched by name again", () => {
    const draft = draftOf(stored);
    draft.ingredients = "5 cloves garlic, minced\na handful of hard cheese";
    expect(inputOf(draft).ingredients).toEqual([
      { text: "5 cloves garlic, minced", food_id: null },
      { text: "a handful of hard cheese", food_id: parmesan.id },
    ]);
  });

  it("drops blank lines and trims what is sent", () => {
    const draft = { ...draftOf(null), title: "  Toast ", ingredients: "\n 2 slices bread \n\n" };
    draft.steps = "Toast the bread.\n\n";
    expect(inputOf(draft)).toMatchObject({
      title: "Toast",
      ingredients: [{ text: "2 slices bread", food_id: null }],
      steps: ["Toast the bread."],
    });
  });
});

describe("amountText", () => {
  it("writes numbers in kitchen style", () => {
    expect(amountText(2)).toBe("2");
    expect(amountText(1.5)).toBe("1½");
    expect(amountText(0.25)).toBe("¼");
    expect(amountText(1 / 3)).toBe("⅓");
    expect(amountText(2.75)).toBe("2¾");
    expect(amountText(1.1)).toBe("1.1");
    expect(amountText(250)).toBe("250");
  });
});

describe("measureText", () => {
  it("writes an amount and its unit as the dish page does", () => {
    expect(measureText({ amount: 1.5, unit: "cup" })).toBe("1½ cups");
    expect(measureText({ amount: 1, unit: "cup" })).toBe("1 cup");
    expect(measureText({ amount: 2, unit: "tablespoon" })).toBe("2 TBSP");
    expect(measureText({ amount: 8, unit: "ounce" })).toBe("8 oz");
    expect(measureText({ amount: 3, unit: "clove" })).toBe("3 cloves");
    expect(measureText({ amount: 2, unit: "bunch" })).toBe("2 bunches");
    expect(measureText({ amount: 0.5, unit: "pinch" })).toBe("½ pinch");
  });

  it("writes a bare count as its number, and nothing for a line with no amount", () => {
    expect(measureText({ amount: 2, unit: "unit" })).toBe("2");
    expect(measureText({ amount: null, unit: null })).toBe("");
  });
});

describe("readingText", () => {
  it("joins what was read from a line", () => {
    const read = line({ amount: 1, unit: "can", name: "diced tomatoes", note: "14 oz; drained" });
    expect(readingText(read)).toBe("1 can · diced tomatoes · 14 oz; drained");
    expect(readingText(line({ name: "Salt", note: "to taste" }))).toBe("Salt · to taste");
  });
});
