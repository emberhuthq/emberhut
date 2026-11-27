import type { ImageSourcePropType } from "react-native";
import { materialTabIcons } from "./tabIconNames";
import type { TabParamList } from "./types";

// Android's tab icons, outlined and filled for the tab picked. The bar draws images, not font glyphs, so they ship as
// PNGs (assets/tabs, 24 dp at 1x, 2x and 3x, black on transparent: the bar tints them), rendered from the Material
// Icons font that @expo/vector-icons carries (Apache-2.0). Bundled, they're there on the first frame and can't fail.

// Metro bundles only what a literal require names.
const images: Record<string, ImageSourcePropType> = {
  "chat-bubble-outline": require("../../assets/tabs/chat-bubble-outline.png"),
  "chat-bubble": require("../../assets/tabs/chat-bubble.png"),
  "back-hand": require("../../assets/tabs/back-hand.png"),
  checklist: require("../../assets/tabs/checklist.png"),
  terminal: require("../../assets/tabs/terminal.png"),
  "more-horiz": require("../../assets/tabs/more-horiz.png"),
};

/** A tab's icon image on Android, plain or picked. */
export const androidTabIcon = (tab: keyof TabParamList, focused: boolean): ImageSourcePropType =>
  images[materialTabIcons[tab][focused ? 1 : 0]] ?? {};
