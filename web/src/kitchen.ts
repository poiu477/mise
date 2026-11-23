// Kitchen mode's reading of a step: its text split round the times it asks for (each one a timer button), the
// ingredients it names, and how a timer reads as it counts down and when it rings.
import type { Ingredient, Step } from "./api/client";
import { tidyStep } from "./recipe";

/** A time a step asks for, as the API found it: "8-10 minutes", 480 seconds, up to 600. */
export type StepTime = Step["times"][number];

/** One run of a step's text, or a time in it that starts a timer. */
export type Piece = { text: string } | { time: StepTime; index: number };

/** A step's text as paragraphs ("•" bullets split it) of pieces, each time the API found cut out where the text
 * has it, in order: "Simmer, 8-10 minutes." → "Simmer, ", the time, ".". `index` counts the times through the
 * step. A time the text doesn't hold (it shouldn't happen) is left out rather than shown in the wrong place. */
export function stepPieces(text: string, times: StepTime[]): Piece[][] {
  const queue = times.map((time, index) => ({
    time: { ...time, text: tidyStep(time.text) },
    index,
  }));
  const paragraphs = text.split("•").map(tidyStep).filter(Boolean);
  return paragraphs.map((p) => {
    const pieces: Piece[] = [];
    let at = 0;
    while (queue.length > 0) {
      const found = p.indexOf(queue[0].time.text, at);
      if (found < 0) break;
      const next = queue.shift()!;
      if (found > at) pieces.push({ text: p.slice(at, found) });
      pieces.push({ time: times[next.index], index: next.index });
      at = found + next.time.text.length;
    }
    if (at < p.length) pieces.push({ text: p.slice(at) });
    return pieces;
  });
}

/** A word folded to its singular, lowercased, for comparing: "Tomatoes" → "tomato", "berries" → "berry". */
export function stem(word: string): string {
  const w = word.toLowerCase();
  if (w.length > 4 && w.endsWith("ies")) return w.slice(0, -3) + "y";
  if (/(o|ch|sh|ss|x)es$/.test(w)) return w.slice(0, -2);
  if (w.length > 2 && w.endsWith("s") && !w.endsWith("ss")) return w.slice(0, -1);
  return w;
}

/** Words of a text, folded for comparing. */
const words = (text: string) => (text.match(/[\p{L}\p{N}]+/gu) ?? []).map(stem);

/** Last words that only say what form a food comes in: a step calls Chicken Breasts "the chicken" and Taco Spice
 * Blend "the taco spice". */
const FORMS = new Set(
  [
    "breast",
    "thigh",
    "drumstick",
    "fillet",
    "filet",
    "cutlet",
    "tenderloin",
    "loin",
    "strip",
    "floret",
    "leaf",
    "sprig",
    "stalk",
    "wedge",
    "blend",
    "mix",
  ].map(stem),
);

/** The words a step may call an ingredient by: its whole name, its last word ("the onion" for Yellow Onion), and
 * for a name ending in a form, the words before it ("the chicken" for Chicken Breasts). */
function callNames(name: string): string[][] {
  const w = words(name);
  if (w.length === 0) return [];
  const names = [w, w.slice(-1)];
  if (w.length > 1 && FORMS.has(w[w.length - 1])) names.push(w.slice(0, -1));
  return names;
}

/** Whether `phrase` appears in `text` as whole words, in order. */
function contains(text: string[], phrase: string[]): boolean {
  for (let i = 0; i + phrase.length <= text.length; i++)
    if (phrase.every((p, j) => text[i + j] === p)) return true;
  return false;
}

/** The ingredients a step names, in the recipe's order: "Add the onion and cook until soft" names Yellow Onion.
 * Kitchen mode shows them beside the step with their amounts. */
export function ingredientsIn(text: string, ingredients: Ingredient[]): Ingredient[] {
  const step = words(text);
  return ingredients.filter((i) => callNames(i.name).some((n) => contains(step, n)));
}

/** A time left as a timer shows it, rounded up to the second: "4:57", "1:05:00", "0:00". */
export function clock(ms: number): string {
  const total = Math.max(0, Math.ceil(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = String(total % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${s}` : `${m}:${s}`;
}

/** A number of seconds in words: "30 seconds", "1 minute", "1 hour 15 minutes", "2 minutes 30 seconds". */
export function duration(seconds: number): string {
  const parts: [number, string][] = [
    [Math.floor(seconds / 3600), "hour"],
    [Math.floor((seconds % 3600) / 60), "minute"],
    [seconds % 60, "second"],
  ];
  const said = parts.filter(([n]) => n > 0).map(([n, unit]) => `${n} ${unit}${n === 1 ? "" : "s"}`);
  return said.length > 0 ? said.join(" ") : "0 seconds";
}

/** What a timer says when it rings. A range's timer rings at its lower end, so it says to check, and how much
 * longer the step allows: "5 minutes are up. Check it: the step allows up to 1 minute more." */
export function timeUp(time: Pick<StepTime, "seconds" | "until_seconds">): string {
  const said = duration(time.seconds);
  const up = `${said} ${/^1 \w+$/.test(said) ? "is" : "are"} up.`;
  if (!time.until_seconds || time.until_seconds <= time.seconds) return up;
  return `${up} Check it: the step allows up to ${duration(time.until_seconds - time.seconds)} more.`;
}
