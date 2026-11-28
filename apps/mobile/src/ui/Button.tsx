import { ActivityIndicator, Pressable, StyleSheet, View } from "react-native";
import { minTap, useTheme } from "@/theme";
import { Icon, type IconName } from "./Icon";
import { Text } from "./Text";

export type ButtonVariant = "primary" | "secondary" | "plain" | "destructive";

export interface ButtonProps {
  label: string;
  onPress: () => void;
  /** As the finger lands, before a text field it takes the focus from hears of it. */
  onPressIn?: () => void;
  variant?: ButtonVariant;
  icon?: IconName;
  disabled?: boolean;
  /** Shows a spinner and takes no taps. */
  loading?: boolean;
  /** As wide as its container. */
  block?: boolean;
  accessibilityLabel?: string;
}

export function Button({
  label,
  onPress,
  onPressIn,
  variant = "primary",
  icon,
  disabled = false,
  loading = false,
  block = false,
  accessibilityLabel,
}: ButtonProps) {
  const { colors, radius, spacing } = useTheme();
  const fill = variant === "primary" ? colors.primary : variant === "plain" ? "transparent" : colors.surfaceRaised;
  const inactive = disabled || loading;
  const toneColor = { primary: colors.onPrimary, secondary: colors.text, plain: colors.accent, destructive: colors.danger }[variant];
  return (
    <Pressable
      onPress={onPress}
      onPressIn={onPressIn}
      disabled={inactive}
      accessibilityRole="button"
      accessibilityLabel={accessibilityLabel ?? label}
      accessibilityState={{ disabled: inactive, busy: loading }}
      style={({ pressed }) => [
        styles.base,
        { backgroundColor: fill, borderRadius: radius.md, paddingHorizontal: spacing.lg, gap: spacing.sm },
        block && styles.block,
        (pressed || inactive) && { opacity: inactive ? 0.5 : 0.75 },
      ]}
    >
      {loading ? (
        <ActivityIndicator color={toneColor} />
      ) : (
        <View style={[styles.row, { gap: spacing.sm }]}>
          {icon && <Icon name={icon} size={18} color={toneColor} />}
          <Text variant="headline" style={{ color: toneColor }}>
            {label}
          </Text>
        </View>
      )}
    </Pressable>
  );
}

const styles = StyleSheet.create({
  base: { minHeight: minTap, alignItems: "center", justifyContent: "center" },
  block: { alignSelf: "stretch" },
  row: { flexDirection: "row", alignItems: "center" },
});
