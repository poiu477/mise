import { Link } from "react-router";

/** "Nothing here", for a path that isn't a page, with a way back to the dishes. */
export function NotFound() {
  return (
    <p className="empty">
      Nothing here. <Link to="/">Back to all dishes</Link>
    </p>
  );
}
