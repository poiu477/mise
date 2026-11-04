import { useCallback, useEffect, useMemo, useRef, useState, type FormEvent } from "react";
import { Link, useSearchParams } from "react-router";
import { api, errorText, type ShoppingItem, type ShoppingList, type Weights } from "../api/client";
import { deviceCopy, isLocal, localId, queue, sendChanges, withChanges, type Change, type Sent } from "../offline";
import { useSession } from "../session";
import { addWeeks, mondayOf, weekLabel, weekOf } from "../week";
import { WEIGHTS, shortWeight } from "../weights";

type Reached<T> = { data: T } | { refused: unknown } | "unreachable";

/** A call's outcome. No answer, a server error or an ended session count as out of reach: worth trying again. */
async function reach<T>(call: Promise<{ data?: T; error?: unknown; response: Response }>): Promise<Reached<T>> {
  try {
    const r = await call;
    if (r.data !== undefined) return { data: r.data };
    return r.response.status >= 500 || r.response.status === 401 ? "unreachable" : { refused: r.error };
  } catch {
    return "unreachable";
  }
}

// A weak signal can hang rather than fail; give up and use the device's copy.
const patience = () => AbortSignal.timeout(8000);

/** Save this week's and next week's lists on the device, so they open in a shop with no signal. */
export async function keepListsOnDevice(user: string) {
  const copy = deviceCopy(user);
  for (const week of [mondayOf(), addWeeks(mondayOf(), 1)]) {
    const r = await reach(api.GET("/api/shopping", { params: { query: { week } }, signal: patience() }));
    if (r === "unreachable") return;
    if ("data" in r) copy.save(r.data);
  }
}

async function send(c: Change): Promise<Sent> {
  const sent = (r: Reached<ShoppingList>, then?: Change): Sent => (r === "unreachable" ? r : "data" in r ? { list: r.data, then } : "refused");
  switch (c.kind) {
    case "tick":
      return sent(await reach(api.PUT("/api/shopping/marks", { body: { week: c.week, key: c.key, checked: c.checked }, signal: patience() })));
    case "tick_extra":
      if (isLocal(c.id)) return "refused";
      return sent(await reach(api.PATCH("/api/shopping/extras/{id}", { params: { path: { id: c.id } }, body: { checked: c.checked }, signal: patience() })));
    case "add_extra": {
      const r = await reach(api.POST("/api/shopping/extras", { body: { week: c.week, name: c.name }, signal: patience() }));
      // Added; ticking it is a change of its own, so a lost answer can't add it twice.
      const made = r !== "unreachable" && "data" in r ? r.data.extras.filter((x) => x.name === c.name.trim()).at(-1) : undefined;
      return sent(r, c.checked && made ? { kind: "tick_extra", week: c.week, id: made.id, checked: true } : undefined);
    }
  }
}

const when = (t: number) => new Date(t).toLocaleString(undefined, { weekday: "short", hour: "numeric", minute: "2-digit" });

export function ShoppingPage() {
  const { session, refresh } = useSession();
  const user = session.status === "in" || session.status === "offline" ? session.me?.id : undefined;
  const copy = useMemo(() => (user ? deviceCopy(user) : null), [user]);
  const [params, setParams] = useSearchParams();
  const week = weekOf(params.get("week") ?? mondayOf());
  const [list, setList] = useState<ShoppingList | null>(null);
  // When the list on screen is the device's copy: when it was saved.
  const [savedAt, setSavedAt] = useState<number | null>(null);
  const [offline, setOffline] = useState(false);
  const [error, setError] = useState("");
  const [extra, setExtra] = useState("");
  const [open, setOpen] = useState<string | null>(null);
  const [pending, setPendingState] = useState<Change[]>(() => copy?.pending() ?? []);
  const pendingRef = useRef(pending);
  const setPending = useCallback(
    (next: Change[]) => {
      pendingRef.current = next;
      copy?.setPending(next);
      setPendingState(next);
    },
    [copy],
  );
  // Only the latest answer counts: quick ticks must not be undone by an older reply arriving late.
  const latest = useRef(0);
  const flushing = useRef(false);
  const flushRef = useRef<() => void>(() => {});

  const accept = useCallback(
    (l: ShoppingList) => {
      setList(l);
      copy?.save(l);
      setSavedAt(null);
      setOffline(false);
      if (session.status === "offline") refresh();
    },
    [copy, session.status, refresh],
  );

  const load = useCallback(async () => {
    const asked = ++latest.current;
    const r = await reach(api.GET("/api/shopping", { params: { query: { week } }, signal: patience() }));
    if (asked !== latest.current) return;
    if (r === "unreachable") {
      const saved = copy?.saved(week);
      setOffline(true);
      if (saved) {
        setList(saved.list);
        setSavedAt(saved.savedAt);
      } else setList((l) => (l?.week === week ? l : null));
      return;
    }
    if ("data" in r) {
      setError("");
      accept(r.data);
      if (pendingRef.current.length) flushRef.current();
    } else setError(errorText(r.refused));
  }, [week, copy, accept]);

  /** Send what is waiting; once everything is through, fetch the list afresh. */
  const flush = useCallback(async () => {
    if (flushing.current) return;
    flushing.current = true;
    try {
      while (pendingRef.current.length) {
        const sent = pendingRef.current;
        const r = await sendChanges(sent, send);
        const since = pendingRef.current.filter((c) => !sent.includes(c));
        setPending([...r.remaining, ...since]);
        if (r.refused) setError("Some changes couldn't be saved: the list changed on the server. It shows what the server has now.");
        if (r.remaining.length) {
          setOffline(true);
          return;
        }
      }
    } finally {
      flushing.current = false;
    }
    load();
  }, [setPending, load]);
  useEffect(() => {
    flushRef.current = flush;
  }, [flush]);

  // Another account on this device has its own waiting changes.
  useEffect(() => {
    const mine = copy?.pending() ?? [];
    pendingRef.current = mine;
    setPendingState(mine);
  }, [copy]);

  useEffect(() => {
    setList((l) => (l?.week === week ? l : null));
    load();
    // Someone else in the household may be shopping too, and the signal comes and goes.
    const refreshList = () => document.visibilityState === "visible" && load();
    document.addEventListener("visibilitychange", refreshList);
    window.addEventListener("online", refreshList);
    return () => {
      document.removeEventListener("visibilitychange", refreshList);
      window.removeEventListener("online", refreshList);
    };
  }, [week, load]);

  // While offline, look for the server now and then.
  useEffect(() => {
    if (!offline) return;
    const t = setInterval(load, 30000);
    return () => clearInterval(t);
  }, [offline, load]);

  const change = (c: Change) => {
    setPending(queue(pendingRef.current, c));
    flush();
  };

  /** Things that need the server there and then: removing, staples, units. */
  const run = async (call: Promise<{ data?: ShoppingList; error?: unknown; response: Response }>) => {
    const asked = ++latest.current;
    setError("");
    const r = await reach(call);
    if (asked !== latest.current) return;
    if (r === "unreachable") {
      setOffline(true);
      setError("Can't reach the server, so that wasn't saved. Ticks and added items still work offline.");
    } else if ("data" in r) accept(r.data);
    else {
      setError(errorText(r.refused));
      load();
    }
  };

  const shown = list && withChanges(list, pending);
  const tick = (item: ShoppingItem, checked: boolean) => shown && change({ kind: "tick", week: shown.week, key: item.key, checked });
  const tickExtra = (id: string, checked: boolean) => shown && change({ kind: "tick_extra", week: shown.week, id, checked });
  const deleteExtra = (id: string) => {
    if (isLocal(id)) setPending(pendingRef.current.filter((c) => !(c.kind === "add_extra" && c.id === id)));
    else run(api.DELETE("/api/shopping/extras/{id}", { params: { path: { id } } }));
  };
  const hide = (key: string, hidden: boolean) => run(api.PUT("/api/shopping/marks", { body: { week, key, hidden } }));
  const always = (foodId: string) => run(api.POST("/api/pantry-staples", { body: { week, food_id: foodId } }));
  const weighIn = (foodId: string, weight_unit: Weights | null) => run(api.PUT("/api/shopping/food-weights", { body: { week, food_id: foodId, weight_unit } }));
  const unstaple = (foodId: string) => run(api.DELETE("/api/pantry-staples/{food_id}", { params: { path: { food_id: foodId }, query: { week } } }));
  const addExtra = (e: FormEvent) => {
    e.preventDefault();
    if (!extra.trim() || !shown) return;
    change({ kind: "add_extra", week: shown.week, id: localId(), name: extra.trim(), checked: false });
    setExtra("");
  };

  const items = shown?.sections.flatMap((s) => s.items) ?? [];
  const total = items.length + (shown?.extras.length ?? 0);
  const left = items.filter((i) => !i.checked).length + (shown?.extras.filter((x) => !x.checked).length ?? 0);
  const waiting = pending.length;

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
      {offline && (
        <p className="notice no-print" role="status">
          {shown ? (savedAt ? `Offline: this is the list as of ${when(savedAt)}.` : "Offline.") : "Offline, and this week's list isn't saved on this device yet."}{" "}
          {shown && (waiting ? `${waiting} ${waiting === 1 ? "change" : "changes"} will be sent when the server is back in reach.` : "Ticks and added items are kept here until then.")}
        </p>
      )}
      {error && <p className="notice">{error}</p>}
      {!shown && !error && !offline && <p className="empty">Loading…</p>}
      {shown && shown.meals === 0 && shown.extras.length === 0 && (
        <p className="empty">
          Nothing planned for this week yet. <Link to={`/plan?week=${week}`}>Plan some dishes</Link> and the list fills itself in.
        </p>
      )}

      {shown && (
        <form className="add-extra no-print" onSubmit={addExtra}>
          <input value={extra} onChange={(e) => setExtra(e.target.value)} placeholder="Add something else" aria-label="Add something else" maxLength={200} enterKeyHint="done" />
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
                  <input type="checkbox" checked={x.checked} onChange={(e) => tickExtra(x.id, e.target.checked)} />
                  <span className="item-name">
                    {x.name}
                    {isLocal(x.id) && <span className="item-amount meta">not sent yet</span>}
                  </span>
                </label>
                <div className="item-actions no-print">
                  <button type="button" className="quiet" onClick={() => deleteExtra(x.id)} disabled={offline && !isLocal(x.id)} aria-label={`Delete ${x.name}`}>
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
                    {i.weighed && i.food_id && (
                      <label className="weigh-in">
                        Weigh in
                        <select value={i.weight_unit ?? ""} disabled={offline} onChange={(e) => weighIn(i.food_id!, (e.target.value || null) as Weights | null)}>
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
                        <button type="button" className="quiet" onClick={() => always(i.food_id!)} disabled={offline} title="Leave this off every list" aria-label={`Always have ${i.name}`}>
                          Always have
                        </button>
                      )}
                      <button type="button" className="quiet" onClick={() => hide(i.key, true)} disabled={offline} title="Take it off this week's list" aria-label={`Remove ${i.name}`}>
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
                <button type="button" className="quiet" onClick={() => unstaple(l.key)} disabled={offline} aria-label={`Put ${l.name} back`}>
                  Put back
                </button>
              </li>
            ))}
            {shown.removed.map((l) => (
              <li key={l.key}>
                <span>{l.name}</span>
                <span className="meta">this week</span>
                <button type="button" className="quiet" onClick={() => hide(l.key, false)} disabled={offline} aria-label={`Put ${l.name} back`}>
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
