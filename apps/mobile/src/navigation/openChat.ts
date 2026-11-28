import type { TabParamList } from "./types";

/**
 * Where a room, or a draft for one, opens: the Chat tab under the screens on top of it (the rooms sheet, a task), as
 * popTo's arguments. navigate("Tabs") would push a second tab bar above them, and from the rooms sheet inside it.
 */
export const chatTarget = (params: NonNullable<TabParamList["Chat"]>) => ["Tabs", { screen: "Chat", params }] as const;

/** The navigation of a screen on the root stack, as far as opening the chat needs it. */
export interface PopsTo {
  popTo(name: "Tabs", params: { screen: "Chat"; params: NonNullable<TabParamList["Chat"]> }): void;
}

/** Shows the chat on that room (or with that draft) in place: the screens over the tabs close, and the tabs stay. */
export function openChat(navigation: PopsTo, params: NonNullable<TabParamList["Chat"]>): void {
  navigation.popTo(...chatTarget(params));
}
