import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useNavigate } from "react-router";
import { api, errorText } from "../api/client";

/** "Make my own version": copies the version into the household and opens the copy in the editor. The dish's
 * cached pages are dropped first, so coming back from the editor shows the copy among the versions. */
export function OwnVersion({ variantId }: { variantId: string }) {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [busy, setBusy] = useState(false);
  // The failure belongs to the version it was for; choosing another version clears it.
  const [failure, setFailure] = useState<{ variantId: string; text: string } | null>(null);
  const error = failure?.variantId === variantId ? failure.text : "";

  const fork = async () => {
    setBusy(true);
    setFailure(null);
    const { data, error: failed } = await api
      .POST("/api/v1/variants/{id}/fork", { params: { path: { id: variantId } } })
      .catch(() => ({
        data: undefined,
        error: { error: "can't reach the server, so nothing was copied" },
      }));
    setBusy(false);
    if (!data) return setFailure({ variantId, text: errorText(failed) });
    await queryClient.invalidateQueries({ queryKey: ["dish"] });
    navigate(`/recipes/${data.id}/edit`);
  };

  return (
    <>
      <button type="button" className="link" onClick={fork} disabled={busy}>
        {busy ? "Copying…" : "Make my own version"}
      </button>
      {error && <span className="form-error">{error}</span>}
    </>
  );
}
