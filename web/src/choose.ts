// Choosing a version from a dish's option toggles. Only combinations some version actually has are offered.
import type { Dish, DishOption, VariantSummary } from "./api/client";

/** The chosen value of each of a dish's options, by option key: `{ portion: "double" }`. */
export type Selection = Record<string, string>;

/** Whether a version has every chosen value, ignoring the option `except`. */
const matches = (v: VariantSummary, sel: Selection, except?: string) =>
  Object.entries(sel).every(([k, val]) => k === except || v.attributes[k] === val);

/** The version for a selection: one with steps when there is one, newest first (the API's order). */
export function pick(variants: VariantSummary[], sel: Selection): VariantSummary | undefined {
  const hits = variants.filter((v) => matches(v, sel));
  return hits.find((v) => v.has_steps) ?? hits[0];
}

/** Whether choosing `value` for `key` keeps the other choices possible. */
export function available(
  variants: VariantSummary[],
  sel: Selection,
  key: string,
  value: string,
): boolean {
  return variants.some((v) => v.attributes[key] === value && matches(v, sel, key));
}

/** The option values of one version, limited to the dish's options. */
export function selectionOf(v: VariantSummary | undefined, options: DishOption[]): Selection {
  const sel: Selection = {};
  for (const o of options) {
    const val = v?.attributes[o.key];
    if (val !== undefined) sel[o.key] = val;
  }
  return sel;
}

/**
 * Choose `value` for `key`. When no version has it together with the other choices, the others follow the
 * newest version that has it, so a toggle never leads nowhere.
 */
export function choose(
  variants: VariantSummary[],
  options: DishOption[],
  sel: Selection,
  key: string,
  value: string,
): Selection {
  const next = { ...sel, [key]: value };
  if (pick(variants, next)) return next;
  const withIt = variants.filter((v) => v.attributes[key] === value);
  return selectionOf(withIt.find((v) => v.has_steps) ?? withIt[0], options);
}

/** Where a dish address starts: choices in the URL (?protein=Pork) win, the rest come from the pinned (?v=) or
 * default version. */
export function startingChoice(dish: Dish, params: URLSearchParams): Selection {
  const start =
    dish.variants.find((v) => v.id === params.get("v")) ??
    dish.variants.find((v) => v.id === dish.default_variant);
  const fromUrl = Object.fromEntries(
    dish.options.flatMap((o) => (params.get(o.key) ? [[o.key, params.get(o.key)!]] : [])),
  );
  const wanted = { ...selectionOf(start, dish.options), ...fromUrl };
  return pick(dish.variants, wanted) ? wanted : selectionOf(start, dish.options);
}

/** The version to show: the pinned one, else the one the options pick, else the default. */
export function versionOf(
  dish: Dish,
  sel: Selection,
  pinned: string | null,
): VariantSummary | undefined {
  return (
    dish.variants.find((v) => v.id === pinned) ??
    pick(dish.variants, sel) ??
    dish.variants.find((v) => v.id === dish.default_variant)
  );
}

/** The version a dish address opens on: the pinned one (?v=), else the one its options pick, else the default. */
export function versionFor(dish: Dish, params: URLSearchParams): VariantSummary | undefined {
  return versionOf(dish, startingChoice(dish, params), params.get("v"));
}

/** The serving count a version opens at when the address names none: 2 when published, else its first size. */
export function startingServings(v: VariantSummary): number {
  return (v.yields.includes(2) ? 2 : v.yields[0]) ?? 2;
}

/** A recipe card's address parameters for what a dish page shows: the page's own (options, a pinned ?v=), with
 * ?s= set to its servings, and ?v= added only when those alone would open another version. Coming back from the
 * card then finds the options as they were. */
export function cardParams(
  dish: Dish,
  params: URLSearchParams,
  version: string,
  servings: number,
): URLSearchParams {
  const card = new URLSearchParams(params);
  card.set("s", String(servings));
  if (versionFor(dish, card)?.id !== version) card.set("v", version);
  return card;
}
