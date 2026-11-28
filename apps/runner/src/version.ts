// The runner's version: its package's, which a release sets from the app's version tag.
import { readFileSync } from "node:fs";

/** This runner's version. */
export const VERSION: string = (() => {
  try {
    const pkg = JSON.parse(readFileSync(new URL("../package.json", import.meta.url), "utf8")) as { version?: unknown };
    return typeof pkg.version === "string" ? pkg.version : "0.0.0";
  } catch {
    return "0.0.0";
  }
})();
