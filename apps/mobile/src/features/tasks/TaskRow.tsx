import { useNavigation } from "@react-navigation/native";
import { useState } from "react";
import { Pressable, StyleSheet, View } from "react-native";
import type { Issue } from "@/api";
import { actionsStore } from "@/services";
import { minTap, radius, useTheme } from "@/theme";
import { ActionSheet, Icon, ProgressBar, Text, type SheetAction } from "@/ui";
import { DeadlineChip } from "./DeadlineChip";
import { fraction, isMine, isOpen, taskLine } from "./model";
import { TaskStateMark } from "./StateMark";
import { useTaskPlans } from "./useTaskPlans";

export interface TaskRowProps {
  task: Issue;
  /** Its project's name, where the list doesn't already say it. */
  project?: string | null;
  /** Its open subtasks, shown under it on a tap of their count. */
  subtasks?: Issue[];
  indent?: boolean;
}

/**
 * One task in a list: where it stands, its title, a gray line when it says something, the deadline on the right. Your own
 * task with nothing under it ticks done from the list; a long press offers Not now and Archive.
 */
export function TaskRow({ task, project, subtasks = [], indent = false }: TaskRowProps) {
  const { colors, spacing } = useTheme();
  const navigation = useNavigation();
  const actions = actionsStore.getState();
  const [open, setOpen] = useState(false);
  const [menu, setMenu] = useState(false);
  const tickable = isMine(task) && task.status === "todo" && !task.sub;
  const line = taskLine(task, project, useTaskPlans());
  const progress = fraction(task);
  const sheet: SheetAction[] = [
    ...(tickable ? [{ label: "Mark done", icon: "checkmark-circle-outline" as const, onPress: () => actions.markDone(task.id) }] : []),
    ...(task.status === "todo" || task.status === "waiting"
      ? [{ label: "Not now: next week", icon: "moon-outline" as const, onPress: () => actions.later(task.id) }]
      : []),
    ...(isOpen(task) ? [{ label: "Archive", icon: "archive-outline" as const, onPress: () => actions.archive(task.id) }] : []),
    ...(task.status === "canceled"
      ? [{ label: "Unarchive", icon: "arrow-undo-outline" as const, onPress: () => actions.unarchive(task.id) }]
      : []),
  ];
  return (
    <View>
      <Pressable
        onPress={() => navigation.navigate("Task", { id: task.id })}
        onLongPress={sheet.length > 0 ? () => setMenu(true) : undefined}
        accessibilityRole="button"
        accessibilityLabel={`${task.title}${line ? `, ${line}` : ""}`}
        accessibilityHint="Opens the task"
        style={({ pressed }) => [
          styles.row,
          {
            gap: spacing.md,
            paddingLeft: indent ? spacing.xxl + spacing.sm : spacing.lg,
            paddingRight: spacing.lg,
            paddingVertical: spacing.sm + 2,
            backgroundColor: pressed ? colors.surfaceRaised : colors.surface,
          },
        ]}
      >
        {tickable ? (
          <Pressable
            onPress={() => actions.markDone(task.id)}
            hitSlop={10}
            accessibilityRole="button"
            accessibilityLabel={`Mark ${task.title} done`}
          >
            <Icon name="ellipse-outline" size={22} color={colors.textSecondary} />
          </Pressable>
        ) : (
          <TaskStateMark task={task} />
        )}
        <View style={styles.text}>
          <Text numberOfLines={2} tone={task.status === "done" ? "secondary" : "primary"}>
            {task.title}
          </Text>
          {line ? (
            <Text variant="footnote" tone="secondary" numberOfLines={1}>
              {line}
            </Text>
          ) : null}
          {progress !== null && (task.sub || task.status !== "done") ? (
            <View style={{ marginTop: spacing.xs }}>
              <ProgressBar value={progress} color={task.sub && task.sub.waiting.length > 0 ? colors.needsYou : undefined} />
            </View>
          ) : null}
        </View>
        <DeadlineChip day={task.status === "done" ? null : task.deadline} hard={task.hardDeadline ?? false} />
        {task.sub && subtasks.length > 0 ? (
          <Pressable
            onPress={() => setOpen(!open)}
            hitSlop={8}
            accessibilityRole="button"
            accessibilityLabel={open ? "Hide subtasks" : `Show ${subtasks.length} subtasks`}
            style={[styles.count, { backgroundColor: colors.surfaceRaised, paddingHorizontal: spacing.sm }]}
          >
            <Text variant="caption" weight="600" tone="secondary">
              {subtasks.length}
            </Text>
            <Icon name={open ? "chevron-up" : "chevron-down"} size={14} color={colors.textSecondary} />
          </Pressable>
        ) : null}
      </Pressable>
      {open && subtasks.map((kid) => <TaskRow key={kid.id} task={kid} indent />)}
      {menu && <ActionSheet visible title={task.title} actions={sheet} onClose={() => setMenu(false)} />}
    </View>
  );
}

const styles = StyleSheet.create({
  row: { minHeight: minTap, flexDirection: "row", alignItems: "center" },
  text: { flex: 1, gap: 2 },
  count: { flexDirection: "row", alignItems: "center", gap: 2, borderRadius: radius.pill, paddingVertical: 4 },
});
