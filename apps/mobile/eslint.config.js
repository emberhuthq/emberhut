// Expo's flat preset (react, hooks, import, TypeScript), then the TypeScript rules the code keeps to, type-aware where
// they need types; prettier last so formatting is Prettier's alone.
const { defineConfig } = require("eslint/config");
const comments = require("@eslint-community/eslint-plugin-eslint-comments/configs");
const expoConfig = require("eslint-config-expo/flat");
const prettierConfig = require("eslint-config-prettier/flat");

module.exports = defineConfig([
  expoConfig,
  {
    files: ["**/*.{ts,tsx}"],
    languageOptions: { parserOptions: { projectService: true } },
    rules: {
      "@typescript-eslint/no-explicit-any": "error",
      "@typescript-eslint/ban-ts-comment": ["error", { "ts-expect-error": "allow-with-description", minimumDescriptionLength: 10 }],
      // A promise nobody awaits must handle its rejection (.catch, or inBackground from src/log): `void` alone doesn't.
      "@typescript-eslint/no-floating-promises": ["error", { ignoreVoid: false }],
      "@typescript-eslint/no-misused-promises": "error",
    },
  },
  { rules: { "no-console": "error" } },
  // Every eslint-disable names its rule and says why.
  comments.recommended,
  { rules: { "@eslint-community/eslint-comments/require-description": ["error", { ignore: [] }] } },
  prettierConfig,
  {
    ignores: ["node_modules/**", "android/**", "ios/**", ".expo/**", "dist/**"],
  },
]);
