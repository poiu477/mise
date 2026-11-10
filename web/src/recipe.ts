// A version's details as people read them: how long it takes, how hard it is, what it contains.
import type { Variant } from "./api/client";

/** Minutes in an ISO 8601 duration ("PT1H5M" → 65), or nothing when it has none. */
export function minutes(iso: unknown): number | undefined {
  if (typeof iso !== "string") return undefined;
  const m = /PT(?:(\d+)H)?(?:(\d+)M)?/.exec(iso);
  if (!m) return undefined;
  const total = Number(m[1] ?? 0) * 60 + Number(m[2] ?? 0);
  return total > 0 ? total : undefined;
}

/** The published difficulty, 1 to 3, in words ("Easy"), or nothing when it has none. */
export function difficulty(d: unknown): string {
  return typeof d === "number" ? (["", "Easy", "Medium", "Hard"][d] ?? "") : "";
}

/** Every allergen in the version's ingredients, once each, sorted. */
export function allergens(v: Variant): string[] {
  return [...new Set(v.ingredients.flatMap((i) => i.allergens))].sort();
}

/** "1 serving", "2 servings", "2.5 servings". */
export function servingsText(n: number): string {
  return `${n} ${n === 1 ? "serving" : "servings"}`;
}
