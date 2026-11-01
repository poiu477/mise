import { useCallback, useEffect, useRef, useState, type FormEvent } from "react";
import { Link, useSearchParams } from "react-router";
import { api, errorText, type ShoppingItem, type ShoppingList } from "../api/client";
import { addWeeks, mondayOf, weekLabel } from "../week";

export function ShoppingPage() {
  const [params, setParams] = useSearchParams();
  const week = params.get("week") ?? mondayOf();
  const [list, setList] = useState<ShoppingList | null>(null);
  const [error, setError] = useState("");
  const [extra, setExtra] = useState("");
  const [open, setOpen] = useState<string | null>(null);
  // Only the latest answer counts: quick ticks must not be undone by an older reply arriving late.
  const latest = useRef(0);

  const load = useCallback(async () => {
    const asked = ++latest.current;
    const { data, error: failed } = await api.GET("/api/shopping", { params: { query: { week } } });
    if (asked !== latest.current) return;
    if (data) setList(data);
    else setError(errorText(failed));
  }, [week]);

  useEffect(() => {
    setList(null);
    load();
    // Someone else in the household may be shopping too.
    const refresh = () => document.visibilityState === "visible" && load();
    document.addEventListener("visibilitychange", refresh);
    return () => document.removeEventListener("visibilitychange", refresh);
  }, [load]);

  const run = async (call: Promise<{ data?: ShoppingList; error?: unknown }>) => {
    const asked = ++latest.current;
    setError("");
    const { data, error: failed } = await call;
    if (asked !== latest.current) return;
    if (data) setList(data);
    else {
      setError(errorText(failed));
      load();
    }
  };

  const tick = (item: ShoppingItem, checked: boolean) => {
    setList((l) => l && { ...l, sections: l.sections.map((s) => ({ ...s, items: s.items.map((i) => (i.key === item.key ? { ...i, checked } : i)) })) });
    run(api.PUT("/api/shopping/marks", { body: { week, key: item.key, checked } }));
  };
  const hide = (key: string, hidden: boolean) => run(api.PUT("/api/shopping/marks", { body: { week, key, hidden } }));
  const always = (foodId: string) => run(api.POST("/api/pantry-staples", { body: { week, food_id: foodId } }));
  const unstaple = (foodId: string) => run(api.DELETE("/api/pantry-staples/{food_id}", { params: { path: { food_id: foodId }, query: { week } } }));
  const addExtra = (e: FormEvent) => {
    e.preventDefault();
    if (!extra.trim()) return;
    run(api.POST("/api/shopping/extras", { body: { week, name: extra } }));
    setExtra("");
  };

  const items = list?.sections.flatMap((s) => s.items) ?? [];
  const total = items.length + (list?.extras.length ?? 0);
  const left = items.filter((i) => !i.checked).length + (list?.extras.filter((x) => !x.checked).length ?? 0);

  return (
    <div className="shopping">
      <header className="week-head">
        <div className="week-nav no-print">
          <button type="button" onClick={() => setParams({ week: addWeeks(week, -1) })} aria-label="Previous week">
            ‹
          </button>
          <h1>{weekLabel(week)}</h1>
          <button type="button" onClick={() => setParams({ week: addWeeks(week, 1) })} aria-label="Next week">
            ›
          </button>
        </div>
        <h1 className="print-only">Shopping list: {weekLabel(week)}</h1>
        {total > 0 && (
          <p className="meta" aria-live="polite">
            {left === 0 ? "All done" : `${left} of ${total} left`}
          </p>
        )}
        <div className="head-actions no-print">
          <Link to={`/plan?week=${week}`}>Plan</Link>
          {total > 0 && (
            <button type="button" className="quiet" onClick={() => window.print()}>
              Print
            </button>
          )}
        </div>
      </header>
      {error && <p className="notice">{error}</p>}
      {!list && !error && <p className="empty">Loading…</p>}
      {list && list.meals === 0 && list.extras.length === 0 && (
        <p className="empty">
          Nothing planned for this week yet. <Link to={`/plan?week=${week}`}>Plan some dishes</Link> and the list fills itself in.
        </p>
      )}

      {list && (
        <form className="add-extra no-print" onSubmit={addExtra}>
          <input value={extra} onChange={(e) => setExtra(e.target.value)} placeholder="Add something else" aria-label="Add something else" maxLength={200} enterKeyHint="done" />
          <button type="submit" disabled={!extra.trim()}>
            Add
          </button>
        </form>
      )}

      {list && list.extras.length > 0 && (
        <section className="shop-section">
          <h2>Added by you</h2>
          <ul>
            {list.extras.map((x) => (
              <li key={x.id} className={x.checked ? "done" : ""}>
                <label>
                  <input
                    type="checkbox"
                    checked={x.checked}
                    onChange={(e) => run(api.PATCH("/api/shopping/extras/{id}", { params: { path: { id: x.id } }, body: { checked: e.target.checked } }))}
                  />
                  <span className="item-name">{x.name}</span>
                </label>
                <div className="item-actions no-print">
                  <button type="button" className="quiet" onClick={() => run(api.DELETE("/api/shopping/extras/{id}", { params: { path: { id: x.id } } }))} aria-label={`Delete ${x.name}`}>
                    Delete
                  </button>
                </div>
              </li>
            ))}
          </ul>
        </section>
      )}

      {list?.sections.map((s) => (
        <section className="shop-section" key={s.name}>
          <h2>{s.name}</h2>
          <ul>
            {s.items.map((i) => (
              <li key={i.key} className={i.checked ? "done" : ""}>
                <label>
                  <input type="checkbox" checked={i.checked} onChange={(e) => tick(i, e.target.checked)} />
                  <span className="item-name">
                    {i.name}
                    {i.amount && <span className="item-amount">{i.amount}</span>}
                  </span>
                </label>
                <button
                  type="button"
                  className="more-toggle no-print"
                  aria-expanded={open === i.key}
                  aria-label={`More about ${i.name}`}
                  onClick={() => setOpen(open === i.key ? null : i.key)}
                >
                  ⋯
                </button>
                {open === i.key && (
                  <div className="item-more no-print">
                    <span className="item-for">For {i.recipes.join(", ")}</span>
                    <div className="item-actions">
                      {i.food_id && (
                        <button type="button" className="quiet" onClick={() => always(i.food_id!)} title="Leave this off every list" aria-label={`Always have ${i.name}`}>
                          Always have
                        </button>
                      )}
                      <button type="button" className="quiet" onClick={() => hide(i.key, true)} title="Take it off this week's list" aria-label={`Remove ${i.name}`}>
                        Remove
                      </button>
                    </div>
                  </div>
                )}
              </li>
            ))}
          </ul>
        </section>
      ))}

      {list && list.removed.length + list.staples.length > 0 && (
        <section className="left-out no-print">
          <h2>Left off this list</h2>
          <ul>
            {list.staples.map((l) => (
              <li key={l.key}>
                <span>{l.name}</span>
                <span className="meta">always on hand</span>
                <button type="button" className="quiet" onClick={() => unstaple(l.key)} aria-label={`Put ${l.name} back`}>
                  Put back
                </button>
              </li>
            ))}
            {list.removed.map((l) => (
              <li key={l.key}>
                <span>{l.name}</span>
                <span className="meta">this week</span>
                <button type="button" className="quiet" onClick={() => hide(l.key, false)} aria-label={`Put ${l.name} back`}>
                  Put back
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
  );
}
