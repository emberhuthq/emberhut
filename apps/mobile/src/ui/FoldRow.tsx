import { Pressable, StyleSheet } from "react-native";
import { minTap, useTheme } from "@/theme";
import { Icon } from "./Icon";
import { Text } from "./Text";

/** A folded part of a page: its title and a chevron; a tap opens or folds what's under it. */
export function FoldRow({ title, open, onToggle }: { title: string; open: boolean; onToggle: () => void }) {
  const { colors, spacing } = useTheme();
  return (
    <Pressable
      onPress={onToggle}
      accessibilityRole="button"
      accessibilityLabel={title}
      accessibilityState={{ expanded: open }}
      style={({ pressed }) => [
        styles.row,
        { paddingHorizontal: spacing.lg, gap: spacing.md, backgroundColor: pressed ? colors.surfaceRaised : colors.surface },
      ]}
    >
      <Text style={styles.title}>{title}</Text>
      <Icon name={open ? "chevron-down" : "chevron-forward"} size={18} color={colors.textTertiary} />
    </Pressable>
  );
}

const styles = StyleSheet.create({
  row: { minHeight: minTap, flexDirection: "row", alignItems: "center" },
  title: { flex: 1 },
});
