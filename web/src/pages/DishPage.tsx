import { useEffect, useMemo, useState } from "react";
import { Link, useParams, useSearchParams } from "react-router";
import { api, type Dish, type Variant } from "../api/client";
import { available, choose, pick, selectionOf, type Selection } from "../choose";
import { Servings } from "../components/Servings";
import { StepText } from "../components/StepText";

export function DishPage() {
  const { slug = "" } = useParams();
  const [params, setParams] = useSearchParams();
  const [dish, setDish] = useState<Dish | null>(null);
  const [missing, setMissing] = useState(false);
  const [sel, setSel] = useState<Selection>({});
  const [variant, setVariant] = useState<Variant | null>(null);
  const [loadingVariant, setLoadingVariant] = useState(false);

  // ?v=<version id> pins a version; ?s=<servings> keeps the serving count across reloads and shares.
  const pinned = params.get("v");
  const servingsParam = Number(params.get("s")) || undefined;
  // Servings live in state (so quick taps queue up) and are mirrored to ?s= for reloads and links.
  const [chosenServings, setChosenServings] = useState<number | undefined>(servingsParam);

  useEffect(() => {
    const ctrl = new AbortController();
    setDish(null);
    setMissing(false);
    api.GET("/api/dishes/{slug}", { params: { path: { slug } }, signal: ctrl.signal }).then(({ data, response }) => {
      if (ctrl.signal.aborted) return;
      if (!data) return setMissing(response.status === 404);
      setDish(data);
      // Choices in the URL (?protein=Pork) win; the rest come from the pinned or default version.
      const start = data.variants.find((v) => v.id === pinned) ?? data.variants.find((v) => v.id === data.default_variant);
      const fromUrl = Object.fromEntries(data.options.flatMap((o) => (params.get(o.key) ? [[o.key, params.get(o.key)!]] : [])));
      const wanted = { ...selectionOf(start, data.options), ...fromUrl };
      setSel(pick(data.variants, wanted) ? wanted : selectionOf(start, data.options));
    });
    return () => ctrl.abort();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [slug]);

  const chosen = useMemo(() => {
    if (!dish) return undefined;
    return dish.variants.find((v) => v.id === pinned) ?? pick(dish.variants, sel) ?? dish.variants.find((v) => v.id === dish.default_variant);
  }, [dish, sel, pinned]);

  const defaultServings = (chosen?.yields.includes(2) ? 2 : chosen?.yields[0]) ?? 2;
  const servings = chosenServings ?? defaultServings;

  useEffect(() => setChosenServings(Number(params.get("s")) || undefined), [slug]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (!chosen) return;
    const ctrl = new AbortController();
    setLoadingVariant(true);
    api.GET("/api/variants/{id}", { params: { path: { id: chosen.id }, query: { servings } }, signal: ctrl.signal }).then(({ data }) => {
      if (ctrl.signal.aborted) return;
      if (data) setVariant(data);
      setLoadingVariant(false);
    });
    return () => ctrl.abort();
  }, [chosen, servings]);

  // Always from the current URL, so quick taps never act on a stale value.
  const update = (changes: (prev: URLSearchParams) => Record<string, string | null>) =>
    setParams(
      (prev) => {
        const next = new URLSearchParams(prev);
        for (const [k, v] of Object.entries(changes(prev))) {
          if (v === null) next.delete(k);
          else next.set(k, v);
        }
        return next;
      },
      { replace: true },
    );
  const setServings = (n: number | ((cur: number) => number)) =>
    setChosenServings((cur) => (typeof n === "function" ? n(cur ?? defaultServings) : n));
  useEffect(() => {
    if (chosenServings !== undefined && String(chosenServings) !== params.get("s")) update(() => ({ s: String(chosenServings) }));
  }, [chosenServings]); // eslint-disable-line react-hooks/exhaustive-deps

  if (missing) {
    return (
      <p className="empty">
        No such dish. <Link to="/">Back to all dishes</Link>
      </p>
    );
  }
  if (!dish || !chosen) return <p className="empty">Loading…</p>;

  const v = variant && variant.id === chosen.id ? variant : null;
  const image = v?.image ?? chosen.image;
  const prep = minutes(v?.meta.prep_time) ?? minutes(v?.meta.total_time);

  return (
    <article className="dish">
      <div className="hero">
        {image ? <img src={image.large} alt="" width={1200} height={800} /> : <div className="noimg" aria-hidden="true" />}
      </div>

      <header className="dish-head">
        <h1>{dish.title}</h1>
        {(v?.subtitle ?? chosen.subtitle) && <p className="sub">{v?.subtitle ?? chosen.subtitle}</p>}
        {chosen.title !== dish.title && <p className="meta">This version: {chosen.title}</p>}
        <p className="meta">
          {[prep && `${prep} min`, typeof v?.meta.difficulty === "number" && difficulty(v.meta.difficulty as number), year(chosen.published_at)]
            .filter(Boolean)
            .join(" · ")}
        </p>
      </header>

      {dish.options.length > 0 && (
        <section className="options" aria-label="Options">
          {dish.options.map((o) => (
            <div key={o.key} className="option">
              <span className="option-label">{o.label}</span>
              <div className="segmented wrap" role="radiogroup" aria-label={o.label}>
                {o.choices.map((c) => {
                  const on = sel[o.key] === c.value && !pinned;
                  const ok = available(dish.variants, sel, o.key, c.value);
                  return (
                    <button
                      key={c.value}
                      type="button"
                      role="radio"
                      aria-checked={on}
                      className={ok ? "" : "other"}
                      title={c.years ? `${c.label}: published ${c.years}` : c.label}
                      onClick={() => {
                        const next = choose(dish.variants, dish.options, sel, o.key, c.value);
                        setSel(next);
                        update(() => ({ v: null, ...Object.fromEntries(dish.options.map((x) => [x.key, next[x.key] ?? null])) }));
                      }}
                    >
                      {c.label}
                    </button>
                  );
                })}
              </div>
            </div>
          ))}
        </section>
      )}

      <Servings value={servings} published={chosen.yields} onChange={setServings} />
      {v && !v.official && <ScaledNotice v={v} servings={servings} />}

      <div className={`cook ${loadingVariant ? "stale" : ""}`}>
        <section className="ingredients" aria-labelledby="ing-h">
          <h2 id="ing-h">Ingredients</h2>
          {v ? (
            <ul>
              {v.ingredients.map((i, n) => (
                <li key={n} className={i.shipped === false ? "pantry" : ""}>
                  {i.image ? <img src={i.image.small} alt="" loading="lazy" width={40} height={40} /> : <span className="dot" aria-hidden="true" />}
                  <span className="amount">
                    {i.scaled && <span className="approx" title="Scaled from the published amounts">≈</span>}
                    {i.amount ?? ""}
                  </span>
                  <span className="name">
                    {i.name}
                    {i.shipped === false && <span className="tag">pantry</span>}
                  </span>
                </li>
              ))}
            </ul>
          ) : (
            <p className="empty">Loading…</p>
          )}
          {v && allergens(v).length > 0 && <p className="allergens">Contains: {allergens(v).join(", ")}</p>}
        </section>

        <section className="steps" aria-labelledby="steps-h">
          <h2 id="steps-h">Steps</h2>
          {v && v.steps.length === 0 && <p className="empty">This version has no steps.</p>}
          <ol>
            {v?.steps.map((s) => (
              <li key={s.position}>
                {s.images[0] && <img src={s.images[0].large} alt="" loading="lazy" width={1200} height={800} />}
                <div className="step-text">
                  <StepText text={s.text} />
                  {(s.scaled || s.check) && (
                    <p className="step-flag">{s.scaled ? "Amounts scaled for this serving count." : "Wording follows the nearest published size."}</p>
                  )}
                </div>
              </li>
            ))}
          </ol>
        </section>
      </div>

      {dish.variants.length > 1 && (
        <details className="history">
          <summary>All {dish.variants.length} versions</summary>
          <ul>
            {dish.variants.map((x) => (
              <li key={x.id}>
                <button type="button" className={x.id === chosen.id ? "current" : ""} onClick={() => update(() => ({ v: x.id }))}>
                  <span>{x.title}</span>
                  <span className="meta">
                    {[year(x.published_at), x.yields.length && `serves ${x.yields.join("/")}`, !x.has_steps && "no steps"].filter(Boolean).join(" · ")}
                  </span>
                </button>
              </li>
            ))}
          </ul>
          {pinned && (
            <button type="button" className="link" onClick={() => update(() => ({ v: null }))}>
              Back to choosing by options
            </button>
          )}
        </details>
      )}
    </article>
  );
}

function minutes(iso: unknown): number | undefined {
  if (typeof iso !== "string") return undefined;
  const m = /PT(?:(\d+)H)?(?:(\d+)M)?/.exec(iso);
  if (!m) return undefined;
  const total = Number(m[1] ?? 0) * 60 + Number(m[2] ?? 0);
  return total > 0 ? total : undefined;
}

const difficulty = (d: number) => ["", "Easy", "Medium", "Hard"][d] ?? "";
const year = (iso?: string | null) => (iso ? new Date(iso).getFullYear().toString() : "");
const allergens = (v: Variant) => [...new Set(v.ingredients.flatMap((i) => i.allergens))].sort();

/** Why some amounts on the page are scaled, and which ones. */
function ScaledNotice({ v, servings }: { v: Variant; servings: number }) {
  const ingredients = v.ingredients.some((i) => i.scaled);
  const steps = v.steps.some((s) => s.scaled);
  const what = ingredients && steps ? "Ingredient amounts marked ≈ and amounts in marked steps" : ingredients ? "Amounts marked ≈" : "Amounts in marked steps";
  const why = !v.published_yield
    ? `${servings} servings isn't a published size.`
    : ingredients
      ? `Not every amount was published for ${servings} servings.`
      : `Ingredient amounts are as published, but the step text was only written for other sizes.`;
  return (
    <p className="notice">
      {why} {what} are scaled from the published ones and rounded to kitchen measures. Check them as you go.
    </p>
  );
}
