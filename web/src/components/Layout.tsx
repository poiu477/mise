import { useEffect, useState, type FormEvent } from "react";
import { Link, Outlet, useLocation, useNavigate, useSearchParams } from "react-router";

export function Layout() {
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
    <div className="app">
      <header className="topbar">
        <Link to="/" className="brand" aria-label="mise home">
          <img src="/icon.svg" alt="" width={28} height={28} />
          <span>mise</span>
        </Link>
        <form className="search" role="search" onSubmit={submit}>
          <input
            type="search"
            value={q}
            onChange={(e) => setQ(e.target.value)}
            placeholder="Search dishes"
            aria-label="Search dishes"
            enterKeyHint="search"
          />
        </form>
      </header>
      <main className="content">
        <Outlet />
      </main>
    </div>
  );
}
