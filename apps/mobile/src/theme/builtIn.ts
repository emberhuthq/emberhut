import type { ThemePage, ThemeTokens } from "@/api";

// The blue preset as the server resolves it (version 1): graphite neutrals with
// a blue accent. What the app shows before Emberhut has ever answered, so the first launch looks right offline too.

const light: ThemeTokens = {
  accent: "#2563EB",
  accentSoft: "#E5ECFD",
  onAccent: "#FFFFFF",
  background: "#F5F6F8",
  surface: "#FFFFFF",
  surfaceRaised: "#ECEEF1",
  border: "#D9DCE1",
  text: "#14171C",
  muted: "#5B6270",
  faint: "#8A909B",
  success: "#15803D",
  warn: "#C2410C",
  danger: "#DC2626",
  info: "#0E7490",
};

const dark: ThemeTokens = {
  accent: "#60A5FA",
  accentSoft: "#27384E",
  onAccent: "#0B0D10",
  background: "#0E1013",
  surface: "#17191D",
  surfaceRaised: "#202328",
  border: "#2C3036",
  text: "#E9EBEE",
  muted: "#9BA1AB",
  faint: "#6B717B",
  success: "#4ADE80",
  warn: "#FB923C",
  danger: "#F87171",
  info: "#22D3EE",
};

export const builtInTheme: ThemePage = {
  version: 1,
  choice: { preset: "blue", appearance: "system", accent: null, at: null },
  tokens: { light, dark },
  contrast: {
    light: { accentOnBackground: 4.77, accentOnSurface: 5.16, onAccent: 5.16, adjusted: false },
    dark: { accentOnBackground: 7.49, accentOnSurface: 6.92, onAccent: 7.65, adjusted: false },
    aa: true,
  },
  presets: [{ id: "blue", name: "Blue", light, dark }],
  names: Object.keys(light) as (keyof ThemeTokens)[],
};
