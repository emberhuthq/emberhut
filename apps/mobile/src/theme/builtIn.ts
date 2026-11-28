import type { ThemePage, ThemePreset, ThemeTokens } from "@/api";

// The paper preset as the server resolves it (version 1), the server's default: kraft paper and brown-black ink with an
// ember accent, the blue preset beside it to pick, and the ember's tints. What the app shows before Emberhut has ever
// answered, so the first launch looks right offline too.

const light: ThemeTokens = {
  accent: "#B83D0C",
  accentSoft: "#F6E6DD",
  onAccent: "#FFF8EF",
  background: "#F6F0E6",
  surface: "#FFFDF9",
  surfaceRaised: "#EFE6D8",
  border: "#DCD6CD",
  text: "#1D1712",
  muted: "#65574C",
  faint: "#6E6055",
  success: "#2B7248",
  warn: "#8A5800",
  danger: "#A51D2D",
  info: "#2B5F8E",
};

const dark: ThemeTokens = {
  accent: "#FF9A5A",
  accentSoft: "#523725",
  onAccent: "#1D1206",
  background: "#15110E",
  surface: "#211B16",
  surfaceRaised: "#261F19",
  border: "#2C2723",
  text: "#F6EEE5",
  muted: "#BDAE9F",
  faint: "#A39486",
  success: "#7FD19B",
  warn: "#E8B04A",
  danger: "#FF8089",
  info: "#8DB8E0",
};

// Blue on the graphite neutrals.
const blue: ThemePreset = {
  id: "blue",
  name: "Blue",
  light: {
    accent: "#2563EB",
    accentSoft: "#E5ECFD",
    onAccent: "#FFFFFF",
    background: "#F5F6F8",
    surface: "#FFFFFF",
    surfaceRaised: "#ECEEF1",
    border: "#D9DCE1",
    text: "#14171C",
    muted: "#5B6270",
    faint: "#646B76",
    success: "#166534",
    warn: "#985A00",
    danger: "#B91C1C",
    info: "#0E7490",
  },
  dark: {
    accent: "#60A5FA",
    accentSoft: "#27384E",
    onAccent: "#0B0D10",
    background: "#0E1013",
    surface: "#17191D",
    surfaceRaised: "#202328",
    border: "#2C3036",
    text: "#E9EBEE",
    muted: "#9BA1AB",
    faint: "#8D939D",
    success: "#4ADE80",
    warn: "#FACC15",
    danger: "#F87171",
    info: "#22D3EE",
  },
};

export const builtInTheme: ThemePage = {
  version: 1,
  choice: { preset: "paper", appearance: "system", accent: null, emberTint: "ember", at: null },
  tokens: { light, dark },
  contrast: {
    light: { accentOnBackground: 4.99, accentOnSurface: 5.57, onAccent: 5.36, adjusted: false },
    dark: { accentOnBackground: 8.96, accentOnSurface: 8.12, onAccent: 8.78, adjusted: false },
    aa: true,
  },
  presets: [{ id: "paper", name: "Paper", light, dark }, blue],
  names: Object.keys(light) as (keyof ThemeTokens)[],
  tints: [
    { id: "ember", name: "Ember" },
    { id: "amber", name: "Amber" },
    { id: "rose", name: "Rose" },
    { id: "sage", name: "Sage" },
  ],
};
