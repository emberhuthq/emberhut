import Ionicons from "@expo/vector-icons/Ionicons";
import type { ComponentProps } from "react";

export type IconName = ComponentProps<typeof Ionicons>["name"];

export interface IconProps {
  name: IconName;
  size?: number;
  color: string;
  /** Read out for an icon that says something on its own (a state); decorative icons go without. */
  accessibilityLabel?: string;
}

export function Icon({ name, size = 22, color, accessibilityLabel }: IconProps) {
  return (
    <Ionicons
      name={name}
      size={size}
      color={color}
      accessibilityLabel={accessibilityLabel}
      importantForAccessibility={accessibilityLabel ? "yes" : "no-hide-descendants"}
    />
  );
}
