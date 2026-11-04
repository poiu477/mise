import { useEffect, useState } from "react";
import { Link, useSearchParams } from "react-router";
import { api, type DishSummary } from "../api/client";

const PAGE = 24;
const TYPES = [
  { value: "recipe", label: "Recipes" },
  { value: "product", label: "Store items" },
  { value: "", label: "Everything" },
];

export function SearchPage() {
  const [params, setParams] = useSearchParams();
  const q = params.get("q") ?? "";
  const ingredient = params.get("ingredient") ?? "";
  const type = params.get("type") ?? "recipe";
  const [items, setItems] = useState<DishSummary[]>([]);
  const [total, setTotal] = useState(0);
  const [state, setState] = useState<"loading" | "ready" | "error">("loading");

  const load = async (offset: number, signal?: AbortSignal) => {
    setState("loading");
    const { data, error } = await api.GET("/api/dishes", {
      params: {
        query: {
          q: q || undefined,
          ingredient: ingredient || undefined,
          type: type || undefined,
          limit: PAGE,
          offset,
        },
      },
      signal,
    });
    if (signal?.aborted) return;
    if (error || !data) {
      setState("error");
      return;
    }
    setItems((prev) => (offset === 0 ? data.items : [...prev, ...data.items]));
    setTotal(data.total);
    setState("ready");
  };

  useEffect(() => {
    const ctrl = new AbortController();
    load(0, ctrl.signal).catch(() => !ctrl.signal.aborted && setState("error"));
    return () => ctrl.abort();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [q, ingredient, type]);

  const setParam = (key: string, value: string) => {
    const next = new URLSearchParams(params);
    if (value) next.set(key, value);
    else next.delete(key);
    setParams(next);
  };

  return (
    <section>
      <div className="filters">
        <div className="segmented" role="radiogroup" aria-label="What to show">
          {TYPES.map((t) => (
            <button
              key={t.label}
              type="button"
              role="radio"
              aria-checked={type === t.value}
              onClick={() => setParam("type", t.value || "all")}
            >
              {t.label}
            </button>
          ))}
        </div>
        <label className="ingredient-filter">
          <span>With</span>
          <input
            type="search"
            defaultValue={ingredient}
            placeholder="an ingredient"
            onKeyDown={(e) =>
              e.key === "Enter" &&
              setParam("ingredient", (e.target as HTMLInputElement).value.trim())
            }
            onBlur={(e) =>
              e.target.value.trim() !== ingredient && setParam("ingredient", e.target.value.trim())
            }
          />
        </label>
        <p className="count" aria-live="polite">
          {state === "error"
            ? "Couldn't reach the server."
            : state === "ready" || items.length
              ? `${total.toLocaleString()} dishes`
              : "Loading…"}
        </p>
      </div>

      <ul className="grid">
        {items.map((d) => (
          <li key={d.id}>
            <Link to={`/dishes/${d.slug}`} className="card">
              {d.image ? (
                <img src={d.image.small} alt="" loading="lazy" width={400} height={300} />
              ) : (
                <div className="noimg" aria-hidden="true" />
              )}
              <div className="card-body">
                <h2>{d.title}</h2>
                {d.subtitle && <p className="sub">{d.subtitle}</p>}
                {d.versions > 1 && <p className="meta">{d.versions} versions</p>}
              </div>
            </Link>
          </li>
        ))}
      </ul>

      {state === "ready" && items.length === 0 && (
        <p className="empty">
          {q || ingredient
            ? "No dishes match. Try fewer words."
            : "No dishes yet. Import a seed bundle or add a recipe to get started."}
        </p>
      )}
      {items.length < total && (
        <div className="more">
          <button type="button" onClick={() => load(items.length)} disabled={state === "loading"}>
            {state === "loading" ? "Loading…" : "Show more"}
          </button>
        </div>
      )}
    </section>
  );
}
