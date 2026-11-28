import type { ThemeChange, ThemeChoice, ThemePage } from "@/api";

// Your ember's look, as the picker draws it: every preset the server lists as a card in its own colours, and the ember's
// tints. It is the theme Appearance changes too (one choice, on the server): a pick is a theme change, nothing else.

/** The preset a choice starts from: what the cards read as the default. */
export const DEFAULT_PRESET = "paper";

/** The ember's tint when none was chosen, or the server is older: the brand's own colours. */
export const DEFAULT_TINT = "ember";

/** A preset's card: its light colours as a small page, its dark ground and accent as a swatch, and whether it's in use. */
export interface LookCard {
  id: string;
  name: string;
  /** Read out as the default. */
  isDefault: boolean;
  /** In use: the preset chosen, with no custom colour over it. */
  selected: boolean;
  light: { paper: string; surface: string; ink: string; muted: string; accent: string; border: string };
  dark: { paper: string; accent: string; border: string };
}

/** The cards, one for each preset the server lists, in its order. */
export function lookCards(page: ThemePage): LookCard[] {
  const custom = page.choice.accent !== null;
  return page.presets.map(({ id, name, light, dark }) => ({
    id,
    name,
    isDefault: id === DEFAULT_PRESET,
    selected: !custom && page.choice.preset === id,
    light: {
      paper: light.background,
      surface: light.surface,
      ink: light.text,
      muted: light.muted,
      accent: light.accent,
      border: light.border,
    },
    dark: { paper: dark.background, accent: dark.accent, border: dark.border },
  }));
}

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

/** The tints the server lists, the one in use marked; none from a server before tints. */
export function tintChoices(page: ThemePage): TintChoice[] {
  const current = tintOf(page.choice);
  return (page.tints ?? []).map(({ id, name }) => ({ id, name, isDefault: id === DEFAULT_TINT, selected: id === current }));
}

/** Picking a tint: it alone changes. */
export const tintChange = (emberTint: string): ThemeChange => ({ emberTint });
