import type { ThemeTokens } from "@/api";

/** What the screens paint with, mapped from one appearance of the server's theme tokens (GET /api/theme). */
export interface Palette {
  /** The screen behind everything. */
  background: string;
  /** Cards, rows, sheets. */
  surface: string;
  /** A surface on a surface: a pressed row, an input. */
  surfaceRaised: string;
  border: string;
  text: string;
  textSecondary: string;
  textTertiary: string;
  /** The theme's colour: links, selection, progress. */
  accent: string;
  /** A pale accent fill behind accent text or a selected thing. */
  accentSoft: string;
  /** Text and icons on an accent fill. */
  onAccent: string;
  /** The primary button's fill and the text on it. */
  primary: string;
  onPrimary: string;
  /** Text and icons on a status fill (done, danger, warning, working). */
  onStatus: string;
  /** An on switch's thumb, on the accent track and still seen against the dark rows. */
  switchThumb: string;
  /** What waits on you: the theme's accent. */
  needsYou: string;
  /** Running: a run going on a task, a step being worked on. The theme's warn, as on the desktop; always drawn with a shape (a spinner, a ring) so it isn't colour alone. */
  working: string;
  /** Queued, and other quiet information. */
  info: string;
  done: string;
  warning: string;
  danger: string;
  /** Your own chat bubble and its text. */
  bubbleMine: string;
  onBubbleMine: string;
}

/** The palette of one appearance. */
export function paletteFrom(t: ThemeTokens, scheme: "light" | "dark"): Palette {
  const dark = scheme === "dark";
  return {
    background: t.background,
    surface: t.surface,
    surfaceRaised: t.surfaceRaised,
    border: t.border,
    text: t.text,
    textSecondary: t.muted,
    textTertiary: t.faint,
    accent: t.accent,
    accentSoft: t.accentSoft,
    onAccent: t.onAccent,
    primary: t.accent,
    onPrimary: t.onAccent,
    // The status colours are deep in light (white reads on them) and light in dark (the background reads on them).
    onStatus: dark ? t.background : t.surface,
    switchThumb: dark ? t.text : t.surface,
    needsYou: t.accent,
    working: t.warn,
    info: t.info,
    done: t.success,
    warning: t.warn,
    danger: t.danger,
    // Light: the accent itself, as in the reference client. Dark: a bright accent fill with dark text shouts in a long chat, so
    // a calmer one, the accent sunk into the surface; at 35% the light text still reads 4.5:1 on it for any accent.
    bubbleMine: dark ? mix(t.accent, t.surface, 0.35) : t.accent,
    onBubbleMine: dark ? t.text : t.onAccent,
  };
}

/** WCAG 2's contrast ratio of two "#RRGGBB" colours: 1 (the same) to 21 (black on white). */
export function contrast(a: string, b: string): number {
  const luminance = (hex: string) =>
    [1, 3, 5]
      .map((i) => parseInt(hex.slice(i, i + 2), 16) / 255)
      .map((c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4))
      .reduce((sum, c, i) => sum + c * [0.2126, 0.7152, 0.0722][i]!, 0);
  const [x, y] = [luminance(a), luminance(b)];
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}

/** The near-black text and icons take on a light fill, as on the desktop. */
export const ink = "#0B0D10";

/** White or near-black, whichever reads better on `fill`: for text on a coloured fill that isn't the accent (which has its own). */
export function readableOn(fill: string): string {
  return contrast("#FFFFFF", fill) >= contrast(ink, fill) ? "#FFFFFF" : ink;
}

/** `share` of colour `a` into `b`, both "#RRGGBB". */
export function mix(a: string, b: string, share: number): string {
  const channel = (hex: string, i: number) => parseInt(hex.slice(1 + i * 2, 3 + i * 2), 16);
  return `#${[0, 1, 2]
    .map((i) => Math.round(channel(a, i) * share + channel(b, i) * (1 - share)))
    .map((c) => c.toString(16).toUpperCase().padStart(2, "0"))
    .join("")}`;
}
