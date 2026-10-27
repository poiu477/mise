import { Link } from "react-router";

export function NotFound() {
  return (
    <p className="empty">
      Nothing here. <Link to="/">Back to all dishes</Link>
    </p>
  );
}
