import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { Link, useSearchParams } from "react-router";
import { api, errorText, type DishSummary, type Plan, type PlanEntry } from "../api/client";
import { answer, queryClient, Refused } from "../api/query";
import { useAccount } from "../session";
import { addWeeks, mondayOf, weekLabel } from "../week";

const SUGGESTIONS = 6;

/** The household's plan for a week (?week=, this week by default): dishes at their serving counts, and a search
 * to add more. */
export function PlanPage() {
  const [params, setParams] = useSearchParams();
  const week = params.get("week") ?? mondayOf();
  const key = ["plan", useAccount(), week];
  const planQuery = useQuery({
    queryKey: key,
    queryFn: ({ signal }) => answer(api.GET("/api/plan", { params: { query: { week } }, signal })),
  });
  const plan = planQuery.data ?? null;
  // A change that didn't go through, shown with the week it was made in.
  const [failure, setFailure] = useState<{ week: string; text: string } | null>(null);
  const error =
    (failure?.week === week && failure.text) ||
    (planQuery.error
      ? planQuery.error instanceof Refused
        ? errorText(planQuery.error.body)
        : "Can't reach the server. Planning needs it; the shopping list works offline."
      : "");

  /** Moves to the week `n` weeks away. */
  const go = (n: number) => setParams({ week: addWeeks(week, n) });
  /** Every change answers with the whole plan, which replaces the one on screen. */
  const run = async (call: () => Promise<{ data?: Plan; error?: unknown }>) => {
    setFailure(null);
    // A reload still on its way would put back the plan from before this change.
    await queryClient.cancelQueries({ queryKey: key });
    const { data, error: failed } = await call().catch(() => ({
      data: undefined,
      error: { error: "can't reach the server, so that wasn't saved" },
    }));
    if (data) queryClient.setQueryData(key, data);
    else {
      setFailure({ week, text: errorText(failed) });
      queryClient.invalidateQueries({ queryKey: key });
    }
  };
  /** Adds a dish at its usual version: 2 servings if it was published for 2, else its first published size. */
  const add = (d: DishSummary) => {
    const variant_id = d.default_variant;
    if (variant_id) run(() => api.POST("/api/plan/entries", { body: { week, variant_id } }));
  };
  /** Changes how many servings of one dish are planned. */
  const setServings = (e: PlanEntry, servings: number) =>
    run(() =>
      api.PATCH("/api/plan/entries/{id}", { params: { path: { id: e.id } }, body: { servings } }),
    );
  /** Takes a dish off the plan. */
  const remove = (e: PlanEntry) =>
    run(() => api.DELETE("/api/plan/entries/{id}", { params: { path: { id: e.id } } }));

  const count = plan?.entries.length ?? 0;
  return (
    <div className="plan">
      <header className="week-head">
        <div className="week-nav">
          <button type="button" onClick={() => go(-1)} aria-label="Previous week">
            ‹
          </button>
          <h1>{weekLabel(week)}</h1>
          <button type="button" onClick={() => go(1)} aria-label="Next week">
            ›
          </button>
        </div>
        {count > 0 && (
          <Link className="primary" to={`/shopping?week=${week}`}>
            Shopping list
          </Link>
        )}
      </header>
      {error && <p className="notice">{error}</p>}

      {!plan && !error && <p className="empty">Loading…</p>}
      {plan && count === 0 && (
        <p className="meta">Nothing planned yet. Add dishes below, or from any recipe page.</p>
      )}
      {plan && count > 0 && (
        <ul className="plan-list">
          {plan.entries.map((e) => (
            <li key={e.id}>
              <Link
                to={`/dishes/${e.dish_slug}?v=${e.variant_id}&s=${e.servings}`}
                className="plan-dish"
              >
                {e.image ? (
                  <img src={e.image.small} alt="" loading="lazy" width={72} height={54} />
                ) : (
                  <span className="noimg" aria-hidden="true" />
                )}
                <span className="plan-text">
                  <strong>{e.title}</strong>
                  {e.subtitle && <span className="sub">{e.subtitle}</span>}
                </span>
              </Link>
              <div className="plan-actions">
                <div className="stepper small">
                  <button
                    type="button"
                    onClick={() => setServings(e, e.servings - 1)}
                    disabled={e.servings <= 1}
                    aria-label={`Fewer servings of ${e.title}`}
                  >
                    −
                  </button>
                  <output aria-live="polite">{e.servings}</output>
                  <button
                    type="button"
                    onClick={() => setServings(e, e.servings + 1)}
                    disabled={e.servings >= 24}
                    aria-label={`More servings of ${e.title}`}
                  >
                    +
                  </button>
                </div>
                <button
                  type="button"
                  className="quiet"
                  onClick={() => remove(e)}
                  aria-label={`Remove ${e.title}`}
                >
                  Remove
                </button>
              </div>
            </li>
          ))}
        </ul>
      )}

      <AddDishes onAdd={add} />
    </div>
  );
}

/** Find a dish and add it in one tap, at its usual version and 2 servings. */
function AddDishes({ onAdd }: { onAdd: (d: DishSummary) => void }) {
  const [q, setQ] = useState("");
  // What to look for: the box's text once typing pauses.
  const [asked, setAsked] = useState("");
  useEffect(() => {
    const wait = setTimeout(() => setAsked(q.trim()), q ? 200 : 0);
    return () => clearTimeout(wait);
  }, [q]);
  const found = useQuery({
    queryKey: ["suggestions", asked],
    queryFn: ({ signal }) =>
      answer(
        api.GET("/api/dishes", {
          params: { query: { q: asked || undefined, type: "recipe", limit: SUGGESTIONS } },
          signal,
        }),
      ),
    placeholderData: keepPreviousData,
  });
  const items = found.data?.items ?? [];

  return (
    <section className="add-dishes" aria-labelledby="add-h">
      <h2 id="add-h">Add a dish</h2>
      <input
        type="search"
        value={q}
        onChange={(e) => setQ(e.target.value)}
        placeholder="Search your dishes"
        aria-label="Search your dishes"
        enterKeyHint="search"
      />
      {items.length === 0 && q && <p className="meta">No dishes match that.</p>}
      <ul className="add-list">
        {items.map((d) => (
          <li key={d.id}>
            <span className="plan-dish">
              {d.image ? (
                <img src={d.image.small} alt="" loading="lazy" width={72} height={54} />
              ) : (
                <span className="noimg" aria-hidden="true" />
              )}
              <span className="plan-text">
                <strong>{d.title}</strong>
                {d.subtitle && <span className="sub">{d.subtitle}</span>}
              </span>
            </span>
            <button
              type="button"
              className="add"
              onClick={() => onAdd(d)}
              aria-label={`Add ${d.title}`}
            >
              Add
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
