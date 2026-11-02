import type { Weights } from "./api/client";

/** The ways a shopping list can read weights, as the household picks them. */
export const WEIGHTS: { value: Weights; label: string; short: string }[] = [
  { value: "lb_oz", label: "Pounds and ounces", short: "lb + oz" },
  { value: "oz", label: "Ounces", short: "oz" },
  { value: "lb", label: "Pounds", short: "lb" },
  { value: "metric", label: "Grams and kilograms", short: "g / kg" },
];

export const shortWeight = (w: Weights) => WEIGHTS.find((x) => x.value === w)?.short ?? w;
