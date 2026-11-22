import { describe, expect, it } from "vitest";
import type { Ingredient } from "./api/client";
import {
  clock,
  duration,
  ingredientsIn,
  stem,
  stepPieces,
  timeUp,
  type Piece,
  type StepTime,
} from "./kitchen";

/** A time as the API sends it. */
const time = (text: string, seconds: number, until: number | null = null): StepTime => ({
  text,
  seconds,
  until_seconds: until,
});

/** Pieces as a person reads them: text as it is, a time in brackets. */
const read = (paragraphs: Piece[][]) =>
  paragraphs.map((p) => p.map((x) => ("time" in x ? `[${x.time.text}]` : x.text)).join(""));

describe("stepPieces", () => {
  it("cuts each time out where the text has it, in order", () => {
    const text =
      "Add the onion and cook until soft, 5-6 minutes. Add the garlic; cook 1 minute more.";
    const times = [time("5-6 minutes", 300, 360), time("1 minute", 60)];
    const pieces = stepPieces(text, times);
    expect(read(pieces)).toEqual([
      "Add the onion and cook until soft, [5-6 minutes]. Add the garlic; cook [1 minute] more.",
    ]);
    const found = pieces[0].filter((x) => "time" in x);
    expect(found.map((x) => ("time" in x ? x.index : -1))).toEqual([0, 1]);
  });

  it("finds the same words twice when the step says them twice", () => {
    const text = "Sear 2 minutes. Flip and sear 2 minutes more.";
    expect(read(stepPieces(text, [time("2 minutes", 120), time("2 minutes", 120)]))).toEqual([
      "Sear [2 minutes]. Flip and sear [2 minutes] more.",
    ]);
  });

  it("tidies the text, splits bullets into paragraphs and carries times across them", () => {
    const text = "• Boil the eggs,  8 minutes.Peel them. • Toast the bread, 2-3 minutes.";
    expect(read(stepPieces(text, [time("8 minutes", 480), time("2-3 minutes", 120, 180)]))).toEqual(
      ["Boil the eggs, [8 minutes]. Peel them.", "Toast the bread, [2-3 minutes]."],
    );
  });

  it("leaves out a time the text doesn't hold, and reads a step with none as it is", () => {
    expect(read(stepPieces("Stir well.", [time("9 minutes", 540)]))).toEqual(["Stir well."]);
    expect(read(stepPieces("Serve.", []))).toEqual(["Serve."]);
  });
});

/** An ingredient line with only what matching reads. */
const line = (name: string): Ingredient => ({
  name,
  amount: "1",
  scaled: false,
  allergens: [],
  food: null,
  food_id: null,
  image: null,
  note: null,
  shipped: true,
});

describe("ingredientsIn", () => {
  const pasta = [
    "Spaghetti",
    "Crushed Tomatoes",
    "Garlic",
    "Yellow Onion",
    "Red Pepper Flakes",
    "Olive Oil",
    "Salt",
  ].map(line);
  const names = (text: string, from = pasta) => ingredientsIn(text, from).map((i) => i.name);

  it("finds an ingredient by its whole name or its last word, in the recipe's order", () => {
    expect(
      names("Heat 1 TBSP olive oil. Add the onion, then the garlic and red pepper flakes."),
    ).toEqual(["Garlic", "Yellow Onion", "Red Pepper Flakes", "Olive Oil"]);
    expect(names("Stir in the crushed tomatoes and a pinch of salt.")).toEqual([
      "Crushed Tomatoes",
      "Salt",
    ]);
  });

  it("folds plurals both ways", () => {
    expect(names("Halve the tomato.")).toEqual(["Crushed Tomatoes"]);
    expect(names("Chop the onions.")).toEqual(["Yellow Onion"]);
    expect(stem("Berries")).toBe("berry");
    expect(stem("glass")).toBe("glass");
  });

  it("knows a food by the words before its form", () => {
    const chicken = ["Chicken Breasts", "Chicken Stock Concentrate", "Taco Spice Blend"].map(line);
    expect(names("Pat the chicken dry.", chicken)).toEqual(["Chicken Breasts"]);
    expect(names("Stir in the stock concentrate.", chicken)).toEqual(["Chicken Stock Concentrate"]);
    expect(names("Add the beans and taco spice.", chicken)).toEqual(["Taco Spice Blend"]);
  });

  it("matches whole words only", () => {
    expect(names("Bring a pot of salted water to a boil.")).toEqual([]);
    expect(names("Toss with the oils.", pasta)).toEqual(["Olive Oil"]);
  });
});

describe("clock", () => {
  it("counts down in minutes and seconds, rounding up", () => {
    expect(clock(300_000)).toBe("5:00");
    expect(clock(297_001)).toBe("4:58");
    expect(clock(297_000)).toBe("4:57");
    expect(clock(0)).toBe("0:00");
    expect(clock(-5000)).toBe("0:00");
  });
  it("shows hours when there are some", () => {
    expect(clock(3_900_000)).toBe("1:05:00");
  });
});

describe("duration", () => {
  it("says a number of seconds in words", () => {
    expect(duration(30)).toBe("30 seconds");
    expect(duration(60)).toBe("1 minute");
    expect(duration(150)).toBe("2 minutes 30 seconds");
    expect(duration(4500)).toBe("1 hour 15 minutes");
  });
});

describe("timeUp", () => {
  it("says a single time is up", () => {
    expect(timeUp(time("1 minute", 60))).toBe("1 minute is up.");
    expect(timeUp(time("30 seconds", 30))).toBe("30 seconds are up.");
  });
  it("says to check a range at its lower end, and how much longer it allows", () => {
    expect(timeUp(time("8-10 minutes", 480, 600))).toBe(
      "8 minutes are up. Check it: the step allows up to 2 minutes more.",
    );
    expect(timeUp(time("45 minutes to 1 hour", 2700, 3600))).toBe(
      "45 minutes are up. Check it: the step allows up to 15 minutes more.",
    );
  });
});
