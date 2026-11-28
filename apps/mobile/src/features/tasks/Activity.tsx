import { useNavigation } from "@react-navigation/native";
import { useState } from "react";
import { Pressable, StyleSheet, View } from "react-native";
import type { RunInfo, RunnerSession } from "@/api";
import { StatusMark } from "@/features/sessions/StatusMark";
import { sessionStatusText, shortTitle } from "@/features/sessions/sessionList";
import { useTheme } from "@/theme";
import { Chip, Icon, ListRow, Section, Text } from "@/ui";
import { activity, taskSessions, type ActivityItem } from "./plan";
import { runTones, runWhen } from "./TaskProgress";

/**
 * What happened on a task, under its plan: each turn of work in a line or two (what it was asked, the first line of its
 * answer, its STATUS), newest first, each with its session one tap away; then the task's sessions, each opening its
 * transcript. Transcripts themselves stay on the session's screen.
 */
export function Activity({ runs, sessions }: { runs: RunInfo[]; sessions: RunnerSession[] }) {
  const [all, setAll] = useState(false);
  const items = activity(runs, sessions);
  const shown = all ? items : items.slice(0, 5);
  return (
    <>
      <Section title="Activity" count={items.length}>
        {shown.map((item) => (
          <ActivityRow key={item.run.id} item={item} />
        ))}
        {items.length > shown.length ? (
          <ListRow key="all" title={`Show all ${items.length} turns`} onPress={() => setAll(true)} opens={false} />
        ) : null}
      </Section>
      <TaskSessions sessions={sessions} />
    </>
  );
}

function ActivityRow({ item }: { item: ActivityItem }) {
  const { colors, spacing } = useTheme();
  const navigation = useNavigation();
  const { run, status } = item;
  const mark = item.going ? colors.working : status ? colors[runTones[status.kind]] : item.failed ? colors.danger : colors.textTertiary;
  const said = item.said ?? (item.going ? (run.started ? "Working on it…" : "Not started yet") : null);
  const session = item.session;
  return (
    <View style={[styles.row, { gap: spacing.md, padding: spacing.lg, backgroundColor: colors.surface }]}>
      <View
        style={[styles.dot, { borderColor: mark, backgroundColor: item.going ? mark : "transparent" }]}
        accessibilityElementsHidden
        importantForAccessibility="no-hide-descendants"
      />
      <View style={{ flex: 1, gap: spacing.xs }}>
        <View
          accessible
          accessibilityLabel={[
            runWhen(run),
            item.asked ? `Asked: ${item.asked}` : null,
            said,
            status?.text,
            item.failed ? `Turn ${item.failed}` : null,
          ]
            .filter(Boolean)
            .join(". ")}
          style={{ gap: spacing.xs }}
        >
          <Text variant="caption" tone="secondary">
            {runWhen(run)}
          </Text>
          {item.asked ? (
            <Text variant="footnote" tone="secondary" numberOfLines={1}>
              Asked: {item.asked}
            </Text>
          ) : null}
          {said ? (
            <Text variant="callout" tone={item.said ? "primary" : "tertiary"} numberOfLines={2}>
              {said}
            </Text>
          ) : null}
          {status ? (
            <Chip label={status.text} tone={runTones[status.kind]} />
          ) : item.failed ? (
            <Chip label={`Turn ${item.failed}`} tone="danger" />
          ) : null}
        </View>
        {session ? (
          <Pressable
            onPress={() => navigation.navigate("Session", { id: session.id, title: shortTitle(session.title) })}
            hitSlop={8}
            accessibilityRole="button"
            accessibilityLabel={`Open session ${shortTitle(session.title)}`}
            style={[styles.open, { gap: spacing.xs }]}
          >
            <Icon name="terminal-outline" size={14} color={colors.accent} />
            <Text variant="footnote" tone="accent" weight="600">
              Open session
            </Text>
          </Pressable>
        ) : null}
      </View>
    </View>
  );
}

/** The task's sessions, running first: each opens its transcript. */
function TaskSessions({ sessions }: { sessions: RunnerSession[] }) {
  const { colors, spacing } = useTheme();
  const navigation = useNavigation();
  return (
    <Section title="Sessions">
      {taskSessions(sessions).map((s) => (
        <Pressable
          key={s.id}
          onPress={() => navigation.navigate("Session", { id: s.id, title: shortTitle(s.title) })}
          accessibilityRole="button"
          accessibilityLabel={`Open session ${shortTitle(s.title)}, ${sessionStatusText(s.status)}`}
          style={({ pressed }) => [
            styles.row,
            {
              alignItems: "center",
              gap: spacing.md,
              paddingHorizontal: spacing.lg,
              paddingVertical: spacing.md,
              backgroundColor: pressed ? colors.surfaceRaised : colors.surface,
            },
          ]}
        >
          <StatusMark status={s.status} />
          <View style={{ flex: 1, gap: 2 }}>
            <Text numberOfLines={1}>{shortTitle(s.title)}</Text>
            <Text variant="footnote" tone="secondary" numberOfLines={1}>
              {[s.doing ?? sessionStatusText(s.status), s.place].filter(Boolean).join(" · ")}
            </Text>
          </View>
          <Text variant="footnote" tone="accent" weight="600">
            Open session
          </Text>
          <Icon name="chevron-forward" size={16} color={colors.textTertiary} />
        </Pressable>
      ))}
    </Section>
  );
}

const styles = StyleSheet.create({
  row: { flexDirection: "row" },
  dot: { width: 10, height: 10, marginTop: 5, borderRadius: 5, borderWidth: 2 },
  open: { flexDirection: "row", alignItems: "center", alignSelf: "flex-start", paddingTop: 2 },
});
