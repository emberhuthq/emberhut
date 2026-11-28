import type { NativeStackHeaderItemButton } from "@react-navigation/native-stack";

type SFSymbol = Extract<NonNullable<NativeStackHeaderItemButton["icon"]>, { type: "sfSymbol" }>["name"];

/** A button on the right of a native header, with a count on it when there is one. */
export interface HeaderButton {
  /** The same across renders: iOS 26 matches items by it from one screen to the next. */
  id: string;
  symbol: SFSymbol;
  /** Read by VoiceOver, and shown in the overflow menu when the bar runs out of room. */
  label: string;
  onPress: () => void;
  /** Running sessions, approvals waiting: none at 0, 99+ past 99. */
  count?: number;
  badge?: { color: string; background: string };
}

/**
 * The header's buttons as UIKit's own bar button items (iOS): the bar sizes, centres and spaces them like every other
 * header control, gives each its 44 pt target inside the bar, follows Dynamic Type, and draws the count as the item's
 * UIBarButtonItemBadge on its top-right corner, inside its bounds. A React view in headerRight got none of that: its
 * margins and an absolutely placed badge set the glass circle's size, so the circle sat off the title's centre, hung
 * below the bar and clipped the badge.
 */
export function headerItems(buttons: readonly HeaderButton[], tint: string): NativeStackHeaderItemButton[] {
  return buttons.map(({ id, symbol, label, onPress, count = 0, badge }) => ({
    type: "button",
    identifier: id,
    icon: { type: "sfSymbol", name: symbol },
    label,
    accessibilityLabel: label,
    tintColor: tint,
    onPress,
    ...(count > 0 && badge
      ? { badge: { value: count > 99 ? "99+" : String(count), style: { color: badge.color, backgroundColor: badge.background } } }
      : {}),
  }));
}
