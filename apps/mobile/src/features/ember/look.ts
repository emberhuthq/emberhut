import type { Appearance, ThemeChange, ThemeChoice, ThemePage, ThemeTokens } from "@/api";

// Your ember's look, as the picker draws it: every preset the server lists as a card in its own colours, in the appearance
// chosen (both, split on a diagonal, for System), and the ember's tints. It is the theme Appearance changes too (one choice,
// on the server): a pick is a theme change, nothing else.

/** The preset a choice starts from: what the cards read as the default. */
export const DEFAULT_PRESET = "paper";

/** The ember's tint when none was chosen, or the server is older: the brand's own colours. */
export const DEFAULT_TINT = "ember";

/** The colours a preset's small page is drawn in, in one appearance. */
export interface PageColours {
  paper: string;
  sheet: string;
  ink: string;
  muted: string;
  accent: string;
}

/** A preset's card: its page in light and in dark, and whether it's in use. */
export interface LookCard {
  id: string;
  name: string;
  /** Read out as the default. */
  isDefault: boolean;
  /** In use: the preset chosen, with no custom colour over it. */
  selected: boolean;
  light: PageColours;
  dark: PageColours;
}

const pageColours = (t: ThemeTokens): PageColours => ({
  paper: t.background,
  sheet: t.surface,
  ink: t.text,
  muted: t.muted,
  accent: t.accent,
});

/** The cards, one for each preset the server lists, in its order. */
export function lookCards(page: ThemePage): LookCard[] {
  const custom = page.choice.accent !== null;
  return page.presets.map(({ id, name, light, dark }) => ({
    id,
    name,
    isDefault: id === DEFAULT_PRESET,
    selected: !custom && page.choice.preset === id,
    light: pageColours(light),
    dark: pageColours(dark),
  }));
}

/** Which of a preset's appearances its page shows: the one chosen, or both (split) for System and one this app doesn't know. */
export type Shown = "light" | "dark" | "both";

export const shownOf = (appearance: Appearance): Shown => (appearance === "light" ? "light" : appearance === "dark" ? "dark" : "both");

/** Picking a card: its preset, in its own colour (a custom colour gives way, as on Appearance). */
export const lookChange = (preset: string): ThemeChange => ({ preset, accent: null });

/** The tint the ember is drawn in: the choice's, or the default (a server before tints sends none). */
export const tintOf = (choice: Pick<ThemeChoice, "emberTint"> | null | undefined): string => choice?.emberTint ?? DEFAULT_TINT;

/** A tint to pick: its id and name, whether it's the default and whether it's in use. */
export interface TintChoice {
  id: string;
  name: string;
  isDefault: boolean;
  selected: boolean;
}

/**
 * The tints the server lists, the one in use marked (the default when the choice names one not listed, an early tint held
 * from before it was dropped, as the face itself draws); none from a server before tints.
 */
export function tintChoices(page: ThemePage): TintChoice[] {
  const tints = page.tints ?? [];
  const chosen = tintOf(page.choice);
  const current = tints.some((t) => t.id === chosen) ? chosen : DEFAULT_TINT;
  return tints.map(({ id, name }) => ({ id, name, isDefault: id === DEFAULT_TINT, selected: id === current }));
}

/** Picking a tint: it alone changes. */
export const tintChange = (emberTint: string): ThemeChange => ({ emberTint });
