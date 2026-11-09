// The ways a shopping list can read weights, with the words the account page and the list use for each.
import type { Weights } from "./api/client";

/** The ways a shopping list can read weights, as the household picks them. */
export const WEIGHTS: { value: Weights; label: string; short: string }[] = [
  { value: "lb_oz", label: "Pounds and ounces", short: "lb + oz" },
  { value: "oz", label: "Ounces", short: "oz" },
  { value: "lb", label: "Pounds", short: "lb" },
  { value: "metric", label: "Grams and kilograms", short: "g / kg" },
];

/** A weight setting in a few characters, as a list item's unit picker names the usual one: "lb + oz", "g / kg". */
export const shortWeight = (w: Weights) => WEIGHTS.find((x) => x.value === w)?.short ?? w;
