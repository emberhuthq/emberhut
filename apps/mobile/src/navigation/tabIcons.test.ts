import { existsSync, readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { materialTabIcons } from "./tabIconNames";

// The PNGs can't be required in Node, so the test reads what tabIcons.ts requires and what's on disk.
const assets = new URL("../../assets/tabs/", import.meta.url);
const source = readFileSync(new URL("./tabIcons.ts", import.meta.url), "utf8");

describe("Android tab icons", () => {
  const names = [...new Set(Object.values(materialTabIcons).flat())];

  it("ship every icon each tab names, at 1x, 2x and 3x", () => {
    for (const name of names)
      for (const suffix of ["", "@2x", "@3x"]) expect(existsSync(new URL(`${name}${suffix}.png`, assets)), `${name}${suffix}`).toBe(true);
  });

  it("bundle each one by a literal require", () => {
    for (const name of names) expect(source).toContain(`require("../../assets/tabs/${name}.png")`);
  });
});
