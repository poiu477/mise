// Who is signed in, for the whole app. Loaded once; refreshed after signing in or out.
import { useQuery } from "@tanstack/react-query";
import { createContext, useCallback, useContext, useEffect, useRef, type ReactNode } from "react";
import {
  actAs,
  api,
  whenAccountChanged,
  whenUnauthorized,
  type AuthInfo,
  type Me,
} from "./api/client";
import { queryClient } from "./api/query";
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

const SESSION = ["session"];
const LOADING: Session = { status: "loading" };

export function SessionProvider({ children }: { children: ReactNode }) {
  // Only the latest check counts: an older one finishing late must not put back an account that has changed since.
  const latest = useRef(0);
  const check = useCallback(async (): Promise<Session> => {
    const mine = ++latest.current;
    // A weak signal can hang rather than fail: give up after a few seconds and open from the device.
    const cfg = await api
      .GET("/api/auth/config", { signal: AbortSignal.timeout(6000) })
      .catch(() => null);
    // A newer check replaced this one, and its answer is the one kept.
    if (mine !== latest.current) return LOADING;
    if (!cfg?.data) {
      const me = rememberedMe();
      actAs(me?.id ?? null);
      return { status: "offline", me };
    }
    const info = cfg.data;
    if (info.setup_required) {
      actAs(null);
      return { status: "setup", info };
    }
    const me = await api.GET("/api/me").catch(() => null);
    if (mine !== latest.current) return LOADING;
    if (me?.data) rememberMe(me.data);
    actAs(me?.data?.id ?? null);
    return me?.data ? { status: "in", info, me: me.data } : { status: "out", info };
  }, []);

  // Checked once when the app opens, and again only when asked (signing in or out, another account, back online).
  const { data: session = LOADING } = useQuery({
    queryKey: SESSION,
    queryFn: check,
    staleTime: Infinity,
    gcTime: Infinity,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
  });

  // A new check cancels one still on its way, so only the newest lands.
  const refresh = useCallback(() => queryClient.refetchQueries({ queryKey: SESSION }), []);

  const signOut = useCallback(async () => {
    await api.POST("/api/auth/logout").catch(() => null);
    forgetDevice();
    actAs(null);
    await refresh();
    // Nothing of the household stays in memory either.
    queryClient.removeQueries({ predicate: (q) => q.queryKey[0] !== SESSION[0] });
  }, [refresh]);

  useEffect(() => {
    whenUnauthorized(() =>
      queryClient.setQueryData<Session>(SESSION, (s) =>
        s?.status === "in" ? { status: "out", info: s.info } : s,
      ),
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

/** The account the app works as: signed in, or offline as whoever last was. Keys the data that is the account's
 * own, so another account's never shows. */
export function useAccount(): string | null {
  const { session } = useSession();
  return session.status === "in" || session.status === "offline" ? (session.me?.id ?? null) : null;
}
