import { useNavigation } from "@react-navigation/native";
import type { ReactNode } from "react";
import { RefreshControl, SectionList, StyleSheet, View } from "react-native";
import type { EmberhutState, Issue, RunnerSession } from "@/api";
import { allSessions, isRunning } from "@/features/sessions/sessionList";
import { useTheme } from "@/theme";
import { Chip, ListRow, SectionHeading, Text, type ChipTone } from "@/ui";
import { tasklessSessions } from "./plan";
import { planLists, quietMinutes } from "./planView";
import { PlanTaskRow } from "./PlanTaskRow";
import { SessionItemRow } from "./SessionItemRow";
import { useMinute } from "./useMinute";

type Entry = { kind: "task"; task: Issue } | { kind: "session"; session: RunnerSession };

interface Group {
  key: string;
  title: string;
  tone: ChipTone;
  footer?: string;
  data: Entry[];
}

/**
 * The Tasks list while the server keeps plans, as a virtualized list (a couple of hundred tasks draw only what's on
 * screen): `header` (the search, a new task), a line to what needs you (its own tab lists it), Running (a blocked task
 * that runs among them, the sessions running with no task after), Blocked, Open, then the other sessions with no task,
 * so nothing that ran leaves the list; `footer` last (the archived tasks). `found`: a search's results, in place of the
 * groups.
 */
export function PlanTaskList({
  state,
  gone,
  header,
  found,
  footer,
  refreshing,
  onRefresh,
}: {
  state: EmberhutState;
  gone: ReadonlySet<number>;
  header: ReactNode;
  found: ReactNode | null;
  footer: ReactNode;
  refreshing: boolean;
  onRefresh: () => void;
}) {
  const { colors, radius, spacing } = useTheme();
  const navigation = useNavigation();
  // Moved on each minute: an unchanged state draws nothing again, and a quiet task is one whose state doesn't change.
  const now = useMinute();
  const lists = planLists(state, gone);
  const sessions = tasklessSessions(state, now);
  const every = allSessions(state);
  const projects = new Map((state.ui?.hideProjects ? [] : (state.projects ?? [])).map((p) => [p.id, p.name]));
  const tasks = (list: Issue[]): Entry[] => list.map((task) => ({ kind: "task", task }));
  const loose = (list: RunnerSession[]): Entry[] => list.map((session) => ({ kind: "session", session }));
  const groups: Group[] = found
    ? []
    : [
        {
          key: "running",
          title: "Running",
          tone: "muted" as const,
          data: [...tasks(lists.running), ...loose(sessions.filter((s) => isRunning(s.status)))],
        },
        { key: "blocked", title: "Blocked", tone: "danger" as const, data: tasks(lists.blocked) },
        { key: "open", title: "Open", tone: "muted" as const, data: tasks(lists.open) },
        {
          key: "sessions",
          title: "Sessions with no task",
          tone: "muted" as const,
          footer: "Every session, with filters: More › Sessions.",
          data: loose(sessions.filter((s) => !isRunning(s.status))),
        },
      ].filter((g) => g.data.length > 0);
  const empty = !found && lists.running.length + lists.blocked.length + lists.open.length === 0;
  return (
    <SectionList<Entry, Group>
      sections={groups}
      // Under the see-through bars iOS insets the list itself, so the large title shrinks as it scrolls.
      contentInsetAdjustmentBehavior="automatic"
      contentContainerStyle={{ padding: spacing.lg, paddingBottom: spacing.xl }}
      stickySectionHeadersEnabled={false}
      keyExtractor={(e) => (e.kind === "task" ? `t${e.task.id}` : `s${e.session.id}`)}
      refreshControl={<RefreshControl refreshing={refreshing} onRefresh={onRefresh} tintColor={colors.accent} />}
      ListHeaderComponent={
        <View style={{ gap: spacing.md, paddingBottom: groups.length > 0 ? 0 : spacing.md }}>
          {header}
          {!found && lists.needs > 0 ? (
            <View style={{ borderRadius: radius.lg, overflow: "hidden" }}>
              <ListRow
                icon="hand-left-outline"
                title={`${lists.needs} ${lists.needs === 1 ? "needs" : "need"} you`}
                onPress={() => navigation.navigate("Tabs", { screen: "NeedsYou" })}
              />
            </View>
          ) : null}
        </View>
      }
      renderSectionHeader={({ section }) => (
        <View
          style={[styles.header, { gap: spacing.sm, paddingHorizontal: spacing.xs, paddingTop: spacing.lg, paddingBottom: spacing.sm }]}
        >
          <SectionHeading tone={section.tone === "danger" ? "danger" : "secondary"} style={styles.fill}>
            {section.title}
          </SectionHeading>
          {section.key === "sessions" ? null : (
            <Chip label={String(section.data.length)} tone={section.tone} accessibilityLabel={`${section.data.length} ${section.title}`} />
          )}
        </View>
      )}
      renderItem={({ item, index, section }) => {
        // One rounded group per section, a hairline between its rows, as the app's Section draws it.
        const first = index === 0;
        const last = index === section.data.length - 1;
        return (
          <View
            style={{
              backgroundColor: colors.surface,
              overflow: "hidden",
              borderTopLeftRadius: first ? radius.lg : 0,
              borderTopRightRadius: first ? radius.lg : 0,
              borderBottomLeftRadius: last ? radius.lg : 0,
              borderBottomRightRadius: last ? radius.lg : 0,
            }}
          >
            {first ? null : <View style={{ height: StyleSheet.hairlineWidth, backgroundColor: colors.border, marginLeft: spacing.lg }} />}
            {item.kind === "task" ? (
              <PlanTaskRow task={item.task} project={projects.get(item.task.pid ?? -1)} quiet={quietMinutes(item.task, every, now)} />
            ) : (
              <SessionItemRow session={item.session} />
            )}
          </View>
        );
      }}
      renderSectionFooter={({ section }) =>
        section.footer ? (
          <Text variant="footnote" tone="secondary" style={{ paddingHorizontal: spacing.xs, paddingTop: spacing.sm }}>
            {section.footer}
          </Text>
        ) : null
      }
      ListFooterComponent={
        <View style={{ gap: spacing.md, paddingTop: spacing.lg }}>
          {found}
          {empty ? <Text tone="secondary">Nothing open.</Text> : null}
          {found ? null : footer}
        </View>
      }
    />
  );
}

const styles = StyleSheet.create({
  header: { flexDirection: "row", alignItems: "center" },
  fill: { flex: 1 },
});
