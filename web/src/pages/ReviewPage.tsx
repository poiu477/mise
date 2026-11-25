import { useQuery } from "@tanstack/react-query";
import { useState, type FormEvent } from "react";
import { Link } from "react-router";
import {
  api,
  errorText,
  type ReviewDecision,
  type ReviewItem,
  type ReviewKind,
} from "../api/client";
import { answer, queryClient } from "../api/query";
import { useAccount, useSession } from "../session";

/** The kinds of question, in the order the page asks them, with a heading and what answering does. */
const KINDS: { kind: ReviewKind; title: string; about: string }[] = [
  {
    kind: "food_match",
    title: "Food matches",
    about:
      "Names that could mean more than one food. Accept keeps it as a food of its own; or say which food it is.",
  },
  {
    kind: "food_parent",
    title: "Food parents",
    about:
      "New foods that may be a kind of another. Kinds of a food share its shopping line and store section.",
  },
  {
    kind: "food_duplicate",
    title: "Near-duplicate foods",
    about:
      "Two foods whose names look like one food written twice. Merging folds the second into the first; or keep two foods, or file one under the other.",
  },
  {
    kind: "dish_merge",
    title: "Dish merges",
    about:
      "Copies too unlike their original to join its dish without asking. Dishes change the next time they're grouped (mise group-dishes).",
  },
];

/** What "reject" means for each kind, in the person's words; every other kind just rejects the proposal. */
const REJECTS: Partial<Record<ReviewKind, string>> = {
  dish_merge: "Keep apart",
  food_duplicate: "Two foods",
};

/** The review queue: what importing recipes left for an admin to decide. Each answer is kept the way the `mise`
 * curation commands keep theirs, so importing again doesn't ask again. Admins only; needs the server. */
export function ReviewPage() {
  const { session } = useSession();
  const account = useAccount();
  const admin = session.status === "in" && session.me.is_admin;
  const key = ["review", account];
  const queue = useQuery({
    queryKey: key,
    queryFn: ({ signal }) => answer(api.GET("/api/v1/review-items", { signal })),
    enabled: admin,
  });

  if (session.status === "offline")
    return (
      <p className="empty">The review queue needs the server. It'll be here when it's back.</p>
    );
  if (session.status !== "in") return null;
  if (!admin) return <p className="empty">Only admins review what importing left to decide.</p>;
  if (queue.isError) return <p className="empty">{queue.error.message}</p>;
  if (!queue.data) return <p className="empty">Loading…</p>;

  const items = queue.data.items;
  return (
    <article className="account review">
      <header className="account-head">
        <div>
          <h1>Review</h1>
          <p className="sub">What importing recipes left for a person to decide.</p>
        </div>
        <Link to="/account">Account</Link>
      </header>
      {items.length === 0 && <p className="empty">Nothing to review.</p>}
      {KINDS.map(({ kind, title, about }) => {
        const mine = items.filter((i) => i.kind === kind);
        if (mine.length === 0) return null;
        return (
          <section className="panel" key={kind} aria-labelledby={`review-${kind}`}>
            <h2 id={`review-${kind}`}>
              {title} <span className="tag">{mine.length}</span>
            </h2>
            <p className="sub">{about}</p>
            <ul className="review-items">
              {mine.map((item) => (
                <Item key={item.id} item={item} queryKey={key} />
              ))}
            </ul>
          </section>
        );
      })}
    </article>
  );
}

/** One open item: what it asks, and its answers. Editing a food asks which food it's a kind of, or the same as. */
function Item({ item, queryKey }: { item: ReviewItem; queryKey: unknown[] }) {
  const [editing, setEditing] = useState(false);
  const [how, setHow] = useState<"parent" | "same_as">(
    item.kind === "food_match" ? "same_as" : "parent",
  );
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const headingId = `item-${item.id}`;

  /** Sends one answer; the server's queue, with this item gone, replaces the page's. */
  const send = async (decision: ReviewDecision) => {
    setBusy(true);
    setError("");
    const { data, error: refused } = await api
      .POST("/api/v1/review-items/{id}/decision", {
        params: { path: { id: item.id } },
        body: decision,
      })
      .catch(() => ({
        data: undefined,
        error: { error: "can't reach the server, so that wasn't saved" },
      }));
    setBusy(false);
    if (data) queryClient.setQueryData(queryKey, data);
    else setError(errorText(refused));
  };
  /** Sends the edit: the food named as its parent, or as the same food. */
  const edit = (e: FormEvent) => {
    e.preventDefault();
    send({ decision: "edit", [how]: name.trim() });
  };

  return (
    <li className="review-item" aria-labelledby={headingId}>
      <h3 id={headingId}>{item.subject}</h3>
      <p>{item.proposal}.</p>
      {item.note && <p className="sub">{item.note}</p>}
      {item.dishes && (
        <p className="sub">
          <DishLink dish={item.dishes.version} /> and <DishLink dish={item.dishes.copied_from} />:
          titles {percent(item.dishes.title_similarity)} alike, shipped ingredients{" "}
          {percent(item.dishes.shipped_overlap)} shared.
        </p>
      )}
      {item.recipe_uses != null && (
        <p className="meta">
          In {item.recipe_uses} recipe {item.recipe_uses === 1 ? "line" : "lines"}
        </p>
      )}
      <div className="review-actions">
        {item.verdicts.includes("accept") && (
          <button
            type="button"
            className="primary"
            disabled={busy}
            onClick={() => send({ decision: "accept" })}
          >
            {item.kind === "food_duplicate" ? "One food" : "Accept"}
          </button>
        )}
        {item.verdicts.includes("reject") && (
          <button type="button" disabled={busy} onClick={() => send({ decision: "reject" })}>
            {REJECTS[item.kind] ?? "Reject"}
          </button>
        )}
        {item.verdicts.includes("edit") && (
          <button
            type="button"
            disabled={busy}
            aria-expanded={editing}
            onClick={() => setEditing(!editing)}
          >
            Something else…
          </button>
        )}
      </div>
      {editing && (
        <form className="form row" onSubmit={edit}>
          <label>
            {item.subject} is
            {/* A near-duplicate's "One food" button already merges the two, so only the parent is left to name. */}
            {item.kind === "food_duplicate" ? (
              " a kind of"
            ) : (
              <select value={how} onChange={(e) => setHow(e.target.value as typeof how)}>
                <option value="same_as">the same food as</option>
                <option value="parent">a kind of</option>
              </select>
            )}
          </label>
          <label>
            Food
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="Black Pepper"
              maxLength={200}
              required
            />
          </label>
          <button type="submit" disabled={busy}>
            Save
          </button>
        </form>
      )}
      {error && <p className="form-error">{error}</p>}
    </li>
  );
}

/** A version's title, linked to its dish when it's in one. */
function DishLink({ dish }: { dish: { title: string; dish_slug?: string | null } }) {
  return dish.dish_slug ? <Link to={`/dishes/${dish.dish_slug}`}>{dish.title}</Link> : dish.title;
}

/** 0.42 as "42%". */
const percent = (n: number) => `${Math.round(n * 100)}%`;
