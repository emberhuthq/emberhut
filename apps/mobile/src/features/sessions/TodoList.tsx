import { useState } from "react";
import { Pressable, StyleSheet, View } from "react-native";
import type { TodoItem } from "@/api";
import { useTheme } from "@/theme";
import { Icon, Text } from "@/ui";

/** Its to-do list: how far along, then each item ticked, in progress or waiting. */
export function TodoList({ todos, live = false }: { todos: TodoItem[]; live?: boolean }) {
  const { colors, spacing } = useTheme();
  const [open, setOpen] = useState<boolean | null>(null);
  const done = todos.filter((t) => t.status === "completed").length;
  const shown = open ?? (live && todos.length <= 5);
  const now = todos.find((t) => t.status === "in_progress");
  return (
    <View style={{ gap: spacing.xs + 2 }}>
      <Pressable
        onPress={() => setOpen(!shown)}
        accessibilityRole="button"
        accessibilityLabel={`Tasks, ${done} of ${todos.length} done`}
        accessibilityState={{ expanded: shown }}
        style={[styles.row, { gap: spacing.sm, minHeight: 28 }]}
      >
        <Icon name="list-outline" size={14} color={colors.textSecondary} />
        <Text variant="caption" weight="600">
          Tasks
        </Text>
        <View style={[styles.track, { backgroundColor: colors.surfaceRaised }]}>
          <View
            style={{
              width: `${(done / Math.max(1, todos.length)) * 100}%`,
              height: "100%",
              borderRadius: 3,
              backgroundColor: done === todos.length ? colors.done : colors.accent,
            }}
          />
        </View>
        <Text variant="caption" tone="secondary">
          {done}/{todos.length}
        </Text>
        {!shown && now && (
          <Text variant="caption" tone="secondary" numberOfLines={1} style={styles.shrink}>
            {now.activeForm ?? now.content}
          </Text>
        )}
        <View style={styles.fill} />
        <Icon name={shown ? "chevron-down" : "chevron-forward"} size={12} color={colors.textTertiary} />
      </Pressable>
      {shown &&
        todos.map((t, i) => (
          <View key={t.id ?? i} style={[styles.row, { gap: spacing.sm }]}>
            <Icon
              name={t.status === "completed" ? "checkmark-circle" : t.status === "in_progress" ? "ellipse" : "ellipse-outline"}
              size={13}
              color={t.status === "completed" ? colors.done : t.status === "in_progress" ? colors.accent : colors.textTertiary}
            />
            <Text
              variant="caption"
              weight={t.status === "in_progress" ? "600" : "400"}
              tone={t.status === "completed" ? "secondary" : "primary"}
              numberOfLines={2}
              style={[styles.fill, t.status === "completed" && styles.struck]}
            >
              {t.content}
            </Text>
          </View>
        ))}
    </View>
  );
}

const styles = StyleSheet.create({
  row: { flexDirection: "row", alignItems: "center" },
  fill: { flex: 1 },
  shrink: { flexShrink: 1 },
  struck: { textDecorationLine: "line-through" },
  track: { width: 60, height: 6, borderRadius: 3, overflow: "hidden" },
});
