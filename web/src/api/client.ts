// Typed API client; types are generated from the server's OpenAPI document (npm run api).
import createClient from "openapi-fetch";
import type { components, paths } from "./schema";

/** The typed client for mise's API: `api.GET("/api/v1/dishes/{slug}", …)` and so on, sent with the session
 * cookie. */
export const api = createClient<paths>({ baseUrl: "", credentials: "same-origin" });

let onUnauthorized: (() => void) | null = null;
/** Calls `f` when the session ends while the app is open (it expired, or signed out elsewhere): any answer of
 * `unauthorized`. */
export const whenUnauthorized = (f: () => void) => {
  onUnauthorized = f;
};

let account: string | null = null;
/** Sets the account this page works as, sent with every request so the server refuses (412) work meant for one
 * person once another has signed in, in another tab, with the shared cookie. Asking who is signed in, and signing
 * in, go without it. The session sets it; `null` sends none. */
export const actAs = (id: string | null) => {
  account = id;
};
let onAccountChanged: (() => void) | null = null;
/** Calls `f` when the server refuses a request because another account has signed in on this browser since. */
export const whenAccountChanged = (f: () => void) => {
  onAccountChanged = f;
};
/** Whether a request only asks who is signed in, or signs in, so goes without the account. */
const asks = (path: string) => path === "/api/v1/me" || path.startsWith("/api/v1/auth/");

api.use({
  onRequest({ request }) {
    if (account && !asks(new URL(request.url).pathname))
      request.headers.set("x-mise-account", account);
    return request;
  },
  async onResponse({ response }) {
    if (response.ok) return response;
    const code = errorCode(
      await response
        .clone()
        .json()
        .catch(() => null),
    );
    // A wrong password is `bad_credentials`, so only an ended session reads as signed out.
    if (code === "unauthorized") onUnauthorized?.();
    if (code === "account_changed") onAccountChanged?.();
    return response;
  },
});

/** What kind of error the API answered with ("not_found", "account_changed"): stable, unlike its wording. */
export type ErrorCode = components["schemas"]["ErrorCode"];

/** The `code` of an error the API answered with; null for anything else, such as a proxy's own error page. */
export function errorCode(error: unknown): ErrorCode | null {
  if (error && typeof error === "object" && "code" in error) {
    const code = (error as { code: unknown }).code;
    if (typeof code === "string") return code as ErrorCode;
  }
  return null;
}

/** The API's error message as a sentence, for showing to people: "enter between 1 and 200 characters" reads
 * "Enter between 1 and 200 characters.". Anything else reads as `fallback`. */
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

/** A dish as search lists it: its title, picture and how many versions it has. */
export type DishSummary = components["schemas"]["DishSummary"];
/** One page of search results, and how many dishes match in all. */
export type DishPage = components["schemas"]["DishPage"];
/** A dish with its versions, newest first, and the options that choose between them. */
export type Dish = components["schemas"]["Dish"];
/** One toggle on a dish page (protein, portion or version) and its choices: "Regular", "Double protein". */
export type DishOption = components["schemas"]["DishOption"];
/** One version of a dish as the dish page lists it: its option values, sizes and whether it has steps. */
export type VariantSummary = components["schemas"]["VariantSummary"];
/** One version of a dish at a serving count: ingredients with their amounts, and steps. */
export type Variant = components["schemas"]["Variant"];
/** One ingredient line at the requested serving count: "1½ TBSP olive oil", or no amount for salt. */
export type Ingredient = components["schemas"]["Ingredient"];
/** One cooking step, with flags for wording or amounts worth a second look at this size. */
export type Step = components["schemas"]["Step"];
/** A stored photo, with links to its original, large and small copies. */
export type Image = components["schemas"]["Image"];
/** How people can sign in here (password, single sign-on), and whether first-run setup is still to do. */
export type AuthInfo = components["schemas"]["AuthInfo"];
/** The signed-in account: names, household and whether it is an admin. */
export type Me = components["schemas"]["Me"];
/** An API token as its owner sees it: name, scope ("read" or "read_write") and when it was last used. */
export type TokenInfo = components["schemas"]["TokenInfo"];
/** An account as an admin sees it on the account page. */
export type UserInfo = components["schemas"]["UserInfo"];
/** A household: the people who share a plan and a shopping list. */
export type Household = components["schemas"]["Household"];
/** A household's plan for one week, named by its Monday: the dishes and how many each serves. */
export type Plan = components["schemas"]["Plan"];
/** One dish on a week's plan, at its serving count. */
export type PlanEntry = components["schemas"]["PlanEntry"];
/** A week's shopping list: items by shop section, things added by hand, and what was left off. */
export type ShoppingList = components["schemas"]["ShoppingList"];
/** One line on the shopping list, added up across the week's dishes: "3 packets (4½ oz)" of something. */
export type ShoppingItem = components["schemas"]["ShoppingItem"];
/** Something to add to a week's list by hand, as sent: its name, the week and the device's key for it. */
export type NewExtra = components["schemas"]["NewExtra"];
/** How a list reads weights: "lb_oz", "oz", "lb" or "metric". */
export type Weights = components["schemas"]["Weights"];
