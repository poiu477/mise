import { useState, type FormEvent } from "react";
import { useSearchParams } from "react-router";
import { api, errorText } from "../api/client";
import { startSignIn } from "../offline";
import { useSession } from "../session";

/** Sign in with a username and password, single sign-on, or both, as the server allows; then back to
 * `return_to`. Shows an error the single-sign-on round trip sent back. */
export function LoginPage() {
  const { session, refresh } = useSession();
  const [params] = useSearchParams();
  const returnTo = params.get("return_to") ?? "/";
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState(params.get("error") ?? "");
  const [busy, setBusy] = useState(false);
  const info = session.status === "out" ? session.info : null;

  /** Signs in, then starts a new sign-in on this device so nothing from before it can write here. */
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError("");
    const { error, response } = await api.POST("/api/auth/login", { body: { username, password } });
    setBusy(false);
    if (error) {
      const wait = response.headers.get("retry-after");
      setError(
        response.status === 429 && wait
          ? `Too many attempts. Try again in ${wait} seconds.`
          : errorText(error),
      );
      return;
    }
    startSignIn();
    await refresh();
  };

  return (
    <section className="auth-card">
      <img src="/icon.svg" alt="" width={48} height={48} />
      <h1>Sign in to mise</h1>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      {info?.local !== false && (
        <form className="form" onSubmit={submit}>
          <label>
            Username
            <input
              autoComplete="username"
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              required
              autoFocus
            />
          </label>
          <label>
            Password
            <input
              type="password"
              autoComplete="current-password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              required
            />
          </label>
          <button type="submit" className="primary" disabled={busy}>
            {busy ? "Signing in…" : "Sign in"}
          </button>
        </form>
      )}
      {info?.oidc && (
        <>
          {info.local && <p className="or">or</p>}
          <a
            className="button sso"
            href={`/api/auth/oidc/start?return_to=${encodeURIComponent(returnTo)}`}
          >
            Sign in with {info.oidc}
          </a>
        </>
      )}
    </section>
  );
}
