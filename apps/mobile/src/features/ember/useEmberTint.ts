import { usePrefs } from "@/state/hooks";
import { tintOf } from "./look";

/** The tint the ember is drawn in now: the theme's, as this phone last had it (the default before Emberhut answers). */
export const useEmberTint = (): string => usePrefs((s) => tintOf(s.themeCache?.choice));
