// The TypeScript rules the code keeps to, type-aware where they need types (as apps/extension's); prettier last so
// formatting is Prettier's alone.
import js from "@eslint/js";
import comments from "@eslint-community/eslint-plugin-eslint-comments/configs";
import prettierConfig from "eslint-config-prettier/flat";
import { defineConfig } from "eslint/config";
import tseslint from "typescript-eslint";

export default defineConfig([
  { ignores: ["node_modules/**", "dist/**"] },
  js.configs.recommended,
  tseslint.configs.recommendedTypeChecked,
  {
    languageOptions: { parserOptions: { projectService: true, tsconfigRootDir: import.meta.dirname } },
    rules: {
      "@typescript-eslint/no-explicit-any": "error",
      "@typescript-eslint/ban-ts-comment": ["error", { "ts-expect-error": "allow-with-description", minimumDescriptionLength: 10 }],
      // A promise nobody awaits must handle its rejection (.catch, or inBackground from src/log.ts): `void` alone doesn't.
      "@typescript-eslint/no-floating-promises": ["error", { ignoreVoid: false }],
      "@typescript-eslint/no-misused-promises": "error",
      "no-console": "error",
    },
  },
  { files: ["**/*.js"], extends: [tseslint.configs.disableTypeChecked] },
  // Every eslint-disable names its rule and says why.
  comments.recommended,
  { rules: { "@eslint-community/eslint-comments/require-description": ["error", { ignore: [] }] } },
  prettierConfig,
]);
