import { useEffect, useState } from "react";
import { Link, useSearchParams } from "react-router";
import { api, errorText, type DishSummary, type Plan, type PlanEntry } from "../api/client";
import { addWeeks, mondayOf, weekLabel } from "../week";

const SUGGESTIONS = 6;

export function PlanPage() {
  const [params, setParams] = useSearchParams();
  const week = params.get("week") ?? mondayOf();
  const [plan, setPlan] = useState<Plan | null>(null);
  const [error, setError] = useState("");

  useEffect(() => {
    const ctrl = new AbortController();
    setPlan(null);
    api.GET("/api/plan", { params: { query: { week } }, signal: ctrl.signal }).then(({ data, error: failed }) => {
      if (ctrl.signal.aborted) return;
      if (data) setPlan(data);
      else setError(errorText(failed));
    });
    return () => ctrl.abort();
  }, [week]);

  const go = (n: number) => setParams({ week: addWeeks(week, n) });
  const run = async (call: Promise<{ data?: Plan; error?: unknown }>) => {
    setError("");
    const { data, error: failed } = await call;
    if (data) setPlan(data);
    else setError(errorText(failed));
  };
  const add = (d: DishSummary) => {
    if (d.default_variant) run(api.POST("/api/plan/entries", { body: { week, variant_id: d.default_variant } }));
  };
  const setServings = (e: PlanEntry, servings: number) =>
    run(api.PATCH("/api/plan/entries/{id}", { params: { path: { id: e.id } }, body: { servings } }));
  const remove = (e: PlanEntry) => run(api.DELETE("/api/plan/entries/{id}", { params: { path: { id: e.id } } }));

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
      {plan && count === 0 && <p className="meta">Nothing planned yet. Add dishes below, or from any recipe page.</p>}
      {plan && count > 0 && (
        <ul className="plan-list">
          {plan.entries.map((e) => (
            <li key={e.id}>
              <Link to={`/dishes/${e.dish_slug}?v=${e.variant_id}&s=${e.servings}`} className="plan-dish">
                {e.image ? <img src={e.image.small} alt="" loading="lazy" width={72} height={54} /> : <span className="noimg" aria-hidden="true" />}
                <span className="plan-text">
                  <strong>{e.title}</strong>
                  {e.subtitle && <span className="sub">{e.subtitle}</span>}
                </span>
              </Link>
              <div className="plan-actions">
                <div className="stepper small">
                  <button type="button" onClick={() => setServings(e, e.servings - 1)} disabled={e.servings <= 1} aria-label={`Fewer servings of ${e.title}`}>
                    −
                  </button>
                  <output aria-live="polite">{e.servings}</output>
                  <button type="button" onClick={() => setServings(e, e.servings + 1)} disabled={e.servings >= 24} aria-label={`More servings of ${e.title}`}>
                    +
                  </button>
                </div>
                <button type="button" className="quiet" onClick={() => remove(e)} aria-label={`Remove ${e.title}`}>
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
  const [items, setItems] = useState<DishSummary[]>([]);

  useEffect(() => {
    const ctrl = new AbortController();
    const wait = setTimeout(
      () =>
        api.GET("/api/dishes", { params: { query: { q: q.trim() || undefined, type: "recipe", limit: SUGGESTIONS } }, signal: ctrl.signal }).then(({ data }) => {
          if (data && !ctrl.signal.aborted) setItems(data.items);
        }),
      q ? 200 : 0,
    );
    return () => {
      clearTimeout(wait);
      ctrl.abort();
    };
  }, [q]);

  return (
    <section className="add-dishes" aria-labelledby="add-h">
      <h2 id="add-h">Add a dish</h2>
      <input type="search" value={q} onChange={(e) => setQ(e.target.value)} placeholder="Search your dishes" aria-label="Search your dishes" enterKeyHint="search" />
      {items.length === 0 && q && <p className="meta">No dishes match that.</p>}
      <ul className="add-list">
        {items.map((d) => (
          <li key={d.id}>
            <span className="plan-dish">
              {d.image ? <img src={d.image.small} alt="" loading="lazy" width={72} height={54} /> : <span className="noimg" aria-hidden="true" />}
              <span className="plan-text">
                <strong>{d.title}</strong>
                {d.subtitle && <span className="sub">{d.subtitle}</span>}
              </span>
            </span>
            <button type="button" className="add" onClick={() => onAdd(d)} aria-label={`Add ${d.title}`}>
              Add
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
