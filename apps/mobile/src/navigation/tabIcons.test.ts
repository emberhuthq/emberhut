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

  it("leave your ember's tab to the ember itself", () => {
    expect(Object.keys(materialTabIcons)).not.toContain("Chat");
  });
});

describe("Your ember's tab icon", () => {
  const faces = readFileSync(new URL("../ui/emberFaces.ts", import.meta.url), "utf8");
  // The tints the server lists (the API's fixture), the default drawn as the brand's own face.
  const page = JSON.parse(
    readFileSync(new URL("../../../../crates/emberhut-api/tests/fixtures/theme_page.json", import.meta.url), "utf8"),
  ) as { tints: { id: string }[] };

  it("ships and bundles the ember in every tint the server lists, and off duty", () => {
    const names = [...page.tints.map((t) => (t.id === "ember" ? "emberhut-face" : `emberhut-face-${t.id}`)), "emberhut-face-off"];
    expect(names).toContain("emberhut-face-blue");
    for (const name of names) {
      expect(existsSync(new URL(`../../assets/${name}.png`, import.meta.url)), name).toBe(true);
      expect(faces).toContain(`require("../../assets/${name}.png")`);
    }
  });
});
