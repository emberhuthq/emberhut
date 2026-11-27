import { DarkTheme, DefaultTheme, type Theme as NavigationTheme } from "@react-navigation/native";
import type { Theme } from "@/theme";

/** React Navigation's colors from Emberhut's theme. */
export function navigationTheme(theme: Theme): NavigationTheme {
  const base = theme.scheme === "dark" ? DarkTheme : DefaultTheme;
  const { colors } = theme;
  return {
    ...base,
    colors: {
      ...base.colors,
      primary: colors.accent,
      background: colors.background,
      card: colors.surface,
      text: colors.text,
      border: colors.border,
      notification: colors.needsYou,
    },
  };
}
