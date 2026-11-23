// The recipe editor's rules, apart from its page: how the text boxes become ingredient lines and steps, which food
// goes with which line, and how a line reads back once the server has read it.
import type { FoodRef, Recipe, RecipeInput, RecipeLine } from "./api/client";

/** Foods the cook picked, by the exact line each was picked for. Changing a line's text drops its pick, so the
 * line is matched by its name again. */
export type Picks = Record<string, FoodRef>;

/** A recipe as the editor holds it: every field as the text in its box. */
export type Draft = {
  title: string;
  subtitle: string;
  description: string;
  servings: string;
  /** One ingredient to a line. */
  ingredients: string;
  /** One step to a line. */
  steps: string;
  picks: Picks;
};

/** The non-blank lines of a text box, trimmed: one ingredient or one step each. */
export function linesOf(text: string): string[] {
  return text
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line !== "");
}

/** Where the editor starts: a stored recipe as it was typed, or empty boxes for a new one. A stored line keeps the
 * food it has, so saving again without touching a line never changes its food. */
export function draftOf(recipe: Recipe | null): Draft {
  if (!recipe)
    return {
      title: "",
      subtitle: "",
      description: "",
      servings: "4",
      ingredients: "",
      steps: "",
      picks: {},
    };
  const picks: Picks = {};
  for (const line of recipe.ingredients) if (line.food) picks[line.text.trim()] = line.food;
  return {
    title: recipe.title,
    subtitle: recipe.subtitle ?? "",
    description: recipe.description ?? "",
    servings: String(recipe.servings),
    ingredients: recipe.ingredients.map((line) => line.text).join("\n"),
    // A step takes one line in its box, so one written elsewhere over several lines is joined up.
    steps: recipe.steps.map((step) => step.replace(/\s*\n\s*/g, " ")).join("\n"),
    picks,
  };
}

/** The recipe to send: each ingredient line with the food picked for it, if one was. */
export function inputOf(draft: Draft): RecipeInput {
  return {
    title: draft.title.trim(),
    subtitle: draft.subtitle.trim() || null,
    description: draft.description.trim() || null,
    servings: Number(draft.servings),
    ingredients: linesOf(draft.ingredients).map((text) => ({
      text,
      food_id: draft.picks[text]?.id ?? null,
    })),
    steps: linesOf(draft.steps),
  };
}

const GLYPHS: [number, string][] = [
  [1 / 8, "⅛"],
  [1 / 4, "¼"],
  [1 / 3, "⅓"],
  [3 / 8, "⅜"],
  [1 / 2, "½"],
  [5 / 8, "⅝"],
  [2 / 3, "⅔"],
  [3 / 4, "¾"],
  [7 / 8, "⅞"],
];

/** A number in kitchen style: 1.5 is "1½", 0.25 is "¼", 2 is "2"; anything else to two decimal places. */
export function amountText(amount: number): string {
  const whole = Math.floor(amount);
  const rest = amount - whole;
  if (rest < 0.005) return String(whole);
  const glyph = GLYPHS.find(([value]) => Math.abs(value - rest) < 0.005);
  if (glyph) return `${whole || ""}${glyph[1]}`;
  return String(Math.round(amount * 100) / 100);
}

const SHORT: Record<string, string> = {
  teaspoon: "tsp",
  tablespoon: "TBSP",
  "fluid ounce": "fl oz",
  milliliter: "ml",
  liter: "L",
  ounce: "oz",
  pound: "lb",
  gram: "g",
  kilogram: "kg",
};

/** An amount and its unit as the house style writes them: "1½ cups", "2 TBSP", "3 cloves", and "2" for a bare
 * count. Nothing for a line with no amount. */
export function measureText(line: Pick<RecipeLine, "amount" | "unit">): string {
  if (line.amount == null) return "";
  const amount = amountText(line.amount);
  const unit = line.unit ?? "unit";
  if (unit === "unit") return amount;
  if (SHORT[unit]) return `${amount} ${SHORT[unit]}`;
  const plural = /(ch|x)$/.test(unit) ? `${unit}es` : `${unit}s`;
  return `${amount} ${line.amount > 1 ? plural : unit}`;
}

/** What mise read from a line, for the cook to check: "1 can · diced tomatoes · 14 oz; drained". */
export function readingText(line: RecipeLine): string {
  return [measureText(line), line.name, line.note].filter(Boolean).join(" · ");
}
