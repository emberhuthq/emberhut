import { defineConfig } from "vitest/config";

// Logic checks in Node: the rules, the pause, the back-off, pairing links, versions, the popup's words, the wire dialect
// and the zip writer are pure; the modules that call Chrome's APIs are not loaded here.
export default defineConfig({
  test: { include: ["src/**/*.test.ts", "scripts/**/*.test.ts"], environment: "node" },
});
