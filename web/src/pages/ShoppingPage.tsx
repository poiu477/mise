import { useQuery } from "@tanstack/react-query";
import {
  useCallback,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type FormEvent,
} from "react";
import { Link, useSearchParams } from "react-router";
import { api, errorText, type ShoppingItem, type ShoppingList, type Weights } from "../api/client";
import { queryClient } from "../api/query";
import {
  currentSignIn,
  deviceCopy,
  isLocal,
  localId,
  onSignInChange,
  queue,
  sendChanges,
  withChanges,
  type Change,
  type Sent,
} from "../offline";
import { useSession } from "../session";
import { addWeeks, mondayOf, weekLabel, weekOf } from "../week";
import { WEIGHTS, shortWeight } from "../weights";

/** What a call came to: the data, a refusal with the server's error, or no usable answer. */
type Reached<T> = { data: T } | { refused: unknown } | "unreachable";

/** A call's outcome. No answer, a server error or an ended session count as out of reach: worth trying again.
 * So does a 412, work made for an account that's no longer the one signed in: it waits until that one is back. */
async function reach<T>(
  call: Promise<{ data?: T; error?: unknown; response: Response }>,
): Promise<Reached<T>> {
  try {
    const r = await call;
    if (r.data !== undefined) return { data: r.data };
    const status = r.response.status;
    return status >= 500 || status === 401 || status === 412 ? "unreachable" : { refused: r.error };
  } catch {
    return "unreachable";
  }
}

/** A time limit for a call: a weak signal can hang rather than fail, so give up and use the device's copy. */
const patience = () => AbortSignal.timeout(8000);

/** Save this week's and next week's lists on the device, so they open in a shop with no signal. */
export async function keepListsOnDevice(user: string) {
  const copy = deviceCopy(user);
  for (const week of [mondayOf(), addWeeks(mondayOf(), 1)]) {
    const r = await reach(
      api.GET("/api/shopping", { params: { query: { week } }, signal: patience() }),
    );
    if (r === "unreachable") return;
    if ("data" in r) copy.save(r.data);
  }
}

/** Sends one waiting change. Adding something that was ticked offline goes as two changes, the add and then
 * the tick, so a lost answer can't add it twice. */
async function send(c: Change): Promise<Sent> {
  const sent = (r: Reached<ShoppingList>, then?: Change): Sent =>
    r === "unreachable" ? r : "data" in r ? { list: r.data, then } : "refused";
  switch (c.kind) {
    case "tick":
      return sent(
        await reach(
          api.PUT("/api/shopping/marks", {
            body: { week: c.week, key: c.key, checked: c.checked },
            signal: patience(),
          }),
        ),
      );
    case "tick_extra":
      if (isLocal(c.id)) return "refused";
      return sent(
        await reach(
          api.PATCH("/api/shopping/extras/{id}", {
            params: { path: { id: c.id } },
            body: { checked: c.checked },
            signal: patience(),
          }),
        ),
      );
    case "add_extra": {
      const r = await reach(
        api.POST("/api/shopping/extras", {
          body: { week: c.week, name: c.name },
          signal: patience(),
        }),
      );
      // Added; ticking it is a change of its own, so a lost answer can't add it twice.
      const made =
        r !== "unreachable" && "data" in r
          ? r.data.extras.filter((x) => x.name === c.name.trim()).at(-1)
          : undefined;
      return sent(
        r,
        c.checked && made
          ? { kind: "tick_extra", week: c.week, id: made.id, checked: true }
          : undefined,
      );
    }
  }
}

/** When the device's copy was saved, for the offline notice: "Sat 2:15 PM". */
const when = (t: number) =>
  new Date(t).toLocaleString(undefined, { weekday: "short", hour: "numeric", minute: "2-digit" });

/** The list as last heard: from the server (`reached`), or else the device's copy (`savedAt`, when it was saved). */
type Seen = { list: ShoppingList | null; savedAt: number | null; reached: boolean };

/** The week's shopping list (?week=), which keeps working with no signal: ticks and added items wait on the
 * device and are sent once the server is back. */
export function ShoppingPage() {
  const { session } = useSession();
  const user = session.status === "in" || session.status === "offline" ? session.me?.id : undefined;
  // A new sign-in in another tab replaces this page's device copy; work begun under the old one stays blocked.
  const signIn = useSyncExternalStore(onSignInChange, () => currentSignIn());
  if (!user) return null;
  // Each account and sign-in starts afresh, with that copy's own waiting changes.
  return <WeekList key={`${user} ${signIn}`} user={user} />;
}

/** The list as one account and sign-in sees it, with that sign-in's own device copy and waiting changes. */
function WeekList({ user }: { user: string }) {
  const { session, refresh } = useSession();
  const [copy] = useState(() => deviceCopy(user));
  const [params, setParams] = useSearchParams();
  const week = weekOf(params.get("week") ?? mondayOf());
  const key = ["shopping", user, week];
  const [error, setError] = useState("");
  const [extra, setExtra] = useState("");
  const [open, setOpen] = useState<string | null>(null);
  const [pending, setPendingState] = useState<Change[]>(() => copy.pending());
  const pendingRef = useRef(pending);
  const setPending = useCallback(
    (next: Change[]) => {
      pendingRef.current = next;
      copy.setPending(next);
      setPendingState(next);
    },
    [copy],
  );
  // Only the latest answer counts: quick ticks must not be undone by an older reply arriving late.
  const latest = useRef(0);
  const flushing = useRef(false);
  const flushRef = useRef<() => void>(() => {});

  /** The list now on screen, as the cache has it. */
  const current = (): Seen =>
    queryClient.getQueryData<Seen>(key) ?? { list: null, savedAt: null, reached: true };
  /** The server's answer: kept on the device, and the session checked again if it thought it was offline. */
  const keep = (l: ShoppingList) => {
    copy.save(l);
    if (session.status === "offline") refresh();
  };
  /** Takes the server's list as the one to show, and keeps it on the device. */
  const accept = (l: ShoppingList) => {
    keep(l);
    queryClient.setQueryData<Seen>(key, { list: l, savedAt: null, reached: true });
  };
  /** Out of reach: the list stays as it is, marked offline. */
  const unreached = () => queryClient.setQueryData<Seen>(key, { ...current(), reached: false });

  // Loads when the page opens or the week changes, when the page comes back into view (someone else in the
  // household may be shopping too) and when the signal returns; while offline, every 30 seconds.
  const seen = useQuery({
    queryKey: key,
    queryFn: async (): Promise<Seen> => {
      const asked = ++latest.current;
      const r = await reach(
        api.GET("/api/shopping", { params: { query: { week } }, signal: patience() }),
      );
      // A change made since answered with a newer list.
      if (asked !== latest.current) return current();
      if (r === "unreachable") {
        const saved = copy.saved(week);
        return saved
          ? { list: saved.list, savedAt: saved.savedAt, reached: false }
          : { ...current(), reached: false };
      }
      if ("refused" in r) {
        setError(errorText(r.refused));
        return current();
      }
      setError("");
      keep(r.data);
      if (pendingRef.current.length) flushRef.current();
      return { list: r.data, savedAt: null, reached: true };
    },
    refetchInterval: (q) => (q.state.data?.reached === false ? 30_000 : false),
  });
  const list = seen.data?.list ?? null;
  const savedAt = seen.data?.savedAt ?? null;
  const offline = seen.data?.reached === false;
  /** Fetches the list afresh (every week of it cached for this account). */
  const load = () => queryClient.invalidateQueries({ queryKey: ["shopping", user] });

  /** Send what is waiting; once everything is through, fetch the list afresh. */
  const flush = async () => {
    if (flushing.current) return;
    flushing.current = true;
    try {
      while (pendingRef.current.length) {
        const sent = pendingRef.current;
        const r = await sendChanges(sent, send);
        const since = pendingRef.current.filter((c) => !sent.includes(c));
        setPending([...r.remaining, ...since]);
        if (r.refused)
          setError(
            "Some changes couldn't be saved: the list changed on the server. It shows what the server has now.",
          );
        if (r.remaining.length) {
          unreached();
          return;
        }
      }
    } finally {
      flushing.current = false;
    }
    load();
  };
  // A layout effect runs in the same task as the render's commit, before any awaited answer can resume, so a
  // request finishing just after a week change never calls the previous week's flush.
  useLayoutEffect(() => {
    flushRef.current = flush;
  });

  /** Queues a tick or an addition on the device, then tries to send it. */
  const change = (c: Change) => {
    setPending(queue(pendingRef.current, c));
    flush();
  };

  /** Things that need the server there and then: removing, staples, units. */
  const run = async (
    call: Promise<{ data?: ShoppingList; error?: unknown; response: Response }>,
  ) => {
    const asked = ++latest.current;
    setError("");
    const r = await reach(call);
    if (asked !== latest.current) return;
    if (r === "unreachable") {
      unreached();
      setError(
        "Can't reach the server, so that wasn't saved. Ticks and added items still work offline.",
      );
    } else if ("data" in r) accept(r.data);
    else {
      setError(errorText(r.refused));
      load();
    }
  };

  const shown = list && withChanges(list, pending);
  const tick = (item: ShoppingItem, checked: boolean) =>
    shown && change({ kind: "tick", week: shown.week, key: item.key, checked });
  const tickExtra = (id: string, checked: boolean) =>
    shown && change({ kind: "tick_extra", week: shown.week, id, checked });
  /** Deletes a hand-added item: one not sent yet just leaves the queue; one the server has needs the server. */
  const deleteExtra = (id: string) => {
    if (isLocal(id))
      setPending(pendingRef.current.filter((c) => !(c.kind === "add_extra" && c.id === id)));
    else run(api.DELETE("/api/shopping/extras/{id}", { params: { path: { id } } }));
  };
  const hide = (key: string, hidden: boolean) =>
    run(api.PUT("/api/shopping/marks", { body: { week, key, hidden } }));
  const always = (foodId: string) =>
    run(api.POST("/api/pantry-staples", { body: { week, food_id: foodId } }));
  const weighIn = (foodId: string, weight_unit: Weights | null) =>
    run(api.PUT("/api/shopping/food-weights", { body: { week, food_id: foodId, weight_unit } }));
  const unstaple = (foodId: string) =>
    run(
      api.DELETE("/api/pantry-staples/{food_id}", {
        params: { path: { food_id: foodId }, query: { week } },
      }),
    );
  /** Adds something by hand; it shows at once and waits on the device until sent. */
  const addExtra = (e: FormEvent) => {
    e.preventDefault();
    if (!extra.trim() || !shown) return;
    change({
      kind: "add_extra",
      week: shown.week,
      id: localId(),
      name: extra.trim(),
      checked: false,
    });
    setExtra("");
  };

  const items = shown?.sections.flatMap((s) => s.items) ?? [];
  const total = items.length + (shown?.extras.length ?? 0);
  const left =
    items.filter((i) => !i.checked).length + (shown?.extras.filter((x) => !x.checked).length ?? 0);
  const waiting = pending.length;

  return (
    <div className="shopping">
      <header className="week-head">
        <div className="week-nav no-print">
          <button
            type="button"
            onClick={() => setParams({ week: addWeeks(week, -1) })}
            aria-label="Previous week"
          >
            ‹
          </button>
          <h1>{weekLabel(week)}</h1>
          <button
            type="button"
            onClick={() => setParams({ week: addWeeks(week, 1) })}
            aria-label="Next week"
          >
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
      {offline && (
        <p className="notice no-print" role="status">
          {shown
            ? savedAt
              ? `Offline: this is the list as of ${when(savedAt)}.`
              : "Offline."
            : "Offline, and this week's list isn't saved on this device yet."}{" "}
          {shown &&
            (waiting
              ? `${waiting} ${waiting === 1 ? "change" : "changes"} will be sent when the server is back in reach.`
              : "Ticks and added items are kept here until then.")}
        </p>
      )}
      {error && <p className="notice">{error}</p>}
      {!shown && !error && !offline && <p className="empty">Loading…</p>}
      {shown && shown.meals === 0 && shown.extras.length === 0 && (
        <p className="empty">
          Nothing planned for this week yet. <Link to={`/plan?week=${week}`}>Plan some dishes</Link>{" "}
          and the list fills itself in.
        </p>
      )}

      {shown && (
        <form className="add-extra no-print" onSubmit={addExtra}>
          <input
            value={extra}
            onChange={(e) => setExtra(e.target.value)}
            placeholder="Add something else"
            aria-label="Add something else"
            maxLength={200}
            enterKeyHint="done"
          />
          <button type="submit" disabled={!extra.trim()}>
            Add
          </button>
        </form>
      )}

      {shown && shown.extras.length > 0 && (
        <section className="shop-section">
          <h2>Added by you</h2>
          <ul>
            {shown.extras.map((x) => (
              <li key={x.id} className={x.checked ? "done" : ""}>
                <label>
                  <input
                    type="checkbox"
                    checked={x.checked}
                    onChange={(e) => tickExtra(x.id, e.target.checked)}
                  />
                  <span className="item-name">
                    {x.name}
                    {isLocal(x.id) && <span className="item-amount meta">not sent yet</span>}
                  </span>
                </label>
                <div className="item-actions no-print">
                  <button
                    type="button"
                    className="quiet"
                    onClick={() => deleteExtra(x.id)}
                    disabled={offline && !isLocal(x.id)}
                    aria-label={`Delete ${x.name}`}
                  >
                    Delete
                  </button>
                </div>
              </li>
            ))}
          </ul>
        </section>
      )}

      {shown?.sections.map((s) => (
        <section className="shop-section" key={s.name}>
          <h2>{s.name}</h2>
          <ul>
            {s.items.map((i) => (
              <li key={i.key} className={i.checked ? "done" : ""}>
                <label>
                  <input
                    type="checkbox"
                    checked={i.checked}
                    onChange={(e) => tick(i, e.target.checked)}
                  />
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
                    {i.weighed && i.food_id && (
                      <label className="weigh-in">
                        Weigh in
                        <select
                          value={i.weight_unit ?? ""}
                          disabled={offline}
                          onChange={(e) =>
                            weighIn(i.food_id!, (e.target.value || null) as Weights | null)
                          }
                        >
                          <option value="">Usual ({shortWeight(shown.weight_unit)})</option>
                          {WEIGHTS.map((w) => (
                            <option key={w.value} value={w.value}>
                              {w.short}
                            </option>
                          ))}
                        </select>
                      </label>
                    )}
                    <div className="item-actions">
                      {i.food_id && (
                        <button
                          type="button"
                          className="quiet"
                          onClick={() => always(i.food_id!)}
                          disabled={offline}
                          title="Leave this off every list"
                          aria-label={`Always have ${i.name}`}
                        >
                          Always have
                        </button>
                      )}
                      <button
                        type="button"
                        className="quiet"
                        onClick={() => hide(i.key, true)}
                        disabled={offline}
                        title="Take it off this week's list"
                        aria-label={`Remove ${i.name}`}
                      >
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

      {shown && shown.removed.length + shown.staples.length > 0 && (
        <section className="left-out no-print">
          <h2>Left off this list</h2>
          <ul>
            {shown.staples.map((l) => (
              <li key={l.key}>
                <span>{l.name}</span>
                <span className="meta">always on hand</span>
                <button
                  type="button"
                  className="quiet"
                  onClick={() => unstaple(l.key)}
                  disabled={offline}
                  aria-label={`Put ${l.name} back`}
                >
                  Put back
                </button>
              </li>
            ))}
            {shown.removed.map((l) => (
              <li key={l.key}>
                <span>{l.name}</span>
                <span className="meta">this week</span>
                <button
                  type="button"
                  className="quiet"
                  onClick={() => hide(l.key, false)}
                  disabled={offline}
                  aria-label={`Put ${l.name} back`}
                >
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
