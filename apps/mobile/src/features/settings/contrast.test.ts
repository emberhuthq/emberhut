import { describe, expect, it } from "vitest";
import { builtInTheme } from "@/theme/builtIn";
import { contrastLines, normalHex } from "./contrast";

describe("custom colour", () => {
  it("reads a hex as the server does", () => {
    expect(normalHex("#36f")).toBe("#3366FF");
    expect(normalHex(" 3366ff ")).toBe("#3366FF");
    expect(normalHex("#3366F")).toBeNull();
    expect(normalHex("blue")).toBeNull();
  });

  it("says how it reads, never rounding up past AA", () => {
    expect(contrastLines(builtInTheme, "light")).toEqual(["Text on it 5.1:1 · AA", "As text on the background 4.7:1"]);
    const close = {
      ...builtInTheme,
      contrast: { ...builtInTheme.contrast, light: { ...builtInTheme.contrast.light, onAccent: 4.48 } },
    };
    expect(contrastLines(close, "light")[0]).toBe("Text on it 4.4:1 · below AA");
  });

  it("says what was changed for readability, this appearance first", () => {
    const yellow = {
      ...builtInTheme,
      tokens: { light: { ...builtInTheme.tokens.light, accent: "#8A7400" }, dark: { ...builtInTheme.tokens.dark, accent: "#FFE000" } },
      contrast: {
        light: { ...builtInTheme.contrast.light, adjusted: true, asked: "#FFE000" },
        dark: { ...builtInTheme.contrast.dark, adjusted: false },
        aa: true,
      },
    };
    expect(contrastLines(yellow, "dark")).toEqual([
      "Text on it 7.6:1 · AA",
      "As text on the background 7.4:1",
      "Darkened to #8A7400 in light mode, so it reads",
    ]);
  });
});
