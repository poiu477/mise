// The web app's entry point: its pages and their paths, the providers every page needs, and the service worker
// that keeps the app opening without a signal.
import { QueryClientProvider } from "@tanstack/react-query";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { createBrowserRouter, RouterProvider } from "react-router";
import { Layout } from "./components/Layout";
import { SearchPage } from "./pages/SearchPage";
import { DishPage } from "./pages/DishPage";
import { NotFound } from "./pages/NotFound";
import { LoginPage } from "./pages/LoginPage";
import { SetupPage } from "./pages/SetupPage";
import { AccountPage } from "./pages/AccountPage";
import { PlanPage } from "./pages/PlanPage";
import { ShoppingPage } from "./pages/ShoppingPage";
import { queryClient } from "./api/query";
import { SessionProvider } from "./session";
import "./styles.css";

/** Every page and its path, inside the shared layout; anything else is "Nothing here". */
const router = createBrowserRouter([
  {
    element: <Layout />,
    children: [
      { path: "/", element: <SearchPage /> },
      { path: "/dishes/:slug", element: <DishPage /> },
      { path: "/plan", element: <PlanPage /> },
      { path: "/shopping", element: <ShoppingPage /> },
      { path: "/login", element: <LoginPage /> },
      { path: "/setup", element: <SetupPage /> },
      { path: "/account", element: <AccountPage /> },
      { path: "*", element: <NotFound /> },
    ],
  },
]);

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <SessionProvider>
        <RouterProvider router={router} />
      </SessionProvider>
    </QueryClientProvider>
  </StrictMode>,
);

if ("serviceWorker" in navigator && import.meta.env.PROD) {
  window.addEventListener("load", () => navigator.serviceWorker.register("/sw.js").catch(() => {}));
}
