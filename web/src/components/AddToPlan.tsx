import { useRef, useState } from "react";
import { Link } from "react-router";
import { api, errorText } from "../api/client";
import { addWeeks, mondayOf } from "../week";

/** Puts the chosen version, at the chosen serving count, on this week's or next week's plan. */
export function AddToPlan({ variantId, servings }: { variantId: string; servings: number }) {
  const thisWeek = mondayOf();
  const [week, setWeek] = useState(thisWeek);
  // What was last added. A different version, size or week is a new thing to add, so changing the choice forgets
  // the last result (even on coming back to it: the same dish can go on the plan twice).
  const target = `${variantId}|${servings}|${week}`;
  const [done, setDone] = useState<{ target: string; state: "saving" | "added" } | null>(null);
  const [chosen, setChosen] = useState(target);
  if (chosen !== target) {
    setChosen(target);
    setDone(null);
  }
  const state = done?.target === target ? done.state : "idle";
  // A failure belongs to the choice it was for, so it never shows next to another one.
  const [failure, setFailure] = useState<{ target: string; text: string } | null>(null);
  const error = failure?.target === target ? failure.text : "";
  // Only the latest submission's answer counts: an earlier one finishing late must not undo a newer one's state.
  const latest = useRef(0);

  /** Adds the choice to the plan; an earlier tap's answer arriving late is ignored. */
  const add = async () => {
    const mine = ++latest.current;
    setDone({ target, state: "saving" });
    setFailure(null);
    const { error: failed } = await api.POST("/api/v1/plan/entries", {
      body: { week, variant_id: variantId, servings },
    });
    if (mine !== latest.current) return;
    if (failed) {
      setDone(null);
      setFailure({ target, text: errorText(failed) });
      return;
    }
    setDone({ target, state: "added" });
  };

  return (
    <section className="add-to-plan" aria-label="Add to plan">
      <span className="option-label">Plan</span>
      <select value={week} onChange={(e) => setWeek(e.target.value)} aria-label="Which week">
        <option value={thisWeek}>This week</option>
        <option value={addWeeks(thisWeek, 1)}>Next week</option>
      </select>
      <button type="button" className="primary" onClick={add} disabled={state !== "idle"}>
        {state === "added" ? "Added" : "Add to plan"}
      </button>
      {state === "added" && <Link to={`/plan?week=${week}`}>View plan</Link>}
      {error && <span className="form-error">{error}</span>}
    </section>
  );
}
