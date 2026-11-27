import type { ReactNode } from "react";
import type { StyleProp, TextStyle } from "react-native";
import { useTheme } from "@/theme";
import { Text, type TextTone } from "./Text";

export interface SectionHeadingProps {
  children: ReactNode;
  tone?: TextTone;
  style?: StyleProp<TextStyle>;
}

/** A group's small uppercase title ("WHERE IT STANDS"), as iOS's grouped lists head their sections. */
export function SectionHeading({ children, tone = "secondary", style }: SectionHeadingProps) {
  const { spacing } = useTheme();
  return (
    <Text
      variant="footnote"
      weight="600"
      tone={tone}
      accessibilityRole="header"
      style={[{ textTransform: "uppercase", letterSpacing: 0.4, paddingHorizontal: spacing.xs }, style]}
    >
      {children}
    </Text>
  );
}
