import { Text as NativeText, type TextProps as NativeTextProps } from "react-native";
import { useTheme, type Palette, type TypeVariant } from "@/theme";

export type TextTone = "primary" | "secondary" | "tertiary" | "accent" | "danger" | "warning" | "needsYou" | "done" | "onPrimary";

export interface TextProps extends NativeTextProps {
  variant?: TypeVariant;
  tone?: TextTone;
  weight?: "400" | "500" | "600" | "700";
  align?: "left" | "center" | "right";
}

const toneColor: Record<TextTone, keyof Palette> = {
  primary: "text",
  secondary: "textSecondary",
  tertiary: "textTertiary",
  accent: "accent",
  danger: "danger",
  warning: "warning",
  needsYou: "needsYou",
  done: "done",
  onPrimary: "onPrimary",
};

export function Text({ variant = "body", tone = "primary", weight, align, style, ...rest }: TextProps) {
  const theme = useTheme();
  return (
    <NativeText
      style={[
        theme.type[variant],
        { color: theme.colors[toneColor[tone]] },
        weight && { fontWeight: weight },
        align && { textAlign: align },
        style,
      ]}
      {...rest}
    />
  );
}
