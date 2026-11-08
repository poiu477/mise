// Typed API client; types are generated from the server's OpenAPI document (npm run api).
import createClient from "openapi-fetch";
import type { components, paths } from "./schema";

export const api = createClient<paths>({ baseUrl: "", credentials: "same-origin" });

// A 401 outside sign-in itself means the session ended (expired, signed out elsewhere): tell the app.
let onUnauthorized: (() => void) | null = null;
export const whenUnauthorized = (f: () => void) => {
  onUnauthorized = f;
};

// The account this page works as, sent with every request so the server refuses (412) work meant for one person
// once another has signed in, in another tab, with the shared cookie. Asking who is signed in, and signing in,
// go without it. Set by the session.
let account: string | null = null;
export const actAs = (id: string | null) => {
  account = id;
};
let onAccountChanged: (() => void) | null = null;
export const whenAccountChanged = (f: () => void) => {
  onAccountChanged = f;
};
const asks = (path: string) => path === "/api/me" || path.startsWith("/api/auth/");

api.use({
  onRequest({ request }) {
    if (account && !asks(new URL(request.url).pathname))
      request.headers.set("x-mise-account", account);
    return request;
  },
  onResponse({ response }) {
    const path = new URL(response.url).pathname;
    if (response.status === 401 && !path.startsWith("/api/auth/")) onUnauthorized?.();
    if (response.status === 412) onAccountChanged?.();
    return response;
  },
});

/** The API's error message, for showing to people. */
export function errorText(error: unknown, fallback = "Something went wrong. Try again."): string {
  if (
    error &&
    typeof error === "object" &&
    "error" in error &&
    typeof (error as { error: unknown }).error === "string"
  ) {
    const text = (error as { error: string }).error;
    return text.charAt(0).toUpperCase() + text.slice(1) + (/[.!?]$/.test(text) ? "" : ".");
  }
  return fallback;
}

export type DishSummary = components["schemas"]["DishSummary"];
export type DishPage = components["schemas"]["DishPage"];
export type Dish = components["schemas"]["Dish"];
export type DishOption = components["schemas"]["DishOption"];
export type VariantSummary = components["schemas"]["VariantSummary"];
export type Variant = components["schemas"]["Variant"];
export type Ingredient = components["schemas"]["Ingredient"];
export type Step = components["schemas"]["Step"];
export type Image = components["schemas"]["Image"];
export type AuthInfo = components["schemas"]["AuthInfo"];
export type Me = components["schemas"]["Me"];
export type TokenInfo = components["schemas"]["TokenInfo"];
export type UserInfo = components["schemas"]["UserInfo"];
export type Household = components["schemas"]["Household"];
export type Plan = components["schemas"]["Plan"];
export type PlanEntry = components["schemas"]["PlanEntry"];
export type ShoppingList = components["schemas"]["ShoppingList"];
export type ShoppingItem = components["schemas"]["ShoppingItem"];
/** Something to add to a week's list by hand, as sent: its name, the week and the device's key for it. */
export type NewExtra = components["schemas"]["NewExtra"];
export type Weights = components["schemas"]["Weights"];
