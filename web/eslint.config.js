// Lint rules for the web app: ESLint's and typescript-eslint's recommended sets, the React hooks rules, and nothing
// that fights Prettier, which owns formatting.
import js from "@eslint/js";
import prettier from "eslint-config-prettier";
import reactHooks from "eslint-plugin-react-hooks";
import globals from "globals";
import tseslint from "typescript-eslint";

export default tseslint.config(
  { ignores: ["dist", "src/api/schema.d.ts"] },
  {
    files: ["**/*.{ts,tsx}"],
    extends: [
      js.configs.recommended,
      ...tseslint.configs.recommended,
      reactHooks.configs.flat.recommended,
    ],
    languageOptions: { globals: globals.browser },
    rules: {
      // Pages still reset and load their data inside effects; moving loading to a data-fetching layer is on the task
      // list (docs/status.md), after which this goes back to an error.
      "react-hooks/set-state-in-effect": "warn",
    },
  },
  {
    files: ["public/sw.js"],
    extends: [js.configs.recommended],
    languageOptions: { globals: globals.serviceworker },
  },
  prettier,
);
