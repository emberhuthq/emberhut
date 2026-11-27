import { TextInput, type TextInputProps } from "react-native";
import { minTap, useTheme } from "@/theme";

export interface TextFieldProps extends TextInputProps {
  /** Grows with what you type, up to a few lines. */
  multiline?: boolean;
}

/** A text box in the app's look, in light and dark. */
export function TextField({ multiline = false, style, ...rest }: TextFieldProps) {
  const { colors, radius, spacing, type } = useTheme();
  return (
    <TextInput
      multiline={multiline}
      placeholderTextColor={colors.textTertiary}
      selectionColor={colors.accent}
      style={[
        type.body,
        {
          color: colors.text,
          // Explicit, as iOS reuses a text box's native view: one with spacing (the pairing code's) passes it on otherwise.
          letterSpacing: 0,
          backgroundColor: colors.surfaceRaised,
          borderRadius: radius.md,
          paddingHorizontal: spacing.md,
          paddingVertical: spacing.sm + 2,
          minHeight: minTap,
          maxHeight: multiline ? 160 : undefined,
          textAlignVertical: multiline ? "top" : "center",
        },
        style,
      ]}
      {...rest}
    />
  );
}
