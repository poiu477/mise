// Who is signed in, for the whole app. Loaded once; refreshed after signing in or out.
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  actAs,
  api,
  whenAccountChanged,
  whenUnauthorized,
  type AuthInfo,
  type Me,
} from "./api/client";
import { forgetDevice, rememberedMe, rememberMe } from "./offline";

export type Session =
  | { status: "loading" }
  /** The server can't be reached; `me` is who last signed in on this device, if anyone. */
  | { status: "offline"; me: Me | null }
  | { status: "setup"; info: AuthInfo }
  | { status: "out"; info: AuthInfo }
  | { status: "in"; info: AuthInfo; me: Me };

type Ctx = { session: Session; refresh: () => Promise<void>; signOut: () => Promise<void> };
const SessionContext = createContext<Ctx | null>(null);

export function SessionProvider({ children }: { children: ReactNode }) {
  const [session, setSession] = useState<Session>({ status: "loading" });

  // Only the latest check counts: an older one finishing late must not put back an account that has changed since.
  const latest = useRef(0);
  const refresh = useCallback(async () => {
    const mine = ++latest.current;
    // A weak signal can hang rather than fail: give up after a few seconds and open from the device.
    const cfg = await api
      .GET("/api/auth/config", { signal: AbortSignal.timeout(6000) })
      .catch(() => null);
    if (mine !== latest.current) return;
    if (!cfg?.data) {
      const me = rememberedMe();
      actAs(me?.id ?? null);
      return setSession({ status: "offline", me });
    }
    const info = cfg.data;
    if (info.setup_required) {
      actAs(null);
      return setSession({ status: "setup", info });
    }
    const me = await api.GET("/api/me").catch(() => null);
    if (mine !== latest.current) return;
    if (me?.data) rememberMe(me.data);
    actAs(me?.data?.id ?? null);
    setSession(me?.data ? { status: "in", info, me: me.data } : { status: "out", info });
  }, []);

  const signOut = useCallback(async () => {
    await api.POST("/api/auth/logout").catch(() => null);
    forgetDevice();
    actAs(null);
    await refresh();
  }, [refresh]);

  useEffect(() => {
    refresh();
    whenUnauthorized(() =>
      setSession((s) => (s.status === "in" ? { status: "out", info: s.info } : s)),
    );
    // Someone else signed in, in another tab: find out who, and the app follows.
    whenAccountChanged(() => refresh());
  }, [refresh]);

  // Back in reach: check again.
  useEffect(() => {
    if (session.status !== "offline") return;
    const again = () => document.visibilityState === "visible" && refresh();
    window.addEventListener("online", again);
    document.addEventListener("visibilitychange", again);
    return () => {
      window.removeEventListener("online", again);
      document.removeEventListener("visibilitychange", again);
    };
  }, [session.status, refresh]);

  return (
    <SessionContext.Provider value={{ session, refresh, signOut }}>
      {children}
    </SessionContext.Provider>
  );
}

export function useSession(): Ctx {
  const ctx = useContext(SessionContext);
  if (!ctx) throw new Error("useSession outside SessionProvider");
  return ctx;
}
