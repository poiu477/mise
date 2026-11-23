import { useState, type FormEvent } from "react";
import { Link, useNavigate } from "react-router";
import { api, errorText, type ImportedRecipe } from "../api/client";

/** Where an imported recipe opens: its dish, on the version the import made. */
const opened = (r: ImportedRecipe) => `/dishes/${r.dish_slug}?v=${r.variant_id}`;

/** "Import from a link": paste a recipe page's address and the server reads it into the household's own recipes.
 * One recipe imported cleanly opens at once; several, or one whose photo couldn't be kept, are listed first. */
export function ImportPage() {
  const navigate = useNavigate();
  const [url, setUrl] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [imported, setImported] = useState<ImportedRecipe[] | null>(null);

  /** Sends the link; opens the recipe, or shows what came of it. */
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError("");
    setImported(null);
    const { data, error: failed } = await api.POST("/api/v1/imports", { body: { url } });
    setBusy(false);
    if (!data) {
      setError(errorText(failed, "Couldn't reach the server, so nothing was imported."));
      return;
    }
    const [only] = data.recipes;
    if (data.recipes.length === 1 && !only.photo_problem) {
      navigate(opened(only));
      return;
    }
    setImported(data.recipes);
  };

  return (
    <section className="import-page">
      <h1>Import from a link</h1>
      <p className="meta">
        Paste the address of a recipe page. mise reads the recipe from it into your household's
        recipes, which nobody else sees.
      </p>
      <form className="form" onSubmit={submit}>
        <label>
          Link to a recipe page
          <input
            type="url"
            inputMode="url"
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            placeholder="https://"
            required
            autoFocus
          />
        </label>
        <button type="submit" className="primary" disabled={busy}>
          {busy ? "Importing…" : "Import"}
        </button>
      </form>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      {imported && (
        <ul className="imported" aria-label="Imported recipes">
          {imported.map((r) => (
            <li key={r.variant_id}>
              <Link to={opened(r)}>{r.title}</Link>
              {r.unchanged && <span className="meta"> (already imported, unchanged)</span>}
              {r.photo_problem && <p className="meta">{r.photo_problem}</p>}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
