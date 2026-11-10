import { keepPreviousData, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { Link, Navigate, useParams, useSearchParams } from "react-router";
import { api, type Variant } from "../api/client";
import { answer, Refused, refusedRequest } from "../api/query";
import {
  available,
  cardParams,
  choose,
  startingChoice,
  startingServings,
  versionOf,
  type Selection,
} from "../choose";
import { AddToPlan } from "../components/AddToPlan";
import { Servings } from "../components/Servings";
import { StepText } from "../components/StepText";
import { allergens, difficulty, minutes } from "../recipe";

const NONE: Selection = {};

/** A dish: its options, any serving count, adding it to the plan, then ingredients and steps for the chosen
 * version. The URL keeps the version (?v=), servings (?s=) and options, so a reload or a link opens the same. */
export function DishPage() {
  const { slug = "" } = useParams();
  const [params, setParams] = useSearchParams();

  // ?v=<version id> pins a version; ?s=<servings> keeps the serving count across reloads and shares.
  const pinned = params.get("v");
  const servingsParam = Number(params.get("s")) || undefined;

  const queryClient = useQueryClient();
  const dishQuery = useQuery({
    queryKey: ["dish", slug],
    /** Loads the dish. An old address answers with the dish under its current slug, which is cached too, so
     * moving to the current address below shows it at once. */
    queryFn: async ({ signal }) => {
      const dish = await answer(
        api.GET("/api/dishes/{slug}", { params: { path: { slug } }, signal }),
      );
      if (dish.slug !== slug) queryClient.setQueryData(["dish", dish.slug], dish);
      return dish;
    },
  });
  const dish = dishQuery.data;
  const missing = dishQuery.error instanceof Refused && dishQuery.error.status === 404;

  // The options chosen: where the dish starts when it first shows, then whatever is tapped.
  const [picked, setPicked] = useState<{ slug: string; sel: Selection } | null>(null);
  if (dish && picked?.slug !== slug) setPicked({ slug, sel: startingChoice(dish, params) });
  const sel = picked?.slug === slug ? picked.sel : NONE;

  // The version follows the options as tapped here; the address only says where they start.
  const chosen = useMemo(() => dish && versionOf(dish, sel, pinned), [dish, sel, pinned]);

  const defaultServings = chosen ? startingServings(chosen) : 2;
  // Servings follow the URL until changed here; then they live in state (so quick taps queue up), mirrored to ?s=
  // for reloads and links.
  const [tapped, setTapped] = useState<{ slug: string; n: number } | null>(null);
  const chosenServings = tapped?.slug === slug ? tapped.n : servingsParam;
  const servings = chosenServings ?? defaultServings;

  const variantQuery = useQuery({
    queryKey: ["variant", chosen?.id, servings],
    queryFn: ({ signal }) =>
      answer(
        api.GET("/api/variants/{id}", {
          params: { path: { id: chosen!.id }, query: { servings } },
          signal,
        }),
      ),
    enabled: !!chosen,
    // The last version or serving count stays on screen, dimmed, until the new one arrives.
    placeholderData: keepPreviousData,
  });
  const variant = variantQuery.data;
  const loadingVariant = variantQuery.isPlaceholderData;

  /** Changes URL parameters (null removes one), always from the current URL so quick taps never act on a stale
   * value. Replaces the history entry rather than adding one. */
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
  /** Sets the serving count; the function form builds on the last tap, so quick taps all count. */
  const setServings = (n: number | ((cur: number) => number)) =>
    setTapped((cur) => {
      const was = cur?.slug === slug ? cur.n : (servingsParam ?? defaultServings);
      return { slug, n: typeof n === "function" ? n(was) : n };
    });
  useEffect(() => {
    if (chosenServings !== undefined && String(chosenServings) !== params.get("s"))
      update(() => ({ s: String(chosenServings) }));
  }, [chosenServings]); // eslint-disable-line react-hooks/exhaustive-deps

  // The dish was renamed since this address was made: replace it with the current one, keeping ?v=, ?s= and the
  // options, so bookmarks and old links still land here.
  if (dish && dish.slug !== slug)
    return (
      <Navigate to={{ pathname: `/dishes/${dish.slug}`, search: params.toString() }} replace />
    );
  if (missing) {
    return (
      <p className="empty">
        No such dish. <Link to="/">Back to all dishes</Link>
      </p>
    );
  }
  if (dishQuery.isError && !dish)
    return (
      <p className="empty">
        Can't reach the server, so this dish didn't load.{" "}
        <button type="button" className="link" onClick={() => dishQuery.refetch()}>
          Try again
        </button>
      </p>
    );
  if (!dish || !chosen) return <p className="empty">Loading…</p>;

  const v = variant && variant.id === chosen.id ? variant : null;
  const image = v?.image ?? chosen.image;
  const prep = minutes(v?.meta.prep_time) ?? minutes(v?.meta.total_time);

  return (
    <article className="dish">
      <div className="hero">
        {image ? (
          <img src={image.large} alt="" width={1200} height={800} />
        ) : (
          <div className="noimg" aria-hidden="true" />
        )}
      </div>

      <header className="dish-head">
        <h1>{dish.title}</h1>
        {(v?.subtitle ?? chosen.subtitle) && (
          <p className="sub">{v?.subtitle ?? chosen.subtitle}</p>
        )}
        {chosen.title !== dish.title && <p className="meta">This version: {chosen.title}</p>}
        <p className="meta">
          {[prep && `${prep} min`, difficulty(v?.meta.difficulty), year(chosen.published_at)]
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
                        setPicked({ slug, sel: next });
                        update(() => ({
                          v: null,
                          ...Object.fromEntries(
                            dish.options.map((x) => [x.key, next[x.key] ?? null]),
                          ),
                        }));
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
      <AddToPlan variantId={chosen.id} servings={servings} />
      <p className="card-link">
        <Link to={`/dishes/${dish.slug}/card?${cardParams(dish, params, chosen.id, servings)}`}>
          Print a recipe card
        </Link>
      </p>
      {v && !v.official && <ScaledNotice v={v} servings={servings} />}

      <div className={`cook ${loadingVariant ? "stale" : ""}`}>
        <section className="ingredients" aria-labelledby="ing-h">
          <h2 id="ing-h">Ingredients</h2>
          {v ? (
            <ul>
              {v.ingredients.map((i, n) => (
                <li key={n} className={i.shipped === false ? "pantry" : ""}>
                  {i.image ? (
                    <img src={i.image.small} alt="" loading="lazy" width={40} height={40} />
                  ) : (
                    <span className="dot" aria-hidden="true" />
                  )}
                  <span className="amount">
                    {i.scaled && (
                      <span className="approx" title="Scaled from the published amounts">
                        ≈
                      </span>
                    )}
                    {i.amount ?? ""}
                  </span>
                  <span className="name">
                    {i.name}
                    {i.shipped === false && <span className="tag">pantry</span>}
                  </span>
                </li>
              ))}
            </ul>
          ) : refusedRequest(variantQuery.error) ? (
            // The server answered but won't give this (a serving count out of range): say why, as it put it.
            <p className="empty">{variantQuery.error.message}</p>
          ) : variantQuery.isError ? (
            <p className="empty">
              Can't reach the server, so this version didn't load.{" "}
              <button type="button" className="link" onClick={() => variantQuery.refetch()}>
                Try again
              </button>
            </p>
          ) : (
            <p className="empty">Loading…</p>
          )}
          {v && allergens(v).length > 0 && (
            <p className="allergens">Contains: {allergens(v).join(", ")}</p>
          )}
        </section>

        <section className="steps" aria-labelledby="steps-h">
          <h2 id="steps-h">Steps</h2>
          {v && v.steps.length === 0 && <p className="empty">This version has no steps.</p>}
          <ol>
            {v?.steps.map((s) => (
              <li key={s.position}>
                {s.images[0] && (
                  <img src={s.images[0].large} alt="" loading="lazy" width={1200} height={800} />
                )}
                <div className="step-text">
                  <StepText text={s.text} />
                  {(s.check_amounts || s.scaled || s.check) && (
                    <p className="step-flag">
                      {s.check_amounts
                        ? "Check amounts: this step reads as published and may not match this serving count."
                        : s.scaled
                          ? "Amounts scaled for this serving count."
                          : "Wording follows the nearest published size."}
                    </p>
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
                <button
                  type="button"
                  className={x.id === chosen.id ? "current" : ""}
                  onClick={() => update(() => ({ v: x.id }))}
                >
                  <span>{x.title}</span>
                  <span className="meta">
                    {[
                      year(x.published_at),
                      x.yields.length && `serves ${x.yields.join("/")}`,
                      !x.has_steps && "no steps",
                    ]
                      .filter(Boolean)
                      .join(" · ")}
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

/** The year a version was published, or nothing. */
const year = (iso?: string | null) => (iso ? new Date(iso).getFullYear().toString() : "");

/** Why some amounts on the page are scaled, and which ones. */
function ScaledNotice({ v, servings }: { v: Variant; servings: number }) {
  const ingredients = v.ingredients.some((i) => i.scaled);
  const steps = v.steps.some((s) => s.scaled);
  const what =
    ingredients && steps
      ? "Ingredient amounts marked ≈ and amounts in marked steps"
      : ingredients
        ? "Amounts marked ≈"
        : "Amounts in marked steps";
  const why = !v.published_yield
    ? `${servings} servings isn't a published size.`
    : ingredients
      ? `Not every amount was published for ${servings} servings.`
      : `Ingredient amounts are as published, but the step text was only written for other sizes.`;
  return (
    <p className="notice">
      {why} {what} are scaled from the published ones and rounded up to standard measures. Check
      them as you go.
    </p>
  );
}
