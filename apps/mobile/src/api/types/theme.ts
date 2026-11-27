import type { Instant, Reply } from "./common";

/** Light, dark, or the phone's own. */
export type Appearance = "system" | "light" | "dark";

/** One appearance's colours, every one "#RRGGBB" (the server documents what each is for). */
export interface ThemeTokens {
  accent: string;
  accentSoft: string;
  onAccent: string;
  background: string;
  surface: string;
  surfaceRaised: string;
  border: string;
  text: string;
  muted: string;
  faint: string;
  success: string;
  warn: string;
  danger: string;
  info: string;
}

/** Your choice, as `GET /api/state` carries it too (`theme`): a newer `at` means another device changed it. */
export interface ThemeChoice {
  preset: string;
  appearance: Appearance;
  /** A custom accent in place of the preset's; null for the preset's own. */
  accent: string | null;
  /** When it last changed; null: never set (the default). */
  at: Instant | null;
}

/** How the accent reads in one appearance (WCAG ratios, 1 to 21). */
export interface ThemeContrast {
  accentOnBackground: number;
  accentOnSurface: number;
  /** The text on an accent fill. */
  onAccent: number;
  /** The accent asked for was darkened (light) or lightened (dark) to read. */
  adjusted: boolean;
  /** The accent asked for, before that. */
  asked?: string;
}

export interface ThemePreset {
  id: string;
  name: string;
  light: ThemeTokens;
  dark: ThemeTokens;
}

/** GET /api/theme: the choice resolved, and every preset so a preset can be switched offline. */
export interface ThemePage {
  /** The presets' values: a cache is kept per version. */
  version: number;
  choice: ThemeChoice;
  tokens: { light: ThemeTokens; dark: ThemeTokens };
  contrast: { light: ThemeContrast; dark: ThemeContrast; aa: boolean };
  presets: ThemePreset[];
  names: (keyof ThemeTokens)[];
}

/** POST /api/theme: the fields given change, the rest stay. */
export interface ThemeChange {
  preset?: string;
  appearance?: Appearance;
  /** "#RRGGBB" or "#RGB"; null goes back to the preset's own. */
  accent?: string | null;
}

export type ThemeReply = Reply & ThemePage;
