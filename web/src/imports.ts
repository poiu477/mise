// What the import page does with what an import made, apart from the page: where a recipe opens, when to open it at
// once, and how a file's import is summed up.
import type { ImportedRecipe } from "./api/client";

/** Where an imported recipe opens: its dish, on the version the import made. */
export const opened = (r: ImportedRecipe) => `/dishes/${r.dish_slug}?v=${r.variant_id}`;

/** Where to go straight after an import: the one recipe it made, when that's all and nothing about it needs saying;
 * otherwise null, and the page lists what came of it. */
export function openAtOnce(recipes: ImportedRecipe[], skipped = 0): string | null {
  const [only] = recipes;
  return recipes.length === 1 && !only.photo_problem && skipped === 0 ? opened(only) : null;
}

/** A file's import in a sentence: "2 recipes from Paprika, 1 of them already imported and unchanged." */
export function fileSummary(formats: string[], recipes: ImportedRecipe[]): string {
  const count = recipes.length === 1 ? "1 recipe" : `${recipes.length} recipes`;
  const from = formats.length ? ` from ${formats.join(" and ")}` : "";
  const unchanged = recipes.filter((r) => r.unchanged).length;
  if (unchanged === 0) return `${count}${from}.`;
  if (unchanged === recipes.length)
    return `${count}${from}, ${recipes.length === 1 ? "already imported and unchanged" : "all already imported and unchanged"}.`;
  return `${count}${from}, ${unchanged} of them already imported and unchanged.`;
}
