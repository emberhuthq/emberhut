import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";

// Logic checks in Node: the API layer and the stores take their native pieces (fetch, secure storage) as arguments; the
// config plugins are checked on the template files they edit.
export default defineConfig({
  resolve: { alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) } },
  test: { include: ["src/**/*.test.{ts,tsx}", "plugins/**/*.test.ts"], environment: "node" },
});
