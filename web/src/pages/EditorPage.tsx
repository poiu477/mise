import { keepPreviousData, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState, type FormEvent } from "react";
import { Link, useNavigate, useParams } from "react-router";
import { api, errorText, type FoodRef, type Recipe } from "../api/client";
import { answer, Refused } from "../api/query";
import { draftOf, inputOf, linesOf, readingText, type Draft } from "../editor";
import { useAccount, useSession } from "../session";

/** `value` once it has stopped changing for `ms`, so typing asks the server once it pauses, not on every key. */
function useSettled<T>(value: T, ms: number): T {
  const [settled, setSettled] = useState(value);
  useEffect(() => {
    const timer = setTimeout(() => setSettled(value), ms);
    return () => clearTimeout(timer);
  }, [value, ms]);
  return settled;
}

/** Write a recipe (`/recipes/new`) or change one the household wrote (`/recipes/:id/edit`). Needs the server. */
export function EditorPage() {
  const { id } = useParams();
  const { session } = useSession();
  const account = useAccount();
  const signedIn = session.status === "in" && !session.me.kitchen;
  const stored = useQuery({
    queryKey: ["recipe", account, id],
    queryFn: ({ signal }) =>
      answer(api.GET("/api/v1/recipes/{id}", { params: { path: { id: id! } }, signal })),
    enabled: !!id && signedIn,
    // The boxes are filled from the recipe as it is now, never from an earlier visit's copy.
    gcTime: 0,
  });

  if (session.status === "offline")
    return (
      <p className="empty">Writing a recipe needs the server. It'll be here when it's in reach.</p>
    );
  if (!signedIn) return <p className="empty">A kitchen device can't write recipes.</p>;
  if (!id) return <Editor key="new" recipe={null} />;
  if (stored.error instanceof Refused && stored.error.code === "not_found")
    return (
      <p className="empty">
        No such recipe, or not one your household wrote. <Link to="/">Back to all dishes</Link>
      </p>
    );
  if (stored.isError)
    return (
      <p className="empty">
        Can't reach the server, so this recipe didn't load.{" "}
        <button type="button" className="link" onClick={() => stored.refetch()}>
          Try again
        </button>
      </p>
    );
  if (!stored.data) return <p className="empty">Loading…</p>;
  return <Editor key={id} recipe={stored.data} />;
}

/** The form itself, starting from `recipe` or from nothing. */
function Editor({ recipe }: { recipe: Recipe | null }) {
  const account = useAccount();
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  // What the server holds: the recipe opened, then whatever the last save stored (a new recipe exists once its
  // first save succeeds, even when its photo then fails).
  const [stored, setStored] = useState(recipe);
  const [draft, setDraft] = useState<Draft>(() => draftOf(recipe));
  const [photo, setPhoto] = useState<File | null>(null);
  const [dropPhoto, setDropPhoto] = useState(false);
  const [picking, setPicking] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");

  /** Changes one box. */
  const set = (field: keyof Omit<Draft, "picks">) => (value: string) =>
    setDraft((d) => ({ ...d, [field]: value }));

  // Each ingredient line as the server reads it, asked for once typing pauses.
  const lines = useMemo(() => linesOf(draft.ingredients), [draft.ingredients]);
  const settled = useSettled(draft.ingredients, 400);
  const asked = useMemo(() => linesOf(settled), [settled]);
  const read = useQuery({
    queryKey: ["recipe-lines", account, asked],
    queryFn: ({ signal }) =>
      answer(api.POST("/api/v1/recipes/lines", { body: { lines: asked }, signal })),
    enabled: asked.length > 0,
    // The last reading stays on screen while the next one loads.
    placeholderData: keepPreviousData,
    staleTime: 60_000,
  });
  const readings = useMemo(
    () => new Map((read.data?.lines ?? []).map((line) => [line.text, line])),
    [read.data],
  );

  /** Says which food a line is (null: match it by its name again). */
  const pick = (text: string, food: FoodRef | null) => {
    setDraft((d) => {
      const picks = { ...d.picks };
      if (food) picks[text] = food;
      else delete picks[text];
      return { ...d, picks };
    });
    setPicking(null);
  };

  // The photo on show: the one just chosen, else the stored one unless it is being taken off.
  const preview = useMemo(() => (photo ? URL.createObjectURL(photo) : null), [photo]);
  useEffect(
    () => () => {
      if (preview) URL.revokeObjectURL(preview);
    },
    [preview],
  );
  const shown = preview ?? (dropPhoto ? null : (stored?.image?.small ?? null));

  /** Forgets what the pages hold of dishes and versions, so they show the recipe as saved. */
  const refresh = () =>
    Promise.all(
      ["dishes", "dish", "variant"].map((key) =>
        queryClient.invalidateQueries({ queryKey: [key] }),
      ),
    );

  /** Saves the recipe, then its photo, then opens its dish. A failure says what wasn't saved and stays here. */
  const save = async (e: FormEvent) => {
    e.preventDefault();
    setSaving(true);
    setError("");
    const unreachable = () => ({
      data: undefined,
      error: { error: "can't reach the server, so that wasn't saved" },
    });
    const body = inputOf(draft);
    const sent = stored
      ? await api
          .PUT("/api/v1/recipes/{id}", { params: { path: { id: stored.id } }, body })
          .catch(unreachable)
      : await api.POST("/api/v1/recipes", { body }).catch(unreachable);
    if (!sent.data) {
      setSaving(false);
      return setError(errorText(sent.error));
    }
    let saved = sent.data;
    setStored(saved);
    const path = { id: saved.id };
    if (photo) {
      const put = await api
        .PUT("/api/v1/recipes/{id}/photo", {
          params: { path },
          // The file itself is the body; the generated types call it text.
          body: photo as unknown as string,
          bodySerializer: (file) => file as unknown as Blob,
          headers: { "Content-Type": photo.type || "application/octet-stream" },
        })
        .catch(unreachable);
      if (!put.data) {
        setSaving(false);
        await refresh();
        return setError(`The recipe is saved, but its photo isn't. ${errorText(put.error)}`);
      }
      saved = put.data;
    } else if (dropPhoto && saved.image) {
      const gone = await api
        .DELETE("/api/v1/recipes/{id}/photo", { params: { path } })
        .catch(unreachable);
      if (!gone.data) {
        setSaving(false);
        await refresh();
        return setError(
          `The recipe is saved, but its photo is still on it. ${errorText(gone.error)}`,
        );
      }
      saved = gone.data;
    }
    await refresh();
    navigate(`/dishes/${saved.dish_slug}`);
  };

  /** Deletes the recipe for good, after asking. */
  const remove = async () => {
    if (!stored) return;
    const sure = `Delete "${stored.title}"? It comes off any plan it's on, and can't be brought back.`;
    if (!window.confirm(sure)) return;
    setError("");
    const { error: failed } = await api
      .DELETE("/api/v1/recipes/{id}", { params: { path: { id: stored.id } } })
      .catch(() => ({ error: { error: "can't reach the server, so it wasn't deleted" } }));
    if (failed) return setError(errorText(failed));
    await refresh();
    navigate("/");
  };

  return (
    <article className="editor">
      <h1>{stored ? "Edit recipe" : "Write a recipe"}</h1>
      <form className="form" onSubmit={save}>
        <label>
          Title
          <input
            value={draft.title}
            onChange={(e) => set("title")(e.target.value)}
            maxLength={200}
            required
          />
        </label>
        <label>
          Subtitle
          <span className="hint">Optional: "with Garlic and Parmesan"</span>
          <input
            value={draft.subtitle}
            onChange={(e) => set("subtitle")(e.target.value)}
            maxLength={200}
          />
        </label>
        <label>
          Description
          <span className="hint">Optional</span>
          <textarea
            value={draft.description}
            onChange={(e) => set("description")(e.target.value)}
            maxLength={4000}
            rows={2}
          />
        </label>
        <label className="servings-field">
          Servings
          <span className="hint">How many the amounts below make</span>
          <input
            type="number"
            inputMode="decimal"
            min={0.5}
            max={1000}
            step="any"
            value={draft.servings}
            onChange={(e) => set("servings")(e.target.value)}
            required
          />
        </label>

        <label>
          Ingredients
          <span className="hint">One to a line, as you'd write them: 2 cups flour, sifted</span>
          <textarea
            value={draft.ingredients}
            onChange={(e) => set("ingredients")(e.target.value)}
            rows={8}
          />
        </label>
        {lines.length > 0 && (
          <section className="readings" aria-label="How each ingredient reads">
            <p className="hint">
              How each line reads, and the food it counts as on the shopping list.
            </p>
            <ul className="reading-list">
              {lines.map((text, n) => {
                const reading = readings.get(text);
                const food = draft.picks[text] ?? reading?.food ?? null;
                return (
                  <li key={`${n}:${text}`}>
                    <span className="reading">{reading ? readingText(reading) : text}</span>
                    <span className="food">
                      {food ? food.name : reading ? <span className="tag">new food</span> : "…"}
                    </span>
                    <button
                      type="button"
                      aria-expanded={picking === text}
                      aria-label={`Change the food for ${reading?.name || text}`}
                      onClick={() => setPicking(picking === text ? null : text)}
                    >
                      {picking === text ? "Close" : "Change food"}
                    </button>
                    {picking === text && (
                      <FoodPicker
                        onPick={(found) => pick(text, found)}
                        onMatch={draft.picks[text] ? () => pick(text, null) : undefined}
                      />
                    )}
                  </li>
                );
              })}
            </ul>
          </section>
        )}

        <label>
          Steps
          <span className="hint">One to a line</span>
          <textarea value={draft.steps} onChange={(e) => set("steps")(e.target.value)} rows={8} />
        </label>

        <div className="editor-photo">
          <label>
            Photo
            <span className="hint">Optional: a JPEG, PNG or WebP, up to 20 MB</span>
            <input
              type="file"
              accept="image/jpeg,image/png,image/webp,image/gif"
              onChange={(e) => {
                setPhoto(e.target.files?.[0] ?? null);
                setDropPhoto(false);
              }}
            />
          </label>
          {shown && <img src={shown} alt="The recipe's photo" />}
          {shown && (
            <button
              type="button"
              onClick={() => {
                setPhoto(null);
                setDropPhoto(true);
              }}
            >
              Remove photo
            </button>
          )}
        </div>

        {error && (
          <p className="form-error" role="alert">
            {error}
          </p>
        )}
        <div className="editor-actions">
          <button type="submit" className="primary" disabled={saving}>
            {saving ? "Saving…" : "Save recipe"}
          </button>
          <Link to={stored ? `/dishes/${stored.dish_slug}` : "/"}>Cancel</Link>
          {stored && (
            <button type="button" className="danger" onClick={remove}>
              Delete recipe
            </button>
          )}
        </div>
      </form>
    </article>
  );
}

/** A search box over the foods, to say which one an ingredient line is. `onMatch`, when the line has a pick,
 * goes back to matching it by its name. */
function FoodPicker({
  onPick,
  onMatch,
}: {
  onPick: (food: FoodRef) => void;
  onMatch?: () => void;
}) {
  const account = useAccount();
  const [q, setQ] = useState("");
  const asked = useSettled(q.trim(), 250);
  const found = useQuery({
    queryKey: ["foods", account, asked],
    queryFn: ({ signal }) =>
      answer(api.GET("/api/v1/foods", { params: { query: { q: asked } }, signal })),
    enabled: asked !== "",
    placeholderData: keepPreviousData,
    staleTime: 60_000,
  });
  const foods = asked ? (found.data?.items ?? []) : [];
  return (
    <div className="food-picker">
      <label>
        Find a food
        <input
          type="search"
          value={q}
          onChange={(e) => setQ(e.target.value)}
          placeholder="parmesan"
          autoFocus
          // Enter here picks nothing and must not save the recipe.
          onKeyDown={(e) => e.key === "Enter" && e.preventDefault()}
        />
      </label>
      {foods.length > 0 && (
        <ul>
          {foods.map((food) => (
            <li key={food.id}>
              <button type="button" onClick={() => onPick(food)}>
                {food.name}
              </button>
            </li>
          ))}
        </ul>
      )}
      {asked && found.isSuccess && !found.isPlaceholderData && foods.length === 0 && (
        <p className="hint">No food by that name.</p>
      )}
      {found.isError && <p className="hint">Can't reach the server.</p>}
      {onMatch && (
        <button type="button" onClick={onMatch}>
          Match by name instead
        </button>
      )}
    </div>
  );
}
