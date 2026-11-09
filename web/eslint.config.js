// Lint rules for the web app: ESLint's and typescript-eslint's recommended sets, the React hooks rules, a doc
// comment on everything a module exports, and nothing that fights Prettier, which owns formatting.
import js from "@eslint/js";
import jsdoc from "eslint-plugin-jsdoc";
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
      // Pages load server data through TanStack Query (src/api/query.ts), never by setting state in an effect.
      "react-hooks/set-state-in-effect": "error",
    },
  },
  {
    // style.md: every exported function, component, hook, class and type says what it is or does. Components and
    // hooks are written both as function declarations and as `const` arrow functions, so both count; functions
    // inside an exported object or call are the exported thing's parts, documented with it.
    files: ["src/**/*.{ts,tsx}"],
    plugins: { jsdoc },
    rules: {
      "jsdoc/require-jsdoc": [
        "error",
        {
          publicOnly: true,
          require: { FunctionDeclaration: true, ClassDeclaration: true },
          contexts: [
            "VariableDeclarator > ArrowFunctionExpression",
            "VariableDeclarator > FunctionExpression",
            "ExportDefaultDeclaration > ArrowFunctionExpression",
            "TSTypeAliasDeclaration",
            "TSInterfaceDeclaration",
            "TSEnumDeclaration",
          ],
        },
      ],
    },
  },
  {
    files: ["public/sw.js"],
    extends: [js.configs.recommended],
    languageOptions: { globals: globals.serviceworker },
  },
  prettier,
);
