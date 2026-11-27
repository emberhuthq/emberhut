import type { ThemeContrast, ThemePage } from "@/api";

/** WCAG AA for text: what the server holds an accent to. */
export const aa = 4.5;

/** "#36f", "3366FF", " #3366ff " → "#3366FF", as the server reads a hex; anything else null. */
export function normalHex(input: string): string | null {
  let h = input.trim().replace(/^#/, "");
  if (h.length === 3) h = [...h].map((c) => c + c).join("");
  return /^[0-9a-f]{6}$/i.test(h) ? `#${h.toUpperCase()}` : null;
}

// One decimal, never rounded up past the line: 4.48 says 4.4, not 4.5.
const ratio = (x: number) => `${(Math.floor(x * 10) / 10).toFixed(1)}:1`;

const readable = (c: ThemeContrast) => c.onAccent >= aa && c.accentOnBackground >= aa && c.accentOnSurface >= aa;

/** What a custom accent's preview says: how it reads in this appearance, and what was changed for either. */
export function contrastLines(page: ThemePage, scheme: "light" | "dark"): string[] {
  const c = page.contrast[scheme];
  const lines = [
    `Text on it ${ratio(c.onAccent)} · ${readable(c) ? "AA" : "below AA"}`,
    `As text on the background ${ratio(c.accentOnBackground)}`,
  ];
  for (const s of [scheme, scheme === "dark" ? "light" : "dark"] as const) {
    const other = page.contrast[s];
    if (!other.adjusted) continue;
    const done = s === "light" ? "Darkened" : "Lightened";
    lines.push(`${done} to ${page.tokens[s].accent} in ${s} mode, so it reads`);
  }
  return lines;
}
