import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { clock, timeUp, type StepTime } from "../kitchen";

/** A running timer: what started it, and when it rings (milliseconds since 1970). */
type Timer = {
  /** The dish, step and time it came from, so the same time isn't started twice. */
  key: string;
  /** "Step 2: 5-6 minutes". */
  label: string;
  time: StepTime;
  ends: number;
};

/** What the kitchen pages can do with timers: start one from a step's time, and tell whether one is running. */
type Ctx = {
  start: (key: string, label: string, time: StepTime) => void;
  running: (key: string) => boolean;
};
const TimersContext = createContext<Ctx | null>(null);

/** Seconds between beeps while a timer rings. */
const RING_EVERY = 2000;

/** Kitchen timers for everything inside it: they keep running as the cook moves between steps and dishes, and
 * show along the top of the screen, counting down. One that runs out rings with a beep in the page until it's
 * stopped, and says what to do: a range's timer runs to its lower end, so it says to check and how long is left. */
export function TimersProvider({ children }: { children: ReactNode }) {
  const [timers, setTimers] = useState<Timer[]>([]);
  const [now, setNow] = useState(() => Date.now());
  // Browsers allow sound only after a tap: the tap that starts a timer readies it.
  const sound = useRef<AudioContext | null>(null);

  /** Starts a timer for a step's time, unless that one is running already. */
  const start = useCallback((key: string, label: string, time: StepTime) => {
    if (typeof AudioContext !== "undefined") {
      sound.current ??= new AudioContext();
      sound.current.resume().catch(() => {});
    }
    const at = Date.now();
    setNow(at);
    setTimers((ts) =>
      ts.some((t) => t.key === key)
        ? ts
        : [...ts, { key, label, time, ends: at + time.seconds * 1000 }],
    );
  }, []);
  /** Stops a timer, running or ringing. */
  const stop = (key: string) => setTimers((ts) => ts.filter((t) => t.key !== key));
  const running = useCallback((key: string) => timers.some((t) => t.key === key), [timers]);

  // The clock ticks while any timer is set.
  const counting = timers.length > 0;
  useEffect(() => {
    if (!counting) return;
    const tick = setInterval(() => setNow(Date.now()), 250);
    return () => clearInterval(tick);
  }, [counting]);

  // Beep while any timer rings.
  const ringing = timers.some((t) => t.ends <= now);
  useEffect(() => {
    if (!ringing) return;
    const beep = () => {
      const ctx = sound.current;
      if (!ctx) return;
      for (const at of [0, 0.3, 0.6]) {
        const tone = ctx.createOscillator();
        const gain = ctx.createGain();
        tone.frequency.value = 880;
        gain.gain.value = 0.2;
        tone.connect(gain).connect(ctx.destination);
        tone.start(ctx.currentTime + at);
        tone.stop(ctx.currentTime + at + 0.18);
      }
    };
    beep();
    const again = setInterval(beep, RING_EVERY);
    return () => clearInterval(again);
  }, [ringing]);

  return (
    <TimersContext.Provider value={{ start, running }}>
      {timers.length > 0 && (
        <section className="timers" aria-label="Timers">
          <ul>
            {timers.map((t) => {
              const done = t.ends <= now;
              return (
                <li key={t.key} className={done ? "timer done" : "timer"}>
                  <span className="timer-label">{t.label}</span>
                  {done ? (
                    <p className="timer-up" role="alert">
                      {timeUp(t.time)}
                    </p>
                  ) : (
                    <span className="timer-left" role="timer">
                      {clock(t.ends - now)}
                    </span>
                  )}
                  <button type="button" onClick={() => stop(t.key)} aria-label={`Stop ${t.label}`}>
                    Stop
                  </button>
                </li>
              );
            })}
          </ul>
        </section>
      )}
      {children}
    </TimersContext.Provider>
  );
}

/** The kitchen's timers; only inside `TimersProvider`. */
export function useTimers(): Ctx {
  const ctx = useContext(TimersContext);
  if (!ctx) throw new Error("useTimers outside TimersProvider");
  return ctx;
}
