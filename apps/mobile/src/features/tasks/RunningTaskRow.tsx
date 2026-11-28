import { useNavigation } from "@react-navigation/native";
import { StyleSheet, View, Pressable } from "react-native";
import type { Issue } from "@/api";
import { LiveDot } from "@/features/sessions/StatusMark";
import { minTap, useTheme } from "@/theme";
import { ProgressBar, Text } from "@/ui";
import { liveLine } from "./model";
import { currentLine, isEmptyPlan, planFraction, planTicks } from "./plan";

/**
 * A task a run is going on, as the Tasks list shows it while the server keeps plans: a live dot, its title, the step it
 * is on (or what blocks it), its ticks and a thin bar of how far along. Queued: the dot is hollow and still.
 */
export function RunningTaskRow({ task, project }: { task: Issue; project?: string | null }) {
  const { colors, spacing } = useTheme();
  const navigation = useNavigation();
  const summary = task.plan;
  const current = summary ? currentLine(summary) : null;
  const blocked = current?.kind === "blocked";
  const fraction = summary ? planFraction(summary) : null;
  const queued = task.live?.state === "queued";
  const detail = current && current.kind !== "none" ? current.text : summary && isEmptyPlan(summary) ? "No plan yet" : liveLine(task);
  const ticks = summary && !isEmptyPlan(summary) ? planTicks(summary) : null;
  const label = [
    task.title,
    queued ? "queued" : "running",
    project,
    ticks && summary ? `${summary.done} of ${summary.total} steps done` : null,
    detail,
  ]
    .filter(Boolean)
    .join(", ");
  return (
    <Pressable
      onPress={() => navigation.navigate("Task", { id: task.id })}
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityHint="Opens the task"
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
      <View style={styles.mark}>
        {queued ? <View style={[styles.ring, { borderColor: colors.working }]} /> : <LiveDot color={colors.working} />}
      </View>
      <View style={styles.text}>
        <View style={[styles.line, { gap: spacing.sm }]}>
          <Text weight="600" numberOfLines={2} style={styles.fill}>
            {task.title}
          </Text>
          {ticks ? (
            <Text variant="footnote" weight="600" tone={blocked ? "danger" : "secondary"} style={styles.ticks}>
              {ticks}
            </Text>
          ) : null}
        </View>
        {detail ? (
          <Text variant="footnote" tone={blocked ? "danger" : "secondary"} numberOfLines={2}>
            {detail}
          </Text>
        ) : null}
        {project ? (
          <Text variant="caption" tone="tertiary" numberOfLines={1}>
            {project}
          </Text>
        ) : null}
        {fraction !== null ? (
          <View style={{ marginTop: spacing.xs }}>
            <ProgressBar value={fraction} color={blocked ? colors.danger : colors.working} />
          </View>
        ) : null}
      </View>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  row: { minHeight: minTap, flexDirection: "row", alignItems: "flex-start" },
  mark: { width: 18, height: 22, alignItems: "center", justifyContent: "center" },
  ring: { width: 10, height: 10, borderRadius: 5, borderWidth: 2 },
  text: { flex: 1, gap: 2 },
  line: { flexDirection: "row", alignItems: "flex-start" },
  fill: { flex: 1 },
  ticks: { fontVariant: ["tabular-nums"], paddingTop: 2 },
});
