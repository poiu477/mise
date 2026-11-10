import { useQuery } from "@tanstack/react-query";
import { useEffect } from "react";
import { Link, useParams, useSearchParams } from "react-router";
import { api } from "../api/client";
import { answer, Refused } from "../api/query";
import { startingServings, versionFor } from "../choose";
import { StepText } from "../components/StepText";
import { allergens, difficulty, minutes, servingsText } from "../recipe";

/** A recipe card to print: one version at one serving count, with its photo, ingredients and steps on a page.
 * The address takes the dish page's parameters (?v=, ?s= and the options), so the dish page links straight here;
 * the browser's print dialog makes the PDF. */
export function CardPage() {
  const { slug = "" } = useParams();
  const [params] = useSearchParams();

  const dishQuery = useQuery({
    queryKey: ["dish", slug],
    queryFn: ({ signal }) =>
      answer(api.GET("/api/dishes/{slug}", { params: { path: { slug } }, signal })),
  });
  const dish = dishQuery.data;
  const missing = dishQuery.error instanceof Refused && dishQuery.error.status === 404;
  const chosen = dish && versionFor(dish, params);
  const servings = Number(params.get("s")) || (chosen ? startingServings(chosen) : 2);

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
  });
  const v = variantQuery.data;

  // The print dialog names the PDF after the page's title.
  const title = dish ? `${dish.title}, ${servingsText(servings)}` : null;
  useEffect(() => {
    if (!title) return;
    const was = document.title;
    document.title = title;
    return () => {
      document.title = was;
    };
  }, [title]);

  if (missing) {
    return (
      <p className="empty">
        No such dish. <Link to="/">Back to all dishes</Link>
      </p>
    );
  }
  if ((dishQuery.isError && !dish) || variantQuery.isError) {
    return (
      <p className="empty">
        Can't reach the server, so this card didn't load.{" "}
        <button
          type="button"
          className="link"
          onClick={() => (dish ? variantQuery.refetch() : dishQuery.refetch())}
        >
          Try again
        </button>
      </p>
    );
  }
  if (!dish || !chosen || !v) return <p className="empty">Loading…</p>;

  const image = v.image ?? chosen.image;
  const subtitle = v.subtitle ?? chosen.subtitle;
  const contains = allergens(v);
  const prep = minutes(v.meta.prep_time) ?? minutes(v.meta.total_time);
  const facts = [servingsText(servings), prep && `${prep} min`, difficulty(v.meta.difficulty)];

  return (
    <article className="recipe-card">
      <div className="card-tools no-print">
        <Link to={`/dishes/${dish.slug}?${params}`}>Back to the dish</Link>
        <button type="button" className="primary" onClick={() => window.print()}>
          Print
        </button>
      </div>

      <header className="card-head">
        <h1>{dish.title}</h1>
        {subtitle && <p className="sub">{subtitle}</p>}
        {chosen.title !== dish.title && <p className="meta">This version: {chosen.title}</p>}
        <p className="card-facts">{facts.filter(Boolean).join(" · ")}</p>
      </header>

      <div className="recipe-body">
        <section className="card-ingredients" aria-labelledby="card-ing-h">
          {image && <img src={image.large} alt="" width={1200} height={800} />}
          <h2 id="card-ing-h">Ingredients</h2>
          <ul>
            {v.ingredients.map((i, n) => (
              <li key={n}>
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
          {contains.length > 0 && <p className="allergens">Contains: {contains.join(", ")}</p>}
        </section>

        <section className="card-steps" aria-labelledby="card-steps-h">
          <h2 id="card-steps-h">Steps</h2>
          {v.steps.length === 0 && <p className="meta">This version has no steps.</p>}
          <ol>
            {v.steps.map((s) => (
              <li key={s.position}>
                <StepText text={s.text} />
                {s.check_amounts ? (
                  <p className="step-flag">Check amounts: written for another serving count.</p>
                ) : (
                  s.scaled && <p className="step-flag">Amounts scaled for this serving count.</p>
                )}
              </li>
            ))}
          </ol>
        </section>
      </div>

      {!v.official && (
        <p className="card-note">
          {v.published_yield
            ? `Not everything was published for ${servingsText(servings)}.`
            : `${servingsText(servings)} isn't a published size.`}{" "}
          Amounts marked ≈ and those in marked steps are scaled and rounded up to standard measures.
        </p>
      )}
    </article>
  );
}
