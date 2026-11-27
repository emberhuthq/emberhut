import { describe, expect, it } from "vitest";
import type { ThemeTokens } from "@/api";
import { builtInTheme } from "./builtIn";
import { mix, paletteFrom } from "./colors";

// WCAG's ratio, to check what the palette derives.
function ratio(a: string, b: string): number {
  const lum = (hex: string) =>
    [1, 3, 5]
      .map((i) => parseInt(hex.slice(i, i + 2), 16) / 255)
      .map((c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4))
      .reduce((sum, c, i) => sum + c * [0.2126, 0.7152, 0.0722][i]!, 0);
  const [x, y] = [lum(a), lum(b)];
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}

describe("palette from the theme's tokens", () => {
  const { light, dark } = builtInTheme.tokens;

  it("maps every token to the names the screens use, needs you in the theme's accent", () => {
    expect(paletteFrom(light, "light")).toEqual({
      background: "#F5F6F8",
      surface: "#FFFFFF",
      surfaceRaised: "#ECEEF1",
      border: "#D9DCE1",
      text: "#14171C",
      textSecondary: "#5B6270",
      textTertiary: "#8A909B",
      accent: "#2563EB",
      accentSoft: "#E5ECFD",
      onAccent: "#FFFFFF",
      primary: "#2563EB",
      onPrimary: "#FFFFFF",
      onStatus: "#FFFFFF",
      switchThumb: "#FFFFFF",
      needsYou: "#2563EB",
      working: "#0E7490",
      done: "#15803D",
      warning: "#C2410C",
      danger: "#DC2626",
      bubbleMine: "#2563EB",
      onBubbleMine: "#FFFFFF",
    });
  });

  it("gives dark a calmer bubble of your own, the status text in the background colour", () => {
    const p = paletteFrom(dark, "dark");
    expect(p).toMatchObject({ needsYou: "#60A5FA", onPrimary: "#0B0D10", onStatus: "#0E1013", switchThumb: "#E9EBEE" });
    expect(p.bubbleMine).toBe(mix("#60A5FA", "#17191D", 0.35));
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

  it("mixes as the server does", () => {
    expect(mix("#2563EB", "#FFFFFF", 0.12)).toBe("#E5ECFD");
    expect(mix("#60A5FA", "#17191D", 0.22)).toBe("#27384E");
  });
});
