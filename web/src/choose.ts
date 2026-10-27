// Choosing a version from a dish's option toggles. Only combinations some version actually has are offered.
import type { DishOption, VariantSummary } from "./api/client";

export type Selection = Record<string, string>;

const matches = (v: VariantSummary, sel: Selection, except?: string) =>
  Object.entries(sel).every(([k, val]) => k === except || v.attributes[k] === val);

/** The version for a selection: one with steps when there is one, newest first (the API's order). */
export function pick(variants: VariantSummary[], sel: Selection): VariantSummary | undefined {
  const hits = variants.filter((v) => matches(v, sel));
  return hits.find((v) => v.has_steps) ?? hits[0];
}

/** Whether choosing `value` for `key` keeps the other choices possible. */
export function available(variants: VariantSummary[], sel: Selection, key: string, value: string): boolean {
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
export function choose(variants: VariantSummary[], options: DishOption[], sel: Selection, key: string, value: string): Selection {
  const next = { ...sel, [key]: value };
  if (pick(variants, next)) return next;
  const withIt = variants.filter((v) => v.attributes[key] === value);
  return selectionOf(withIt.find((v) => v.has_steps) ?? withIt[0], options);
}
