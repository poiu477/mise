import { useState } from "react";
import { Link } from "react-router";
import { api, errorText } from "../api/client";
import { addWeeks, mondayOf } from "../week";

/** Puts the chosen version, at the chosen serving count, on this week's or next week's plan. */
export function AddToPlan({ variantId, servings }: { variantId: string; servings: number }) {
  const thisWeek = mondayOf();
  const [week, setWeek] = useState(thisWeek);
  // What was last added; a different version, size or week is a new thing to add.
  const target = `${variantId}|${servings}|${week}`;
  const [done, setDone] = useState<{ target: string; state: "saving" | "added" } | null>(null);
  const state = done?.target === target ? done.state : "idle";
  const [error, setError] = useState("");

  const add = async () => {
    setDone({ target, state: "saving" });
    setError("");
    const { error: failed } = await api.POST("/api/plan/entries", { body: { week, variant_id: variantId, servings } });
    if (failed) {
      setDone(null);
      setError(errorText(failed));
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
