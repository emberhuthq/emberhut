import { useNavigation } from "@react-navigation/native";
import { StyleSheet, View, Pressable } from "react-native";
import type { Issue } from "@/api";
import { minTap, useTheme } from "@/theme";
import { Icon, Text, type TextTone } from "@/ui";
import { currentLine, isEmptyPlan, isRunningTask, planFraction, planTicks } from "./plan";
import { isBlocked, quietText, summarySegments } from "./planView";
import { SegmentBar, TaskLead } from "./PlanMarks";

/** A task's one line under its title while the server keeps plans, and how it reads. */
function detailOf(task: Issue): { text: string; tone: TextTone; weight?: "600" } {
  const summary = task.plan;
  if (task.status === "waiting")
    return { text: `Waiting on you: ${task.action ?? task.decision?.question ?? "your answer"}`, tone: "needsYou", weight: "600" };
  if (summary && isBlocked(task)) {
    const line = currentLine(summary);
    return { text: line.kind === "blocked" ? line.text : `Blocked: ${summary.blocker ?? ""}`, tone: "danger" };
  }
  if (!summary || isEmptyPlan(summary))
    return {
      text: task.live?.state === "queued" ? "Queued · No plan yet" : task.live?.state === "running" ? "No plan yet" : "To do · No plan yet",
      tone: "tertiary",
    };
  const line = currentLine(summary);
  if (task.live?.state === "queued") return { text: `Queued · ${line.text}`, tone: "secondary" };
  return { text: line.kind === "none" ? "Not started" : line.text, tone: "secondary" };
}

/**
 * A task in the Tasks list while the server keeps plans: its mark (the spinner while a run goes, the red mark when
 * blocked, the ember dot when it waits on you), the title whole, the step it is on or what blocks it with how long it has
 * been quiet, then the plan's bar with its ticks. `quiet`: minutes with neither its plan nor its session moving.
 */
export function PlanTaskRow({ task, project, quiet = null }: { task: Issue; project?: string | null; quiet?: number | null }) {
  const { colors, spacing } = useTheme();
  const navigation = useNavigation();
  const summary = task.plan;
  const detail = detailOf(task);
  const planned = summary && !isEmptyPlan(summary) ? summary : null;
  // The bar once something is done or a run is going; before that the ticks sit on the line above, quietly.
  const bar = planned != null && (planned.done > 0 || isRunningTask(task));
  const state =
    task.status === "waiting"
      ? "waiting on you"
      : task.live?.state === "running"
        ? "running"
        : task.live?.state === "queued"
          ? "queued"
          : "open";
  const label = [
    task.title,
    state,
    detail.text,
    quiet != null ? `quiet for ${quietText(quiet)}` : null,
    planned ? `${planned.done} of ${planned.total} steps done` : null,
    project,
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
      <TaskLead task={task} />
      <View style={styles.text}>
        <Text numberOfLines={2}>{task.title}</Text>
        <View style={[styles.line, { gap: spacing.sm }]}>
          <Text variant="footnote" tone={detail.tone} weight={detail.weight} numberOfLines={1} style={styles.fill}>
            {detail.text}
          </Text>
          {planned && !bar ? (
            <Text variant="footnote" tone="secondary" style={styles.ticks}>
              {planTicks(planned)}
            </Text>
          ) : null}
          {quiet != null ? (
            <View style={[styles.line, { gap: 3 }]}>
              <Icon name="time-outline" size={13} color={colors.textSecondary} />
              <Text variant="footnote" tone="secondary">{`quiet ${quietText(quiet)}`}</Text>
            </View>
          ) : null}
        </View>
        {project ? (
          <Text variant="caption" tone="tertiary" numberOfLines={1}>
            {project}
          </Text>
        ) : null}
        {planned && bar ? (
          <View style={[styles.line, { gap: spacing.sm, marginTop: spacing.xs }]}>
            <SegmentBar
              segments={summarySegments(planned)}
              fraction={planFraction(planned) ?? 0}
              blocked={isBlocked(task)}
              label={`${planned.done} of ${planned.total} steps done`}
            />
            <Text variant="footnote" tone="secondary" style={styles.ticks}>
              {planTicks(planned)}
            </Text>
          </View>
        ) : null}
      </View>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  row: { minHeight: minTap, flexDirection: "row", alignItems: "flex-start" },
  text: { flex: 1, gap: 2 },
  line: { flexDirection: "row", alignItems: "center" },
  fill: { flex: 1 },
  ticks: { fontVariant: ["tabular-nums"], minWidth: 28, textAlign: "right" },
});
