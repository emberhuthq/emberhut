import type { ReactNode } from "react";
import { Pressable, StyleSheet, View } from "react-native";
import { minTap, useTheme } from "@/theme";
import { Icon, type IconName } from "./Icon";
import { Text } from "./Text";

export interface ListRowProps {
  title: string;
  subtitle?: string;
  icon?: IconName;
  /** On the right: a count, a state, a switch. */
  trailing?: ReactNode;
  onPress?: () => void;
  /** Shows the chevron of a row that opens a page. */
  opens?: boolean;
  destructive?: boolean;
}

export function ListRow({ title, subtitle, icon, trailing, onPress, opens = onPress !== undefined, destructive = false }: ListRowProps) {
  const { colors, spacing } = useTheme();
  return (
    <Pressable
      onPress={onPress}
      disabled={!onPress}
      accessibilityRole={onPress ? "button" : undefined}
      style={({ pressed }) => [
        styles.row,
        {
          gap: spacing.md,
          paddingHorizontal: spacing.lg,
          paddingVertical: spacing.md,
          backgroundColor: pressed ? colors.surfaceRaised : colors.surface,
        },
      ]}
    >
      {icon && <Icon name={icon} color={destructive ? colors.danger : colors.textSecondary} />}
      <View style={styles.text}>
        <Text tone={destructive ? "danger" : "primary"} numberOfLines={2}>
          {title}
        </Text>
        {subtitle ? (
          <Text variant="footnote" tone="secondary" numberOfLines={2}>
            {subtitle}
          </Text>
        ) : null}
      </View>
      {trailing}
      {opens && <Icon name="chevron-forward" size={18} color={colors.textTertiary} />}
    </Pressable>
  );
}

const styles = StyleSheet.create({
  row: { minHeight: minTap, flexDirection: "row", alignItems: "center" },
  text: { flex: 1, gap: 2 },
});
