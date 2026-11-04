import { useState, type FormEvent } from "react";
import { api, errorText } from "../api/client";
import { useSession } from "../session";

/** First run: the first person to open mise creates the admin account and the first household. */
export function SetupPage() {
  const { refresh } = useSession();
  const [form, setForm] = useState({
    display_name: "",
    username: "",
    password: "",
    confirm: "",
    household: "Home",
  });
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const set = (k: keyof typeof form) => (e: React.ChangeEvent<HTMLInputElement>) =>
    setForm({ ...form, [k]: e.target.value });

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (form.password !== form.confirm) return setError("The passwords don't match.");
    setBusy(true);
    setError("");
    const { error } = await api.POST("/api/auth/setup", {
      body: {
        username: form.username,
        password: form.password,
        display_name: form.display_name || undefined,
        household: form.household || undefined,
      },
    });
    setBusy(false);
    if (error) return setError(errorText(error));
    await refresh();
  };

  return (
    <section className="auth-card">
      <img src="/icon.svg" alt="" width={48} height={48} />
      <h1>Welcome to mise</h1>
      <p className="sub">
        Create the admin account. You can add the rest of the household afterwards.
      </p>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      <form className="form" onSubmit={submit}>
        <label>
          Your name
          <input
            value={form.display_name}
            onChange={set("display_name")}
            autoComplete="name"
            autoFocus
          />
        </label>
        <label>
          Username
          <input
            value={form.username}
            onChange={set("username")}
            autoComplete="username"
            required
          />
        </label>
        <label>
          Password <span className="hint">at least 8 characters</span>
          <input
            type="password"
            value={form.password}
            onChange={set("password")}
            autoComplete="new-password"
            minLength={8}
            required
          />
        </label>
        <label>
          Password again
          <input
            type="password"
            value={form.confirm}
            onChange={set("confirm")}
            autoComplete="new-password"
            required
          />
        </label>
        <label>
          Household name
          <input value={form.household} onChange={set("household")} />
        </label>
        <button type="submit" className="primary" disabled={busy}>
          {busy ? "Creating…" : "Create account"}
        </button>
      </form>
    </section>
  );
}
