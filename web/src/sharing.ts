// How far a household's own recipes are seen, with the words the account page and the editor use for each level.
import type { Visibility } from "./api/client";

/** The levels from least to most open, as a household picks them. */
export const LEVELS: { value: Visibility; label: string }[] = [
  { value: "private", label: "Only our household" },
  { value: "partners", label: "Our household and our partners" },
  { value: "install", label: "Every household here" },
];

/** The levels this install allows: those up to its cap. */
export const allowedLevels = (max: Visibility) =>
  LEVELS.slice(0, LEVELS.findIndex((l) => l.value === max) + 1);

/** A level as a recipe's page names it: "Private", "Shared with partners", "Shared with every household". */
export const sharedAs = (v: Visibility) =>
  v === "private"
    ? "Private"
    : v === "partners"
      ? "Shared with partners"
      : "Shared with every household";
