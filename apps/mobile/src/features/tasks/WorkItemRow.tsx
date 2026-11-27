import { useNavigation } from "@react-navigation/native";
import { useState } from "react";
import { Pressable, StyleSheet, View } from "react-native";
import { taskKey } from "@/api";
import { minTap, useTheme } from "@/theme";
import { Icon, Text } from "@/ui";
import { DeadlineChip } from "./DeadlineChip";
import { leaves, type WorkItem } from "./laneModel";
import { AreaStateMark } from "./StateMark";

/** A task in a project's lane: its state, its id and title, what you should do; its steps fold under it. */
export function WorkItemRow({ item, depth = 0 }: { item: WorkItem; depth?: number }) {
  const { colors, spacing } = useTheme();
  const navigation = useNavigation();
  const [open, setOpen] = useState(false);
  const count = item.kids.length > 0 ? leaves(item) : null;
  return (
    <View>
      <Pressable
        onPress={() => navigation.navigate("Task", { id: item.id })}
        accessibilityRole="button"
        accessibilityLabel={`${taskKey(item.id)} ${item.title}${item.action ? `, ${item.action}` : ""}`}
        style={({ pressed }) => [
          styles.row,
          {
            gap: spacing.sm,
            paddingLeft: spacing.lg + depth * spacing.lg,
            paddingRight: spacing.lg,
            paddingVertical: spacing.sm,
            backgroundColor: pressed ? colors.surfaceRaised : colors.surface,
          },
        ]}
      >
        <AreaStateMark state={item.state} size={16} />
        <Text variant="caption" tone="tertiary">
          {taskKey(item.id)}
        </Text>
        <View style={styles.text}>
          <Text variant="callout" numberOfLines={2} tone={item.state === "done" ? "secondary" : "primary"}>
            {item.title}
          </Text>
          {item.action ? (
            <Text variant="footnote" tone="needsYou" numberOfLines={1}>
              {item.action}
            </Text>
          ) : null}
        </View>
        {item.state !== "done" && <DeadlineChip day={item.day} hard={item.hard} />}
        {count ? (
          <Pressable
            onPress={() => setOpen(!open)}
            hitSlop={8}
            accessibilityRole="button"
            accessibilityLabel={open ? "Hide its steps" : `Show its steps, ${count.done} of ${count.total} done`}
            style={styles.count}
          >
            <Text variant="caption" tone="secondary">
              {count.done}/{count.total}
            </Text>
            <Icon name={open ? "chevron-up" : "chevron-down"} size={14} color={colors.textSecondary} />
          </Pressable>
        ) : null}
      </Pressable>
      {open && item.kids.map((kid) => <WorkItemRow key={kid.id} item={kid} depth={depth + 1} />)}
    </View>
  );
}

const styles = StyleSheet.create({
  row: { minHeight: minTap, flexDirection: "row", alignItems: "center" },
  text: { flex: 1, gap: 2 },
  count: { flexDirection: "row", alignItems: "center", gap: 2 },
});
