import { useQuery } from "@tanstack/react-query";
import { useState, type FormEvent } from "react";
import {
  api,
  errorText,
  type Me,
  type TokenInfo,
  type UserInfo,
  type Weights,
} from "../api/client";
import { answer, queryClient } from "../api/query";
import { useAccount, useSession } from "../session";
import { WEIGHTS } from "../weights";

/** The account page: who is signed in, the household's list units, password, API tokens and, for admins,
 * people and households. Needs the server. */
export function AccountPage() {
  const { session, signOut } = useSession();
  if (session.status === "offline")
    return (
      <p className="empty">
        Account settings need the server. They'll be here when it's back in reach.
      </p>
    );
  if (session.status !== "in") return null;
  const me = session.me;
  return (
    <article className="account">
      <header className="account-head">
        <div>
          <h1>{me.display_name}</h1>
          <p className="sub">
            {me.username} · {me.household.name}
            {me.is_admin && <span className="tag">admin</span>}
          </p>
        </div>
        <button type="button" onClick={signOut}>
          Sign out
        </button>
      </header>
      <ShoppingUnits />
      <PasswordForm me={me} />
      <Tokens />
      {me.is_admin && <Admin me={me} />}
    </article>
  );
}

/** How weights read on the household's shopping list; each food can still read its own way. */
function ShoppingUnits() {
  const key = ["shopping-settings", useAccount()];
  const settings = useQuery({
    queryKey: key,
    queryFn: ({ signal }) => answer(api.GET("/api/shopping/settings", { signal })),
  });
  const weights = settings.data?.weight_unit ?? null;
  const [error, setError] = useState("");
  /** Saves a new unit for the whole household. */
  const change = async (next: Weights) => {
    // Shown at once; the server's answer, or what it still has, replaces it. A reload still on its way would put
    // back the old unit.
    await queryClient.cancelQueries({ queryKey: key });
    queryClient.setQueryData(key, { weight_unit: next });
    setError("");
    const { data, error: failed } = await api
      .PUT("/api/shopping/settings", { body: { weight_unit: next } })
      .catch(() => ({
        data: undefined,
        error: { error: "can't reach the server, so that wasn't saved" },
      }));
    if (data) queryClient.setQueryData(key, data);
    else {
      setError(errorText(failed));
      queryClient.invalidateQueries({ queryKey: key });
    }
  };
  return (
    <section className="panel">
      <h2>Shopping list</h2>
      <p className="sub">
        How weights read on your household's list. To read one food differently, open it on the list
        (⋯).
      </p>
      <div className="form row">
        <label>
          Weights
          <select
            value={weights ?? ""}
            disabled={!weights}
            onChange={(e) => change(e.target.value as Weights)}
          >
            {WEIGHTS.map((w) => (
              <option key={w.value} value={w.value}>
                {w.label}
              </option>
            ))}
          </select>
        </label>
      </div>
      {error && <p className="form-error">{error}</p>}
    </section>
  );
}

/** Change the password, or set one for someone who signs in with single sign-on. */
function PasswordForm({ me }: { me: Me }) {
  const [form, setForm] = useState({ current: "", next: "", confirm: "" });
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);
  /** Checks the two new passwords match, then saves; other browsers are signed out. */
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (form.next !== form.confirm)
      return setMsg({ ok: false, text: "The new passwords don't match." });
    const { error } = await api.POST("/api/me/password", {
      body: { current: me.has_password ? form.current : undefined, new: form.next },
    });
    if (error) return setMsg({ ok: false, text: errorText(error) });
    setForm({ current: "", next: "", confirm: "" });
    setMsg({ ok: true, text: "Password changed. Other browsers were signed out." });
  };
  return (
    <section className="panel">
      <h2>{me.has_password ? "Change password" : "Set a password"}</h2>
      {!me.has_password && (
        <p className="sub">
          You sign in with single sign-on. A password lets you sign in without it too.
        </p>
      )}
      <form className="form row" onSubmit={submit}>
        {me.has_password && (
          <label>
            Current password
            <input
              type="password"
              autoComplete="current-password"
              value={form.current}
              onChange={(e) => setForm({ ...form, current: e.target.value })}
              required
            />
          </label>
        )}
        <label>
          New password
          <input
            type="password"
            autoComplete="new-password"
            minLength={8}
            value={form.next}
            onChange={(e) => setForm({ ...form, next: e.target.value })}
            required
          />
        </label>
        <label>
          Again
          <input
            type="password"
            autoComplete="new-password"
            value={form.confirm}
            onChange={(e) => setForm({ ...form, confirm: e.target.value })}
            required
          />
        </label>
        <button type="submit">Save</button>
      </form>
      {msg && <p className={msg.ok ? "form-ok" : "form-error"}>{msg.text}</p>}
    </section>
  );
}

/** API tokens for scripts and integrations: list, create (shown once) and revoke. */
function Tokens() {
  const key = ["tokens", useAccount()];
  const tokens =
    useQuery({
      queryKey: key,
      queryFn: ({ signal }) => answer(api.GET("/api/me/tokens", { signal })),
    }).data ?? [];
  const [name, setName] = useState("");
  const [scope, setScope] = useState("read");
  const [fresh, setFresh] = useState<string | null>(null);
  const [error, setError] = useState("");
  /** Shows the tokens afresh after a change. */
  const load = () => queryClient.invalidateQueries({ queryKey: key });
  /** Makes a token and shows it once, to copy now. */
  const create = async (e: FormEvent) => {
    e.preventDefault();
    const { data, error } = await api.POST("/api/me/tokens", { body: { name, scope } });
    if (error || !data) return setError(errorText(error));
    setFresh(data.token);
    setName("");
    setError("");
    load();
  };
  /** Revokes a token after asking, since anything using it stops working. */
  const revoke = async (t: TokenInfo) => {
    if (!window.confirm(`Revoke "${t.name}"? Anything using it stops working.`)) return;
    await api.DELETE("/api/me/tokens/{id}", { params: { path: { id: t.id } } });
    load();
  };
  return (
    <section className="panel">
      <h2>API tokens</h2>
      <p className="sub">
        For scripts and integrations: send as <code>Authorization: Bearer …</code>. Read tokens
        can't change anything.
      </p>
      {fresh && (
        <div className="fresh-token">
          <p>Copy this now; it won't be shown again.</p>
          <code>{fresh}</code>
          <button type="button" onClick={() => navigator.clipboard?.writeText(fresh)}>
            Copy
          </button>
        </div>
      )}
      {tokens.length > 0 && (
        <table className="table">
          <thead>
            <tr>
              <th>Name</th>
              <th>Access</th>
              <th>Created</th>
              <th>Last used</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {tokens.map((t) => (
              <tr key={t.id}>
                <td>{t.name}</td>
                <td>{t.scope === "read" ? "Read" : "Read and write"}</td>
                <td>{day(t.created_at)}</td>
                <td>{t.last_used_at ? day(t.last_used_at) : "Never"}</td>
                <td>
                  <button type="button" className="link danger" onClick={() => revoke(t)}>
                    Revoke
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <form className="form row" onSubmit={create}>
        <label>
          Name
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="Grocy sync"
            required
          />
        </label>
        <label>
          Access
          <select value={scope} onChange={(e) => setScope(e.target.value)}>
            <option value="read">Read</option>
            <option value="read_write">Read and write</option>
          </select>
        </label>
        <button type="submit">Create token</button>
      </form>
      {error && <p className="form-error">{error}</p>}
    </section>
  );
}

/** For admins: everyone's accounts and households, adding people and households. */
function Admin({ me }: { me: Me }) {
  const users =
    useQuery({
      queryKey: ["users", me.id],
      queryFn: ({ signal }) => answer(api.GET("/api/users", { signal })),
    }).data ?? [];
  const households =
    useQuery({
      queryKey: ["households", me.id],
      queryFn: ({ signal }) => answer(api.GET("/api/households", { signal })),
    }).data ?? [];
  const [error, setError] = useState("");
  const [form, setForm] = useState({
    username: "",
    display_name: "",
    email: "",
    password: "",
    household: me.household.id,
    is_admin: false,
  });
  const [newHousehold, setNewHousehold] = useState("");
  /** Shows people and households afresh after a change. */
  const load = () => {
    queryClient.invalidateQueries({ queryKey: ["users", me.id] });
    queryClient.invalidateQueries({ queryKey: ["households", me.id] });
  };

  /** Changes one person (admin, signing in, household), then shows everyone afresh. */
  const update = async (
    u: UserInfo,
    change: { is_admin?: boolean; disabled?: boolean; household_id?: string },
  ) => {
    const { error } = await api.PATCH("/api/users/{id}", {
      params: { path: { id: u.id } },
      body: change,
    });
    setError(error ? errorText(error) : "");
    load();
  };
  /** Adds someone, in an existing household or a new one of their own. */
  const add = async (e: FormEvent) => {
    e.preventDefault();
    const { error } = await api.POST("/api/users", {
      body: {
        username: form.username,
        display_name: form.display_name || undefined,
        email: form.email || undefined,
        password: form.password || undefined,
        is_admin: form.is_admin,
        household_id: form.household === "new" ? undefined : form.household,
      },
    });
    if (error) return setError(errorText(error));
    setError("");
    setForm({ ...form, username: "", display_name: "", email: "", password: "", is_admin: false });
    load();
  };
  /** Adds an empty household to put people in. */
  const addHousehold = async (e: FormEvent) => {
    e.preventDefault();
    const { error } = await api.POST("/api/households", { body: { name: newHousehold } });
    if (error) return setError(errorText(error));
    setNewHousehold("");
    load();
  };

  return (
    <section className="panel">
      <h2>People</h2>
      {error && <p className="form-error">{error}</p>}
      <table className="table">
        <thead>
          <tr>
            <th>Name</th>
            <th>Household</th>
            <th>Signs in with</th>
            <th>Admin</th>
            <th>On</th>
          </tr>
        </thead>
        <tbody>
          {users.map((u) => (
            <tr key={u.id} className={u.disabled ? "off" : ""}>
              <td>
                {u.display_name}
                <span className="meta"> {u.username}</span>
              </td>
              <td>
                <select
                  value={u.household.id}
                  onChange={(e) => update(u, { household_id: e.target.value })}
                  aria-label={`Household of ${u.username}`}
                >
                  {households.map((h) => (
                    <option key={h.id} value={h.id}>
                      {h.name}
                    </option>
                  ))}
                </select>
              </td>
              <td>
                {[u.has_password && "password", u.identities > 0 && "single sign-on"]
                  .filter(Boolean)
                  .join(", ") || "nothing yet"}
              </td>
              <td>
                <input
                  type="checkbox"
                  checked={u.is_admin}
                  onChange={(e) => update(u, { is_admin: e.target.checked })}
                  aria-label={`${u.username} is admin`}
                />
              </td>
              <td>
                <input
                  type="checkbox"
                  checked={!u.disabled}
                  disabled={u.id === me.id}
                  onChange={(e) => update(u, { disabled: !e.target.checked })}
                  aria-label={`${u.username} can sign in`}
                />
              </td>
            </tr>
          ))}
        </tbody>
      </table>

      <h3>Add someone</h3>
      <form className="form row" onSubmit={add}>
        <label>
          Username
          <input
            value={form.username}
            onChange={(e) => setForm({ ...form, username: e.target.value })}
            required
          />
        </label>
        <label>
          Name
          <input
            value={form.display_name}
            onChange={(e) => setForm({ ...form, display_name: e.target.value })}
          />
        </label>
        <label>
          Email <span className="hint">links single sign-on</span>
          <input
            type="email"
            value={form.email}
            onChange={(e) => setForm({ ...form, email: e.target.value })}
          />
        </label>
        <label>
          Password <span className="hint">optional</span>
          <input
            type="password"
            autoComplete="new-password"
            value={form.password}
            onChange={(e) => setForm({ ...form, password: e.target.value })}
          />
        </label>
        <label>
          Household
          <select
            value={form.household}
            onChange={(e) => setForm({ ...form, household: e.target.value })}
          >
            {households.map((h) => (
              <option key={h.id} value={h.id}>
                {h.name}
              </option>
            ))}
            <option value="new">A new household of their own</option>
          </select>
        </label>
        <label className="check">
          <input
            type="checkbox"
            checked={form.is_admin}
            onChange={(e) => setForm({ ...form, is_admin: e.target.checked })}
          />{" "}
          Admin
        </label>
        <button type="submit">Add</button>
      </form>

      <h3>Households</h3>
      <p className="sub">{households.map((h) => h.name).join(" · ")}</p>
      <form className="form row" onSubmit={addHousehold}>
        <label>
          New household
          <input value={newHousehold} onChange={(e) => setNewHousehold(e.target.value)} required />
        </label>
        <button type="submit">Add household</button>
      </form>
    </section>
  );
}

/** A timestamp as the viewer's local date. */
const day = (iso: string) => new Date(iso).toLocaleDateString();
