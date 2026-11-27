import { useNavigation } from "@react-navigation/native";
import { Pressable, StyleSheet, View } from "react-native";
import type { AreaTree } from "@/api";
import { minTap, useTheme } from "@/theme";
import { Icon, Text } from "@/ui";
import { areaTotal } from "./laneModel";
import { AreaStateMark } from "./StateMark";

/** An area of a busy project as one row in the task list: its name, where it stands, what needs you; it opens the area. */
export function AreaRow({ area }: { area: AreaTree }) {
  const { colors, spacing } = useTheme();
  const navigation = useNavigation();
  const c = area.counts;
  const open = area.findings.partly + area.findings.notLanded + area.findings.unchecked;
  const line = [
    areaTotal(c) === 0 ? "No tasks yet" : `${c.done}/${areaTotal(c)} done`,
    c.needsYou > 0 ? `${c.needsYou} needs you` : null,
    open > 0 ? `${open} findings not landed` : null,
  ]
    .filter(Boolean)
    .join(" · ");
  return (
    <Pressable
      onPress={() => navigation.navigate("Area", { id: area.id })}
      accessibilityRole="button"
      accessibilityLabel={`${area.name} area, ${line}`}
      style={({ pressed }) => [
        styles.row,
        {
          gap: spacing.md,
          paddingHorizontal: spacing.lg,
          paddingVertical: spacing.sm + 2,
          backgroundColor: pressed ? colors.surfaceRaised : colors.surface,
        },
      ]}
    >
      <AreaStateMark state={area.state} />
      <View style={styles.text}>
        <Text numberOfLines={1}>{area.name}</Text>
        <Text variant="footnote" tone="secondary" numberOfLines={1}>
          {line}
        </Text>
      </View>
      <Icon name="layers-outline" size={18} color={colors.textTertiary} />
    </Pressable>
  );
}

const styles = StyleSheet.create({
  row: { minHeight: minTap, flexDirection: "row", alignItems: "center" },
  text: { flex: 1, gap: 2 },
});
