import { useNavigation } from "@react-navigation/native";
import type { ReactNode } from "react";
import { Pressable, StyleSheet, View } from "react-native";
import type { Issue, Plan, RunnerSession } from "@/api";
import { isRunning, shortTitle } from "@/features/sessions/sessionList";
import { parseInstant, relative } from "@/format/time";
import { useTheme } from "@/theme";
import { Icon, Text, type TextTone } from "@/ui";
import { statusName } from "./model";
import { isPlanless, planFraction } from "./plan";
import { countLine, currentPath, isBlocked, nextSteps, planSegments, quietMinutes, quietText } from "./planView";
import { SegmentBar, TaskLead } from "./PlanMarks";
import { useMinute } from "./useMinute";

/**
 * The first screen of a task's page while the server keeps plans, so it answers at a glance what's done, what's
 * happening now, what's next and what needs you: its state (and how long it has been quiet), Now, what needs you or what
 * blocks it (a tap goes down to the ask), Next, the estimate, its session one tap away, then the count and the bar.
 * `watched`: every session that may be its own, for how long it has been quiet.
 */
export function PlanHeader({
  task,
  plan,
  sessions,
  watched,
  onAsk,
}: {
  task: Issue;
  plan: Plan;
  sessions: RunnerSession[];
  watched: readonly RunnerSession[];
  onAsk?: () => void;
}) {
  const { colors, spacing } = useTheme();
  const navigation = useNavigation();
  const quiet = quietMinutes(task, watched, useMinute());
  const summary = plan.summary;
  const empty = isPlanless(plan);
  const now = empty ? null : currentPath(plan);
  const next = nextSteps(plan, 1)[0]?.title ?? null;
  const asked = task.status === "waiting" ? (task.action ?? task.decision?.question ?? null) : null;
  const blocker = isBlocked(task) ? (summary.blocker ?? summary.current?.title ?? null) : null;
  const updated = parseInstant(summary.updated);
  const thread = task.live?.run?.thread;
  const session = sessions.find((s) => s.id === thread) ?? sessions.find((s) => isRunning(s.status)) ?? sessions[0];
  const sessionAt = parseInstant(session?.updatedAt);
  const fraction = planFraction(summary);
  return (
    <View style={{ gap: spacing.sm }}>
      <Fact label="State">
        <View style={[styles.line, { gap: spacing.xs }]}>
          <TaskLead task={task} />
          <Text variant="callout" style={styles.shrink}>
            {statusName(task)}
          </Text>
          {quiet != null ? (
            <View style={[styles.line, { gap: 3, marginLeft: spacing.sm }]} accessible accessibilityLabel={`quiet for ${quietText(quiet)}`}>
              <Icon name="time-outline" size={14} color={colors.textSecondary} />
              <Text variant="footnote" tone="secondary">{`quiet ${quietText(quiet)}`}</Text>
            </View>
          ) : null}
        </View>
      </Fact>
      <Fact
        label="Now"
        value={now ?? (empty ? "No plan yet" : "Nothing being worked on")}
        tone={now ? "primary" : "tertiary"}
        weight={now ? "600" : undefined}
      />
      {asked ? (
        <Fact label="Needs you" value={asked} tone="needsYou" weight="600" onPress={onAsk} />
      ) : blocker ? (
        <Fact label="Blocked" value={blocker} tone="danger" onPress={onAsk} />
      ) : null}
      {next ? <Fact label="Next" value={next} tone="secondary" /> : null}
      {summary.eta && summary.done < summary.total ? (
        <Fact label="ETA" value={[summary.eta, updated ? `updated ${relative(updated)}` : null].filter(Boolean).join(" · ")} />
      ) : null}
      {session ? (
        <Pressable
          onPress={() => navigation.navigate("Session", { id: session.id, title: shortTitle(session.title) })}
          accessibilityRole="button"
          accessibilityLabel={`Open session ${shortTitle(session.title)}`}
          hitSlop={6}
          style={[styles.line, { gap: spacing.xs }]}
        >
          <Icon name="terminal-outline" size={14} color={colors.textTertiary} />
          <Text variant="footnote" tone="secondary" numberOfLines={1} style={styles.shrink}>
            {[shortTitle(session.title), sessionAt ? `updated ${relative(sessionAt)}` : null].filter(Boolean).join(" · ")}
          </Text>
          <Text variant="footnote" tone="accent" weight="600">
            {" · Open session ›"}
          </Text>
        </Pressable>
      ) : null}
      {!empty && fraction !== null ? (
        <View style={{ gap: spacing.xs, paddingTop: spacing.xs }}>
          <View style={styles.line}>
            <Text variant="callout" weight="600" style={styles.fill}>
              {countLine(plan)}
            </Text>
            <Text variant="footnote" tone="secondary">{`${Math.round(fraction * 100)}%`}</Text>
          </View>
          <SegmentBar segments={planSegments(plan)} fraction={fraction} blocked={blocker != null} height={6} label={countLine(plan)} />
        </View>
      ) : null}
    </View>
  );
}

/** One line of the header: a label on the left, what it says on the right. */
function Fact({
  label,
  value,
  tone = "primary",
  weight,
  onPress,
  children,
}: {
  label: string;
  value?: string;
  tone?: TextTone;
  weight?: "600";
  onPress?: () => void;
  children?: ReactNode;
}) {
  const { spacing } = useTheme();
  const body = children ?? (
    <Text variant="callout" tone={tone} weight={weight} numberOfLines={2} style={styles.fill}>
      {value}
      {onPress ? " ↓" : ""}
    </Text>
  );
  const style = [styles.line, { gap: spacing.md, alignItems: "flex-start" as const }];
  const head = (
    <Text variant="callout" tone="secondary" style={styles.label}>
      {label}
    </Text>
  );
  if (onPress)
    return (
      <Pressable onPress={onPress} accessibilityRole="button" accessibilityLabel={`${label}: ${value ?? ""}. Go to it`} style={style}>
        {head}
        {body}
      </Pressable>
    );
  return (
    <View
      style={style}
      accessible={children === undefined}
      accessibilityLabel={children === undefined ? `${label}: ${value ?? ""}` : undefined}
    >
      {head}
      <View style={styles.fill}>{body}</View>
    </View>
  );
}

const styles = StyleSheet.create({
  line: { flexDirection: "row", alignItems: "center" },
  label: { width: 84 },
  fill: { flex: 1 },
  shrink: { flexShrink: 1 },
});
