import type { ReactNode } from "react";
import { Pressable, View, type StyleProp, type ViewStyle } from "react-native";
import { useTheme } from "@/theme";

export interface CardProps {
  children: ReactNode;
  onPress?: () => void;
  /** Tinted along its edge: what waits on you, a warning. */
  accent?: string;
  style?: StyleProp<ViewStyle>;
  accessibilityLabel?: string;
}

export function Card({ children, onPress, accent, style, accessibilityLabel }: CardProps) {
  const { colors, radius, spacing } = useTheme();
  const look: StyleProp<ViewStyle> = [
    {
      backgroundColor: colors.surface,
      borderRadius: radius.lg,
      padding: spacing.lg,
      gap: spacing.sm,
      borderWidth: 1,
      borderColor: accent ?? colors.border,
    },
    style,
  ];
  if (!onPress) return <View style={look}>{children}</View>;
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="button"
      accessibilityLabel={accessibilityLabel}
      style={({ pressed }) => [look, pressed && { opacity: 0.8 }]}
    >
      {children}
    </Pressable>
  );
}
