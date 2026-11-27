import { readdirSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

// The iOS build bundles on a Mac, whose disk ignores case: "./Lanes" there can load lanes.ts, the Android build's
// Linux never would. So no two modules in a folder may share a name that differs only in case.
const src = fileURLToPath(new URL("..", import.meta.url));

function clashes(dir: string): string[] {
  const entries = readdirSync(dir, { withFileTypes: true });
  const seen = new Map<string, string>();
  const found: string[] = [];
  for (const entry of entries) {
    if (entry.isDirectory()) {
      found.push(...clashes(join(dir, entry.name)));
      continue;
    }
    const module = entry.name.replace(/\.(tsx?|jsx?)$/, "");
    const other = seen.get(module.toLowerCase());
    if (other !== undefined && other !== module) found.push(`${join(dir, other)} and ${module}`);
    seen.set(module.toLowerCase(), module);
  }
  return found;
}

describe("module names", () => {
  it("never differ only in case within a folder", () => {
    expect(clashes(src)).toEqual([]);
  });
});
