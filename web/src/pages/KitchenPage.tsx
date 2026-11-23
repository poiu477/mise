import { useQuery } from "@tanstack/react-query";
import { useEffect, useRef, useState, type PointerEvent } from "react";
import { Link, Outlet, useNavigate, useParams, useSearchParams } from "react-router";
import { api, type Ingredient, type Variant } from "../api/client";
import { answer, Refused, refusedRequest } from "../api/query";
import { TimersProvider, useTimers } from "../components/Timers";
import { ingredientsIn, stepPieces } from "../kitchen";
import { servingsText } from "../recipe";
import { useAccount, useSession } from "../session";
import { addWeeks, mondayOf, weekLabel } from "../week";

/** How far a finger moves sideways, in pixels, before it counts as a swipe to the next or previous step. */
const SWIPE = 60;

/** The frame round kitchen mode, for a screen in the kitchen (1280 × 800, or a tablet): no top bar, timers along
 * the top that keep running from step to step, and the screen kept awake while it's open. A kitchen device opens
 * nothing else; anyone signed in can open it too. */
export function KitchenFrame() {
  const { session } = useSession();
  const awake = useWakeLock();
  return (
    <TimersProvider>
      <main className="kitchen">
        {session.status === "offline" && (
          <p className="offline-bar">Offline: showing what this device has saved.</p>
        )}
        <Outlet />
        {awake === "unavailable" && (
          <p className="kitchen-awake">
            This screen may dim while you cook: keeping it awake needs a secure (HTTPS) address.
          </p>
        )}
      </main>
    </TimersProvider>
  );
}

/** Keeps the screen on while kitchen mode is open (the Screen Wake Lock API), asking again whenever the page
 * comes back into view, since the browser lets go of it when the page is hidden. "unavailable" where the browser
 * has no wake lock, as over plain HTTP. */
function useWakeLock(): "held" | "waiting" | "unavailable" {
  const supported = typeof navigator !== "undefined" && "wakeLock" in navigator;
  const [held, setHeld] = useState(false);
  useEffect(() => {
    if (!supported) return;
    let lock: WakeLockSentinel | null = null;
    let gone = false;
    /** Asks for the lock while the page is in view and doesn't hold one. */
    const take = async () => {
      if (gone || document.visibilityState !== "visible" || (lock && !lock.released)) return;
      try {
        const got = await navigator.wakeLock.request("screen");
        if (gone) return void got.release();
        lock = got;
        setHeld(true);
        got.addEventListener("release", () => setHeld(false));
      } catch {
        // Refused (a battery saver, a page not in focus): the screen sleeps as usual.
        setHeld(false);
      }
    };
    take();
    document.addEventListener("visibilitychange", take);
    return () => {
      gone = true;
      document.removeEventListener("visibilitychange", take);
      lock?.release().catch(() => {});
    };
  }, [supported]);
  if (!supported) return "unavailable";
  return held ? "held" : "waiting";
}

/** Kitchen mode's start: the week's planned dishes (?week=, this week by the screen's own calendar by default),
 * each opening its steps at the planned serving count. */
export function KitchenPlan() {
  const { session } = useSession();
  const kitchen = (session.status === "in" || session.status === "offline") && session.me?.kitchen;
  const [params, setParams] = useSearchParams();
  const week = params.get("week") ?? mondayOf();
  const planQuery = useQuery({
    queryKey: ["plan", useAccount(), week],
    queryFn: ({ signal }) =>
      answer(api.GET("/api/v1/plan", { params: { query: { week } }, signal })),
  });
  const plan = planQuery.data;

  return (
    <div className="kitchen-plan">
      <header className="kitchen-head">
        <div className="week-nav">
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
        {!kitchen && (
          <Link to="/plan" className="link">
            Leave kitchen mode
          </Link>
        )}
      </header>
      {planQuery.isError && (
        <p className="empty">
          {planQuery.error instanceof Refused
            ? planQuery.error.message
            : "Can't reach the server, so the plan didn't load."}{" "}
          <button type="button" className="link" onClick={() => planQuery.refetch()}>
            Try again
          </button>
        </p>
      )}
      {!plan && !planQuery.isError && <p className="empty">Loading…</p>}
      {plan && plan.entries.length === 0 && (
        <p className="empty">
          Nothing planned for this week. Dishes planned on a phone or computer show here.
        </p>
      )}
      {plan && plan.entries.length > 0 && (
        <ul className="kitchen-dishes">
          {plan.entries.map((e) => (
            <li key={e.id}>
              <Link to={`/kitchen/${e.variant_id}?s=${e.servings}`}>
                {e.image ? (
                  <img src={e.image.large} alt="" width={1200} height={800} />
                ) : (
                  <span className="noimg" aria-hidden="true" />
                )}
                <strong>{e.title}</strong>
                {e.subtitle && <span className="sub">{e.subtitle}</span>}
                <span className="meta">{servingsText(e.servings)}</span>
              </Link>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** One planned dish to cook, a screen at a time: what goes in it, then each step in large type with the
 * ingredients it names, at the planned serving count (?s=). The screen (?step=, 0 for the ingredients) is in the
 * address, so a reload stays on it. Move with the buttons, a swipe, or the arrow keys; a time in a step starts a
 * timer. */
export function KitchenCook() {
  const { variant: id = "" } = useParams();
  const [params, setParams] = useSearchParams();
  const navigate = useNavigate();
  const servings = Number(params.get("s")) || undefined;
  const variantQuery = useQuery({
    queryKey: ["variant", id, servings],
    queryFn: ({ signal }) =>
      answer(
        api.GET("/api/v1/variants/{id}", {
          params: { path: { id }, query: { servings } },
          signal,
        }),
      ),
  });
  const v = variantQuery.data;
  const last = v?.steps.length ?? 0;
  const screen = Math.min(Math.max(Number(params.get("step")) || 0, 0), last);

  /** Moves to another screen, replacing the history entry so Back leaves kitchen mode's dish. */
  const go = (to: number) => {
    if (!v || to < 0 || to > last) return;
    setParams(
      (prev) => {
        const next = new URLSearchParams(prev);
        next.set("step", String(to));
        return next;
      },
      { replace: true },
    );
  };
  const goRef = useRef(go);
  useEffect(() => {
    goRef.current = go;
  });
  // The arrow keys move too, for a keyboard or a remote.
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLInputElement) return;
      if (e.key === "ArrowRight") goRef.current(screen + 1);
      if (e.key === "ArrowLeft") goRef.current(screen - 1);
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [screen]);

  // Where a touch began, to tell a swipe from a tap.
  const touch = useRef<{ x: number; y: number } | null>(null);
  /** A sideways swipe moves a screen: left for the next, right for the one before. */
  const swiped = (e: PointerEvent) => {
    const from = touch.current;
    touch.current = null;
    if (!from) return;
    const dx = e.clientX - from.x;
    const dy = e.clientY - from.y;
    if (Math.abs(dx) >= SWIPE && Math.abs(dx) > Math.abs(dy)) go(dx < 0 ? screen + 1 : screen - 1);
  };

  if (refusedRequest(variantQuery.error)) {
    return (
      <p className="empty">
        {variantQuery.error.code === "not_found"
          ? "This dish isn't one this kitchen can see."
          : variantQuery.error.message}{" "}
        <Link to="/kitchen">Back to the week</Link>
      </p>
    );
  }
  if (variantQuery.isError)
    return (
      <p className="empty">
        Can't reach the server, so this dish didn't load.{" "}
        <button type="button" className="link" onClick={() => variantQuery.refetch()}>
          Try again
        </button>
      </p>
    );
  if (!v) return <p className="empty">Loading…</p>;

  return (
    <article
      className="kitchen-cook"
      onPointerDown={(e) => (touch.current = { x: e.clientX, y: e.clientY })}
      onPointerUp={swiped}
      onPointerCancel={() => (touch.current = null)}
    >
      <header className="kitchen-bar">
        <Link to="/kitchen" className="kitchen-back">
          ‹ Week
        </Link>
        <h1>{v.title}</h1>
        <span className="meta">{servingsText(v.servings)}</span>
      </header>

      {screen === 0 ? (
        <section className="kitchen-all" aria-labelledby="k-all-h">
          <h2 id="k-all-h">Ingredients</h2>
          <Amounts list={v.ingredients} />
          {v.steps.length === 0 && <p className="empty">This version has no steps.</p>}
        </section>
      ) : (
        <Step v={v} n={screen} />
      )}

      <nav className="kitchen-nav" aria-label="Steps">
        <button type="button" onClick={() => go(screen - 1)} disabled={screen === 0}>
          Previous
        </button>
        <span className="kitchen-where" aria-live="polite">
          {screen === 0 ? "Ingredients" : `Step ${screen} of ${last}`}
        </span>
        {screen < last ? (
          <button type="button" className="primary" onClick={() => go(screen + 1)}>
            {screen === 0 ? "Start cooking" : "Next"}
          </button>
        ) : (
          <button type="button" className="primary" onClick={() => navigate("/kitchen")}>
            Done
          </button>
        )}
      </nav>
    </article>
  );
}

/** Step `n` of a version: its text in large type, each time in it a button that starts a timer, and beside it the
 * ingredients it names with their amounts. */
function Step({ v, n }: { v: Variant; n: number }) {
  const { start, running } = useTimers();
  const step = v.steps[n - 1];
  const used = ingredientsIn(step.text, v.ingredients);
  return (
    <div className="kitchen-step">
      <section className="kitchen-text" aria-labelledby="k-step-h">
        <h2 id="k-step-h">Step {n}</h2>
        {stepPieces(step.text, step.times).map((paragraph, i) => (
          <p key={i}>
            {paragraph.map((piece, j) => {
              if (!("time" in piece)) return <span key={j}>{piece.text}</span>;
              const key = `${v.id}:${n}:${piece.index}`;
              const label = `Step ${n}: ${piece.time.text}`;
              return (
                <button
                  key={j}
                  type="button"
                  className={running(key) ? "time running" : "time"}
                  onClick={() => start(key, label, piece.time)}
                  aria-label={`Start a timer for ${piece.time.text}`}
                >
                  {piece.time.text}
                </button>
              );
            })}
          </p>
        ))}
        {step.check_amounts ? (
          <p className="step-flag">Check amounts: written for another serving count.</p>
        ) : (
          step.scaled && <p className="step-flag">Amounts scaled for this serving count.</p>
        )}
      </section>
      <section className="kitchen-used" aria-labelledby="k-used-h">
        <h2 id="k-used-h">In this step</h2>
        {used.length > 0 ? <Amounts list={used} /> : <p className="meta">Nothing to measure.</p>}
      </section>
    </div>
  );
}

/** Ingredient lines with their amounts, marking those scaled from the published ones. */
function Amounts({ list }: { list: Ingredient[] }) {
  return (
    <ul className="kitchen-amounts">
      {list.map((i, n) => (
        <li key={n}>
          <span className="amount">
            {i.scaled && (
              <span className="approx" title="Scaled from the published amounts">
                ≈
              </span>
            )}
            {i.amount ?? ""}
          </span>
          <span className="name">{i.name}</span>
        </li>
      ))}
    </ul>
  );
}
