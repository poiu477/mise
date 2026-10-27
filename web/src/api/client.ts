// Typed API client; types are generated from the server's OpenAPI document (npm run api).
import createClient from "openapi-fetch";
import type { components, paths } from "./schema";

export const api = createClient<paths>({ baseUrl: "" });

export type DishSummary = components["schemas"]["DishSummary"];
export type DishPage = components["schemas"]["DishPage"];
export type Dish = components["schemas"]["Dish"];
export type DishOption = components["schemas"]["DishOption"];
export type VariantSummary = components["schemas"]["VariantSummary"];
export type Variant = components["schemas"]["Variant"];
export type Ingredient = components["schemas"]["Ingredient"];
export type Step = components["schemas"]["Step"];
export type Image = components["schemas"]["Image"];
