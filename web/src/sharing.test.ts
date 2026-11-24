import { describe, expect, it } from "vitest";
import { allowedLevels, sharedAs } from "./sharing";

describe("allowedLevels", () => {
  it("offers the levels up to the install's cap", () => {
    expect(allowedLevels("install").map((l) => l.value)).toEqual([
      "private",
      "partners",
      "install",
    ]);
    expect(allowedLevels("partners").map((l) => l.value)).toEqual(["private", "partners"]);
    expect(allowedLevels("private").map((l) => l.value)).toEqual(["private"]);
  });
});

describe("sharedAs", () => {
  it("names each level", () => {
    expect(sharedAs("private")).toBe("Private");
    expect(sharedAs("partners")).toBe("Shared with partners");
    expect(sharedAs("install")).toBe("Shared with every household");
  });
});
