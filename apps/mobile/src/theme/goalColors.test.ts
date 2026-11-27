import { describe, expect, it } from "vitest";
import { builtInTheme } from "./builtIn";
import { contrast } from "./colors";
import { goalColors } from "./goalColors";

describe("the goals' colors", () => {
  it("read 3:1 on every ground of the built-in presets, in each scheme", () => {
    // Paper and blue: between them every preset's neutrals (the others share blue's).
    for (const preset of builtInTheme.presets) {
      for (const scheme of ["light", "dark"] as const) {
        const t = preset[scheme];
        const { ranks, none } = goalColors(scheme);
        for (const color of [...ranks, none]) {
          for (const ground of [t.background, t.surface, t.surfaceRaised]) {
            expect(contrast(color, ground), `${preset.id} ${scheme} ${color} on ${ground}`).toBeGreaterThanOrEqual(3);
          }
        }
      }
    }
  });
});
