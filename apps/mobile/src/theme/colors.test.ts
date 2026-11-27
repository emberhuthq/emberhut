import { describe, expect, it } from "vitest";
import type { ThemeTokens } from "@/api";
import { builtInTheme } from "./builtIn";
import { contrast as ratio, mix, paletteFrom } from "./colors";

describe("palette from the theme's tokens", () => {
  const { light, dark } = builtInTheme.tokens;

  it("maps every token to the names the screens use, needs you in the theme's accent", () => {
    expect(paletteFrom(light, "light")).toEqual({
      background: "#F6F0E6",
      surface: "#FFFDF9",
      surfaceRaised: "#EFE6D8",
      border: "#DCD6CD",
      text: "#1D1712",
      textSecondary: "#65574C",
      textTertiary: "#6E6055",
      accent: "#B83D0C",
      accentSoft: "#F6E6DD",
      onAccent: "#FFF8EF",
      primary: "#B83D0C",
      onPrimary: "#FFF8EF",
      onStatus: "#FFFDF9",
      switchThumb: "#FFFDF9",
      needsYou: "#B83D0C",
      working: "#2B5F8E",
      done: "#2B7248",
      warning: "#8A5800",
      danger: "#A51D2D",
      bubbleMine: "#B83D0C",
      onBubbleMine: "#FFF8EF",
    });
  });

  it("gives dark a calmer bubble of your own, the status text in the background colour", () => {
    const p = paletteFrom(dark, "dark");
    expect(p).toMatchObject({ needsYou: "#FF9A5A", onPrimary: "#1D1206", onStatus: "#15110E", switchThumb: "#F6EEE5" });
    expect(p.bubbleMine).toBe(mix("#FF9A5A", "#211B16", 0.35));
    expect(p.onBubbleMine).toBe(dark.text);
  });

  it("follows a custom accent everywhere it shows", () => {
    const custom: ThemeTokens = { ...light, accent: "#BE123C", accentSoft: "#F8E8EC" };
    const p = paletteFrom(custom, "light");
    expect([p.accent, p.primary, p.needsYou, p.bubbleMine]).toEqual(["#BE123C", "#BE123C", "#BE123C", "#BE123C"]);
  });

  it("keeps the dark bubble's text readable for any accent, white the worst", () => {
    for (const accent of ["#FFFFFF", "#FFFF00", "#00FFFF", "#60A5FA", "#A78BFA"]) {
      const p = paletteFrom({ ...dark, accent }, "dark");
      expect(ratio(p.onBubbleMine, p.bubbleMine)).toBeGreaterThanOrEqual(4.5);
    }
  });

  it("measures contrast as WCAG 2 does", () => {
    expect(ratio("#000000", "#FFFFFF")).toBeCloseTo(21, 9);
    expect(ratio("#123456", "#123456")).toBe(1);
    expect(ratio("#2563EB", "#FFFFFF")).toBeCloseTo(5.17, 2);
  });

  it("mixes as the server does", () => {
    expect(mix("#2563EB", "#FFFFFF", 0.12)).toBe("#E5ECFD");
    expect(mix("#60A5FA", "#17191D", 0.22)).toBe("#27384E");
    expect(mix("#B83D0C", "#FFFDF9", 0.12)).toBe(light.accentSoft);
    expect(mix("#FF9A5A", "#211B16", 0.22)).toBe(dark.accentSoft);
  });

  it("starts in paper, every text and status colour 4.5:1 on every ground, blue beside it", () => {
    expect(builtInTheme.choice.preset).toBe("paper");
    expect(builtInTheme.presets.map((p) => p.id)).toEqual(["paper", "blue"]);
    expect(builtInTheme.presets[0]).toMatchObject({ light, dark });
    for (const [scheme, t] of [
      ["light", light],
      ["dark", dark],
    ] as const) {
      const p = paletteFrom(t, scheme);
      for (const ground of [p.background, p.surface, p.surfaceRaised]) {
        for (const ink of [p.text, p.textSecondary, p.textTertiary, p.accent, p.working, p.done, p.warning, p.danger]) {
          expect(ratio(ink, ground), `${scheme} ${ink} on ${ground}`).toBeGreaterThanOrEqual(4.5);
        }
      }
      expect(ratio(p.onPrimary, p.primary)).toBeGreaterThanOrEqual(4.5);
      expect(ratio(p.onBubbleMine, p.bubbleMine)).toBeGreaterThanOrEqual(4.5);
    }
  });
});
