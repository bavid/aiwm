import js from "@eslint/js";
import globals from "globals";
import tseslint from "typescript-eslint";
import jsxA11y from "eslint-plugin-jsx-a11y";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";

export default tseslint.config(
  { ignores: ["dist"] },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    // Node scripts (`pnpm lint:help`), not browser code.
    files: ["scripts/**/*.mjs"],
    languageOptions: {
      ecmaVersion: 2022,
      globals: globals.node,
    },
  },
  {
    // Static accessibility checks on the JSX (labels, alt text, roles,
    // keyboard handlers) — the `?` hints and fit badges rely on these
    // conventions holding across the whole tree.
    files: ["**/*.tsx"],
    ...jsxA11y.flatConfigs.recommended,
    rules: {
      ...jsxA11y.flatConfigs.recommended.rules,
      // Every `autoFocus` in this tree is on an inline editor (a rename or
      // create field) that mounts in response to a click; moving focus into
      // what the person just opened is the expected behaviour, not the
      // page-load focus steal the rule is about.
      "jsx-a11y/no-autofocus": "off",
      // The <audio>/<video> elements play media this app generated (a
      // narration, a rendered clip); there is no caption track to offer.
      "jsx-a11y/media-has-caption": "off",
      // Radio rows nest their text two spans deep (name + meta line).
      "jsx-a11y/label-has-associated-control": ["error", { depth: 3 }],
    },
  },
  {
    files: ["**/*.{ts,tsx}"],
    languageOptions: {
      ecmaVersion: 2022,
      globals: globals.browser,
    },
    plugins: {
      "react-hooks": reactHooks,
      "react-refresh": reactRefresh,
    },
    rules: {
      ...reactHooks.configs.recommended.rules,
      "react-refresh/only-export-components": [
        "warn",
        { allowConstantExport: true },
      ],
    },
  },
);
