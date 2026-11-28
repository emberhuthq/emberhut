import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { defineConfig } from "vitest/config";

// The runner's logic in Node: the wire shapes against the server's contract fixtures, account picking, config, the
// HTTP API over a stand-in agent and server. No test starts a real agent, and the home folder is a fresh temporary one,
// so nothing reads or links the real agent folder.
export default defineConfig({
  test: {
    include: ["test/**/*.test.ts"],
    environment: "node",
    pool: "forks",
    testTimeout: 15_000,
    env: { HOME: mkdtempSync(join(tmpdir(), "emberhut-runner-home-")), USERPROFILE: "" },
  },
});
