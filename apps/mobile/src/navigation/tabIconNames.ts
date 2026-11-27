import type { TabParamList } from "./types";

/** The tabs with a Material icon: every one but your ember's, whose icon is the ember itself (emberTab.ts). */
export type IconTab = Exclude<keyof TabParamList, "Chat">;

/** Each tab's Material icon names, plain and picked; each has its PNGs in assets/tabs. */
export const materialTabIcons: Record<IconTab, readonly [string, string]> = {
  NeedsYou: ["back-hand", "back-hand"],
  Tasks: ["checklist", "checklist"],
  Sessions: ["terminal", "terminal"],
  More: ["more-horiz", "more-horiz"],
};
