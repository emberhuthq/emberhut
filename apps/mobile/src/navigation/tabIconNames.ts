import type { TabParamList } from "./types";

/** Each tab's Material icon names, plain and picked; each has its PNGs in assets/tabs. */
export const materialTabIcons: Record<keyof TabParamList, readonly [string, string]> = {
  Chat: ["chat-bubble-outline", "chat-bubble"],
  NeedsYou: ["back-hand", "back-hand"],
  Tasks: ["checklist", "checklist"],
  Sessions: ["terminal", "terminal"],
  More: ["more-horiz", "more-horiz"],
};
