import { createContext, useContext, useEffect, useMemo, type ReactNode } from "react";
import { Appearance, Platform, useColorScheme } from "react-native";
import type { Appearance as AppearanceChoice, ThemePage } from "@/api";
import { builtInTheme } from "./builtIn";
import { paletteFrom, type Palette } from "./colors";
import { radius, spacing, typeFor, type TypeScale } from "./tokens";

export interface Theme {
  scheme: "light" | "dark";
  colors: Palette;
  spacing: typeof spacing;
  radius: typeof radius;
  type: TypeScale;
}

const platformType = typeFor(Platform.OS);

function themeFor(scheme: "light" | "dark", tokens: ThemePage["tokens"]): Theme {
  return { scheme, colors: paletteFrom(tokens[scheme], scheme), spacing, radius, type: platformType };
}

const ThemeContext = createContext<Theme>(themeFor("light", builtInTheme.tokens));

/**
 * Emberhut's theme (its tokens, from the server or the built-in one) in the system's light or dark, unless you picked one;
 * native views (dialogs, the keyboard) follow too. A new choice shows at once, no restart.
 */
export function ThemeProvider({
  appearance,
  tokens,
  children,
}: {
  appearance: AppearanceChoice;
  tokens: ThemePage["tokens"];
  children: ReactNode;
}) {
  useEffect(() => {
    Appearance.setColorScheme(appearance === "system" ? "unspecified" : appearance);
  }, [appearance]);
  const system = useColorScheme();
  const scheme = appearance === "system" ? (system === "dark" ? "dark" : "light") : appearance;
  const theme = useMemo(() => themeFor(scheme, tokens), [scheme, tokens]);
  return <ThemeContext.Provider value={theme}>{children}</ThemeContext.Provider>;
}

export function useTheme(): Theme {
  return useContext(ThemeContext);
}
