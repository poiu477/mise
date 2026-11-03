import { useEffect, useState, type FormEvent } from "react";
import { keepListsOnDevice } from "../pages/ShoppingPage";
import { Link, NavLink, Navigate, Outlet, useLocation, useNavigate, useSearchParams } from "react-router";
import { useSession } from "../session";

const OPEN = ["/login", "/setup"];

export function Layout() {
  const { session } = useSession();
  const location = useLocation();
  const here = location.pathname + location.search;
  const onOpenPage = OPEN.includes(location.pathname);

  if (session.status === "loading") return <p className="empty">Loading…</p>;
  if (session.status === "offline" && !session.me) return <p className="empty">Can't reach the mise server. Check that it's running and reload.</p>;
  if (session.status === "setup" && location.pathname !== "/setup") return <Navigate to="/setup" replace />;
  if (session.status === "out" && location.pathname !== "/login") {
    return <Navigate to={`/login?return_to=${encodeURIComponent(here)}`} replace />;
  }
  if (session.status === "in" && onOpenPage) {
    const back = new URLSearchParams(location.search).get("return_to");
    return <Navigate to={back && back.startsWith("/") && !back.startsWith("//") ? back : "/"} replace />;
  }
  if (onOpenPage) {
    return (
      <main className="content open">
        <Outlet />
      </main>
    );
  }
  return (
    <div className="app">
      <TopBar />
      {session.status === "offline" && <p className="offline-bar">Offline: showing what this device has saved.</p>}
      <main className="content">
        <Outlet />
      </main>
    </div>
  );
}

function TopBar() {
  const { session } = useSession();
  const me = session.status === "in" || session.status === "offline" ? session.me : null;
  // This week's and next week's lists, kept on the device for the shop.
  const signedIn = session.status === "in" ? session.me.id : null;
  useEffect(() => {
    if (signedIn) keepListsOnDevice(signedIn);
  }, [signedIn]);
  const [params] = useSearchParams();
  const location = useLocation();
  const navigate = useNavigate();
  const [q, setQ] = useState(params.get("q") ?? "");

  // Keep the box in step with the URL (back/forward, links).
  useEffect(() => {
    if (location.pathname === "/") setQ(params.get("q") ?? "");
  }, [location.pathname, params]);

  const submit = (e: FormEvent) => {
    e.preventDefault();
    const next = new URLSearchParams();
    if (q.trim()) next.set("q", q.trim());
    navigate({ pathname: "/", search: next.toString() });
  };

  return (
    <header className="topbar">
      <Link to="/" className="brand" aria-label="mise home">
        <img src="/icon.svg" alt="" width={28} height={28} />
        <span>mise</span>
      </Link>
      <form className="search" role="search" onSubmit={submit}>
        <input type="search" value={q} onChange={(e) => setQ(e.target.value)} placeholder="Search dishes" aria-label="Search dishes" enterKeyHint="search" />
      </form>
      {me && (
        <nav className="mainnav" aria-label="Main">
          <NavLink to="/plan">Plan</NavLink>
          <NavLink to="/shopping">Shopping</NavLink>
        </nav>
      )}
      {me && (
        <Link to="/account" className="account-link" title="Account">
          <span className="avatar" aria-hidden="true">
            {me.display_name.slice(0, 1).toUpperCase()}
          </span>
          <span className="account-name">{me.display_name}</span>
        </Link>
      )}
    </header>
  );
}
