import { describe, expect, it } from "vitest";
import type { ImportedRecipe } from "./api/client";
import { fileSummary, openAtOnce, opened } from "./imports";

const recipe = (over: Partial<ImportedRecipe> = {}): ImportedRecipe => ({
  variant_id: "0190f4a2-0000-7000-8000-00000000000a",
  dish_slug: "brown-butter-cornbread",
  title: "Brown Butter Cornbread",
  unchanged: false,
  photo_problem: null,
  ...over,
});

describe("opened", () => {
  it("opens the dish on the version the import made", () => {
    expect(opened(recipe())).toBe(
      "/dishes/brown-butter-cornbread?v=0190f4a2-0000-7000-8000-00000000000a",
    );
  });
});

describe("openAtOnce", () => {
  it("opens the one recipe an import made", () => {
    expect(openAtOnce([recipe()])).toBe(opened(recipe()));
  });

  it("lists what came of it when there is more to say", () => {
    expect(openAtOnce([recipe(), recipe({ dish_slug: "other" })])).toBeNull();
    expect(openAtOnce([recipe({ photo_problem: "its photo couldn't be kept" })])).toBeNull();
    expect(openAtOnce([recipe()], 1)).toBeNull();
    expect(openAtOnce([])).toBeNull();
  });
});

describe("fileSummary", () => {
  it("counts the recipes and names where they came from", () => {
    expect(fileSummary(["Paprika"], [recipe(), recipe()])).toBe("2 recipes from Paprika.");
    expect(fileSummary(["Mealie"], [recipe()])).toBe("1 recipe from Mealie.");
    expect(fileSummary(["Mealie", "Tandoor"], [recipe()])).toBe(
      "1 recipe from Mealie and Tandoor.",
    );
  });

  it("says how many were already imported and unchanged", () => {
    expect(fileSummary(["Paprika"], [recipe({ unchanged: true }), recipe()])).toBe(
      "2 recipes from Paprika, 1 of them already imported and unchanged.",
    );
    expect(
      fileSummary(["Paprika"], [recipe({ unchanged: true }), recipe({ unchanged: true })]),
    ).toBe("2 recipes from Paprika, all already imported and unchanged.");
    expect(fileSummary(["Paprika"], [recipe({ unchanged: true })])).toBe(
      "1 recipe from Paprika, already imported and unchanged.",
    );
  });
});
