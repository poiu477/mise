import { describe, expect, it } from "vitest";
import type { Dish, DishOption, VariantSummary } from "./api/client";
import {
  available,
  choose,
  pick,
  selectionOf,
  startingServings,
  versionFor,
  type Selection,
} from "./choose";

const v = (id: string, attributes: Record<string, string>, has_steps = true): VariantSummary => ({
  id,
  title: id,
  yields: [2, 4],
  has_steps,
  attributes,
});
const opt = (key: string, values: string[]): DishOption => ({
  key,
  label: key,
  choices: values.map((value) => ({ value, label: value, variants: 1 })),
});

// Newest first, as the API orders them. Beef only exists as the Standard version.
const variants = [
  v("chicken-lime", { protein: "Chicken", version: "lime" }),
  v("chicken-std-nosteps", { protein: "Chicken", version: "std" }, false),
  v("chicken-std", { protein: "Chicken", version: "std" }),
  v("pork-lime", { protein: "Pork", version: "lime" }),
  v("pork-std", { protein: "Pork", version: "std" }),
  v("beef-std", { protein: "Beef", version: "std" }),
];
const options = [opt("protein", ["Chicken", "Pork", "Beef"]), opt("version", ["std", "lime"])];

describe("pick", () => {
  it("prefers a version with steps, newest first", () => {
    expect(pick(variants, { protein: "Chicken", version: "std" })?.id).toBe("chicken-std");
    expect(pick(variants, { protein: "Pork" })?.id).toBe("pork-lime");
  });
  it("finds nothing for a combination no version has", () => {
    expect(pick(variants, { protein: "Beef", version: "lime" })).toBeUndefined();
  });
});

describe("available", () => {
  it("only offers choices that fit the other selections", () => {
    const sel = { protein: "Chicken", version: "lime" };
    expect(available(variants, sel, "protein", "Pork")).toBe(true);
    expect(available(variants, sel, "protein", "Beef")).toBe(false);
    expect(available(variants, { protein: "Beef", version: "std" }, "version", "lime")).toBe(false);
  });
});

describe("choose", () => {
  it("keeps the other selections when the combination exists", () => {
    expect(
      choose(variants, options, { protein: "Chicken", version: "lime" }, "protein", "Pork"),
    ).toEqual({ protein: "Pork", version: "lime" });
  });
  it("moves the other selections when it does not, so a tap never leads nowhere", () => {
    const next = choose(
      variants,
      options,
      { protein: "Chicken", version: "lime" },
      "protein",
      "Beef",
    );
    expect(next).toEqual({ protein: "Beef", version: "std" });
    expect(pick(variants, next)?.id).toBe("beef-std");
  });
  it("selectionOf keeps only the dish's options", () => {
    expect(selectionOf(v("x", { protein: "Pork", portion: "double" }), [options[0]])).toEqual({
      protein: "Pork",
    });
  });
});

describe("versionFor", () => {
  const dish: Dish = {
    id: "dish",
    slug: "dish",
    title: "Dish",
    content_type: "recipe",
    default_variant: "pork-std",
    options,
    variants,
  };
  const at = (query: string) => versionFor(dish, new URLSearchParams(query))?.id;
  it("opens on the default version when the address names nothing", () => {
    expect(at("")).toBe("pork-std");
  });
  it("opens on a pinned version, whatever the options say", () => {
    expect(at("v=chicken-std-nosteps&protein=Beef")).toBe("chicken-std-nosteps");
  });
  it("lets options in the address override the default version's", () => {
    expect(at("version=lime")).toBe("pork-lime");
    expect(at("protein=Chicken")).toBe("chicken-std");
  });
  it("ignores a combination no version has", () => {
    expect(at("protein=Beef&version=lime")).toBe("pork-std");
  });
});

describe("startingServings", () => {
  it("prefers 2, else the first published size", () => {
    expect(startingServings(v("a", {}))).toBe(2);
    expect(startingServings({ ...v("b", {}), yields: [4, 6] })).toBe(4);
    expect(startingServings({ ...v("c", {}), yields: [] })).toBe(2);
  });
});

describe("every tap lands on a version with the tapped choice", () => {
  // Random dishes: up to 3 options with up to 4 values, 1-12 versions with random (often partial) attributes.
  let seed = 7;
  const rand = (n: number) => (seed = (seed * 1103515245 + 12345) % 2 ** 31) % n;
  for (let dish = 0; dish < 300; dish++) {
    const opts = Array.from({ length: 1 + rand(3) }, (_, i) =>
      opt(
        `k${i}`,
        Array.from({ length: 2 + rand(3) }, (_, j) => `v${j}`),
      ),
    );
    const vs = Array.from({ length: 1 + rand(12) }, (_, i) =>
      v(
        `d${dish}-${i}`,
        Object.fromEntries(opts.map((o) => [o.key, o.choices[rand(o.choices.length)].value])),
        rand(5) > 0,
      ),
    );
    it(`dish ${dish}`, () => {
      for (const start of vs) {
        const sel: Selection = selectionOf(start, opts);
        for (const o of opts) {
          for (const c of o.choices) {
            if (!vs.some((x) => x.attributes[o.key] === c.value)) continue; // the API never offers such a choice
            const shown = pick(vs, choose(vs, opts, sel, o.key, c.value));
            expect(shown, `${o.key}=${c.value} from ${JSON.stringify(sel)}`).toBeDefined();
            expect(shown!.attributes[o.key]).toBe(c.value);
          }
        }
      }
    });
  }
});
