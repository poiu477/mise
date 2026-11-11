import { keepPreviousData, useInfiniteQuery } from "@tanstack/react-query";
import { Link, useSearchParams } from "react-router";
import { api } from "../api/client";
import { answer } from "../api/query";

const PAGE = 24;
const TYPES = [
  { value: "recipe", label: "Recipes" },
  { value: "product", label: "Store items" },
  { value: "", label: "Everything" },
];

/** The home page: dishes matching the top bar's search (?q=), an ingredient (?ingredient=) and a kind (?type=,
 * recipes by default), 24 at a time with "Show more". */
export function SearchPage() {
  const [params, setParams] = useSearchParams();
  const q = params.get("q") ?? "";
  const ingredient = params.get("ingredient") ?? "";
  const type = params.get("type") ?? "recipe";
  const asked = { q: q || undefined, ingredient: ingredient || undefined, type: type || undefined };

  const found = useInfiniteQuery({
    queryKey: ["dishes", asked],
    queryFn: ({ pageParam, signal }) =>
      answer(
        api.GET("/api/v1/dishes", {
          params: { query: { ...asked, limit: PAGE, offset: pageParam } },
          signal,
        }),
      ),
    initialPageParam: 0,
    getNextPageParam: (last, pages) => {
      const shown = pages.reduce((n, p) => n + p.items.length, 0);
      return shown < last.total ? shown : undefined;
    },
    // A new search keeps the last results on screen until its own arrive.
    placeholderData: keepPreviousData,
  });
  const items = found.data?.pages.flatMap((p) => p.items) ?? [];
  const total = found.data?.pages.at(-1)?.total ?? 0;
  const ready = found.isSuccess && !found.isPlaceholderData && !found.isFetchingNextPage;

  /** Sets one search parameter, or removes it when empty, keeping the others. */
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
          {found.isError
            ? "Couldn't reach the server."
            : ready || items.length
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

      {ready && items.length === 0 && (
        <p className="empty">
          {q || ingredient
            ? "No dishes match. Try fewer words."
            : "No dishes yet. Import a seed bundle or add a recipe to get started."}
        </p>
      )}
      {found.hasNextPage && (
        <div className="more">
          <button type="button" onClick={() => found.fetchNextPage()} disabled={found.isFetching}>
            {found.isFetchingNextPage ? "Loading…" : "Show more"}
          </button>
        </div>
      )}
    </section>
  );
}
