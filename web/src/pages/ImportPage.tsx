import { useState, type FormEvent } from "react";
import { Link, useNavigate } from "react-router";
import { api, errorText, type FileSkipped, type ImportedRecipe } from "../api/client";
import { fileSummary, openAtOnce, opened } from "../imports";

/** What an import made, as the page lists it. */
type Outcome = { summary: string | null; recipes: ImportedRecipe[]; skipped: FileSkipped[] };

/** Files the picker offers: what other recipe apps export, and schema.org JSON. Others can still be picked. */
const EXPORTS = ".json,.zip,.paprikarecipes,.paprikarecipe,application/json,application/zip";

/** "Import recipes": from a recipe page's address, which the server fetches, or from a file another recipe app
 * exported. One recipe imported cleanly opens at once; several, or anything that needs saying, are listed first. */
export function ImportPage() {
  const navigate = useNavigate();
  const [url, setUrl] = useState("");
  const [file, setFile] = useState<File | null>(null);
  const [busy, setBusy] = useState<"link" | "file" | null>(null);
  const [error, setError] = useState("");
  const [outcome, setOutcome] = useState<Outcome | null>(null);

  const start = (what: "link" | "file") => {
    setBusy(what);
    setError("");
    setOutcome(null);
  };

  /** Sends the link; opens the recipe, or shows what came of it. */
  const importLink = async (e: FormEvent) => {
    e.preventDefault();
    start("link");
    const { data, error: failed } = await api
      .POST("/api/v1/imports", { body: { url } })
      .catch(() => ({ data: undefined, error: undefined }));
    setBusy(null);
    if (!data) {
      setError(errorText(failed, "Couldn't reach the server, so nothing was imported."));
      return;
    }
    const at = openAtOnce(data.recipes);
    if (at) navigate(at);
    else setOutcome({ summary: null, recipes: data.recipes, skipped: [] });
  };

  /** Sends the file whole; opens the recipe, or shows what came of it. */
  const importFile = async (e: FormEvent) => {
    e.preventDefault();
    if (!file) return;
    start("file");
    const { data, error: failed } = await api
      .POST("/api/v1/imports/file", {
        params: { query: { name: file.name } },
        // The file itself is the body; the generated types call it text.
        body: file as unknown as string,
        bodySerializer: (sent) => sent as unknown as Blob,
        headers: { "Content-Type": "application/octet-stream" },
      })
      .catch(() => ({ data: undefined, error: undefined }));
    setBusy(null);
    if (!data) {
      setError(errorText(failed, "Couldn't reach the server, so nothing was imported."));
      return;
    }
    const at = openAtOnce(data.recipes, data.skipped.length);
    if (at) navigate(at);
    else
      setOutcome({
        summary: fileSummary(data.formats, data.recipes),
        recipes: data.recipes,
        skipped: data.skipped,
      });
  };

  return (
    <section className="import-page">
      <h1>Import recipes</h1>
      <p className="meta">
        Bring recipes in from a web page or from another recipe app. They become your household's
        own, which nobody else sees.
      </p>

      <h2>From a link</h2>
      <form className="form" onSubmit={importLink}>
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
        <button type="submit" className="primary" disabled={busy !== null}>
          {busy === "link" ? "Importing…" : "Import"}
        </button>
      </form>

      <h2>From a file</h2>
      <p className="meta">
        An export from Paprika, Mealie, Tandoor or Nextcloud Cookbook (the Recipes folder, zipped),
        or a recipe saved as schema.org JSON. Photos come too when the file holds them.
      </p>
      <form className="form" onSubmit={importFile}>
        <label>
          Export file
          <input
            type="file"
            accept={EXPORTS}
            onChange={(e) => setFile(e.target.files?.[0] ?? null)}
            required
          />
        </label>
        <button type="submit" className="primary" disabled={busy !== null || !file}>
          {busy === "file" ? "Importing…" : "Import file"}
        </button>
      </form>

      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      {outcome && (
        <div className="import-outcome" role="status">
          {outcome.summary && <p>{outcome.summary}</p>}
          <ul className="imported" aria-label="Imported recipes">
            {outcome.recipes.map((r) => (
              <li key={r.variant_id}>
                <Link to={opened(r)}>{r.title}</Link>
                {r.unchanged && <span className="meta"> (already imported, unchanged)</span>}
                {r.photo_problem && <p className="meta">{r.photo_problem}</p>}
              </li>
            ))}
          </ul>
          {outcome.skipped.length > 0 && (
            <>
              <p>Left out, as they couldn't be read:</p>
              <ul className="imported" aria-label="Left out">
                {outcome.skipped.map((s) => (
                  <li key={s.path}>
                    <code>{s.path}</code>: {s.reason}
                  </li>
                ))}
              </ul>
            </>
          )}
        </div>
      )}
    </section>
  );
}
