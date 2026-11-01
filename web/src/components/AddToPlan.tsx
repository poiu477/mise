import { useEffect, useState } from "react";
import { Link } from "react-router";
import { api, errorText } from "../api/client";
import { addWeeks, mondayOf } from "../week";

/** Puts the chosen version, at the chosen serving count, on this week's or next week's plan. */
export function AddToPlan({ variantId, servings }: { variantId: string; servings: number }) {
  const thisWeek = mondayOf();
  const [week, setWeek] = useState(thisWeek);
  const [state, setState] = useState<"idle" | "saving" | "added">("idle");
  const [error, setError] = useState("");

  // A different version or size is a new thing to add.
  useEffect(() => setState("idle"), [variantId, servings, week]);

  const add = async () => {
    setState("saving");
    setError("");
    const { error: failed } = await api.POST("/api/plan/entries", { body: { week, variant_id: variantId, servings } });
    if (failed) {
      setState("idle");
      setError(errorText(failed));
      return;
    }
    setState("added");
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
