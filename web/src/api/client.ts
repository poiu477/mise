// Typed API client; types are generated from the server's OpenAPI document (npm run api).
import createClient from "openapi-fetch";
import type { components, paths } from "./schema";

export const api = createClient<paths>({ baseUrl: "", credentials: "same-origin" });

// A 401 outside sign-in itself means the session ended (expired, signed out elsewhere): tell the app.
let onUnauthorized: (() => void) | null = null;
export const whenUnauthorized = (f: () => void) => {
  onUnauthorized = f;
};
api.use({
  onResponse({ response }) {
    if (response.status === 401 && !new URL(response.url).pathname.startsWith("/api/auth/")) onUnauthorized?.();
    return response;
  },
});

/** The API's error message, for showing to people. */
export function errorText(error: unknown, fallback = "Something went wrong. Try again."): string {
  if (error && typeof error === "object" && "error" in error && typeof (error as { error: unknown }).error === "string") {
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
