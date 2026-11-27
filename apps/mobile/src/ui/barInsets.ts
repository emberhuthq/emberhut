import { HeaderHeightContext } from "@react-navigation/elements";
import { createContext, useContext } from "react";
import { BottomTabBarHeightContext } from "react-native-bottom-tabs";
import { useReduceTransparency } from "./Glass";

/** True under the tabs on iOS, where the native header and tab bar are see-through and the page runs under both. */
export const TranslucentBars = createContext(false);

/**
 * Whether what floats over a page (the bars, the message box) is see-through: iOS's translucent bars, unless Reduce
 * Transparency is on. Where it isn't, a floating piece needs a solid background of its own, or the page shows through it.
 */
export function useSeeThrough(): boolean {
  const translucent = useContext(TranslucentBars);
  const reduce = useReduceTransparency();
  return translucent && !reduce;
}

/**
 * How much of the page the see-through bars cover, top and bottom; zero where the bars are solid. A ScrollView doesn't
 * need it: iOS insets those itself (contentInsetAdjustmentBehavior). It's for fixed pages and the chat's inverted list.
 */
export function useBarInsets(): { top: number; bottom: number } {
  const translucent = useContext(TranslucentBars);
  const header = useContext(HeaderHeightContext) ?? 0;
  const tabBar = useContext(BottomTabBarHeightContext) ?? 0;
  return translucent ? { top: header, bottom: tabBar } : { top: 0, bottom: 0 };
}
