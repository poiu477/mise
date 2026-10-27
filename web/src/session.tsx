// Who is signed in, for the whole app. Loaded once; refreshed after signing in or out.
import { createContext, useCallback, useContext, useEffect, useState, type ReactNode } from "react";
import { api, whenUnauthorized, type AuthInfo, type Me } from "./api/client";

export type Session =
  | { status: "loading" }
  | { status: "offline" }
  | { status: "setup"; info: AuthInfo }
  | { status: "out"; info: AuthInfo }
  | { status: "in"; info: AuthInfo; me: Me };

type Ctx = { session: Session; refresh: () => Promise<void>; signOut: () => Promise<void> };
const SessionContext = createContext<Ctx | null>(null);

export function SessionProvider({ children }: { children: ReactNode }) {
  const [session, setSession] = useState<Session>({ status: "loading" });

  const refresh = useCallback(async () => {
    const cfg = await api.GET("/api/auth/config").catch(() => null);
    if (!cfg?.data) return setSession({ status: "offline" });
    const info = cfg.data;
    if (info.setup_required) return setSession({ status: "setup", info });
    const me = await api.GET("/api/me").catch(() => null);
    setSession(me?.data ? { status: "in", info, me: me.data } : { status: "out", info });
  }, []);

  const signOut = useCallback(async () => {
    await api.POST("/api/auth/logout").catch(() => null);
    await refresh();
  }, [refresh]);

  useEffect(() => {
    refresh();
    whenUnauthorized(() => setSession((s) => (s.status === "in" ? { status: "out", info: s.info } : s)));
  }, [refresh]);

  return <SessionContext.Provider value={{ session, refresh, signOut }}>{children}</SessionContext.Provider>;
}

export function useSession(): Ctx {
  const ctx = useContext(SessionContext);
  if (!ctx) throw new Error("useSession outside SessionProvider");
  return ctx;
}
