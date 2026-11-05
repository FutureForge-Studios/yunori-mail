import js from "@eslint/js";
import tseslint from "typescript-eslint";
import reactHooks from "eslint-plugin-react-hooks";
import globals from "globals";

/**
 * Mirrors the Yunori client's flat config (pragmatic: real-bug rules + classic
 * hooks rules). Vendored design files are linted upstream, so they're skipped.
 */
export default tseslint.config(
  {
    ignores: ["dist/**", "dist-mock/**", "node_modules/**", "scripts/**", "src/design/vendor/**", "*.config.{js,ts,mjs,cjs}"],
  },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["**/*.{ts,tsx}"],
    languageOptions: {
      ecmaVersion: 2022,
      sourceType: "module",
      globals: { ...globals.browser },
    },
    plugins: { "react-hooks": reactHooks },
    rules: {
      "react-hooks/rules-of-hooks": "error",
      "react-hooks/exhaustive-deps": "warn",
      "@typescript-eslint/no-explicit-any": "error",
      "@typescript-eslint/no-non-null-assertion": "off",
      "@typescript-eslint/no-unused-vars": [
        "warn",
        { argsIgnorePattern: "^_", varsIgnorePattern: "^_", ignoreRestSiblings: true },
      ],
      "no-empty": ["warn", { allowEmptyCatch: true }],
    },
  },
);
