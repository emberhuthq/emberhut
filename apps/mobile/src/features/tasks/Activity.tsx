import { useNavigation } from "@react-navigation/native";
import { useState } from "react";
import { Linking, Pressable, StyleSheet, View } from "react-native";
import { OWNER, type RunInfo, type RunnerSession, type TaskArtifact, type TaskMoment } from "@/api";
import { useMoveToTask, useSessionMenu, type MoveTo } from "@/features/sessions/SessionMenu";
import { StatusMark } from "@/features/sessions/StatusMark";
import { sessionStatusText, shortTitle } from "@/features/sessions/sessionList";
import { daysFromToday, weekday } from "@/format/time";
import { useTheme, type Palette } from "@/theme";
import { Chip, Icon, ListRow, Section, Segmented, Text, type IconName } from "@/ui";
import { taskSessions, type ActivityItem } from "./plan";
import { activityFeed, type FeedFilter, type FeedItem } from "./planView";
import { Spinner } from "./PlanMarks";
import { runTones } from "./TaskProgress";

const filters: { value: FeedFilter; label: string }[] = [
  { value: "all", label: "All" },
  { value: "turns", label: "Turns" },
  { value: "plan", label: "Plan" },
  { value: "files", label: "Files" },
];

/**
 * What happened on a task, under its plan and its ask: each turn of work in a line or two (what it was asked, the first
 * line of its answer, its STATUS) with its session one tap away, the history (ticks, skips, blocks, plan changes) and
 * what it produced, in one feed, newest first, with a filter. Then the task's sessions. Transcripts stay on the session's
 * screen.
 */
export function Activity({
  runs,
  sessions,
  history,
  artifacts,
}: {
  runs: RunInfo[];
  sessions: RunnerSession[];
  history: TaskMoment[];
  artifacts: TaskArtifact[];
}) {
  const { spacing } = useTheme();
  const move = useMoveToTask();
  const [filter, setFilter] = useState<FeedFilter>("all");
  const [all, setAll] = useState(false);
  const items = activityFeed(runs, sessions, history, artifacts, filter);
  const shown = all ? items : items.slice(0, 8);
  const turns = runs.length;
  // Nothing has happened yet: no feed to filter, only the sessions (if any).
  if (activityFeed(runs, sessions, history, artifacts).length === 0) return <TaskSessions sessions={sessions} move={move} />;
  return (
    <>
      <View style={{ gap: spacing.sm }}>
        <Section
          title="Activity"
          action={turns > 0 ? <Text variant="footnote" tone="secondary">{`${turns} ${turns === 1 ? "turn" : "turns"}`}</Text> : undefined}
        >
          <View key="filter" style={{ padding: spacing.sm }}>
            <Segmented options={filters} value={filter} onChange={setFilter} accessibilityLabel="Show in the activity" />
          </View>
          {shown.map((item, i) => (
            <FeedRow key={`${item.kind}-${i}`} item={item} move={move} />
          ))}
          {shown.length === 0 ? <ListRow key="none" title="Nothing here yet." /> : null}
          {items.length > shown.length ? (
            <ListRow key="all" title={`Show all ${items.length}`} onPress={() => setAll(true)} opens={false} />
          ) : null}
        </Section>
      </View>
      <TaskSessions sessions={sessions} move={move} />
    </>
  );
}

/** "14:05" today; "Mon" and the time before. */
function when(at: number): [string, string | null] {
  if (!at || at === Number.MAX_SAFE_INTEGER) return ["", null];
  const date = new Date(at);
  const time = date.toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit" });
  return [time, daysFromToday(date) === 0 ? null : weekday(date)];
}

function FeedRow({ item, move }: { item: FeedItem; move: MoveTo | undefined }) {
  const { colors, spacing } = useTheme();
  const [time, day] = when(item.at);
  return (
    <View style={[styles.row, { gap: spacing.md, padding: spacing.lg, backgroundColor: colors.surface }]}>
      <View style={styles.time} accessibilityElementsHidden importantForAccessibility="no-hide-descendants">
        <Text variant="caption" tone="tertiary">
          {time}
        </Text>
        {day ? (
          <Text variant="caption" tone="tertiary">
            {day}
          </Text>
        ) : null}
      </View>
      <View style={styles.fill}>
        {item.kind === "turn" ? (
          <Turn item={item.item} time={[day, time].filter(Boolean).join(" ")} move={move} />
        ) : item.kind === "moment" ? (
          <Moment moment={item.moment} time={[day, time].filter(Boolean).join(" ")} />
        ) : (
          <File artifact={item.artifact} />
        )}
      </View>
    </View>
  );
}

function Turn({ item, time, move }: { item: ActivityItem; time: string; move: MoveTo | undefined }) {
  const { spacing } = useTheme();
  const { run, status, session } = item;
  const said = item.said ?? (item.going ? (run.started ? "Working on it…" : "Not started yet") : null);
  return (
    <View style={{ gap: spacing.xs }}>
      <View
        accessible
        accessibilityLabel={[
          time,
          item.going ? "running now" : null,
          item.asked ? `Asked: ${item.asked}` : null,
          said,
          status?.text,
          item.failed ? `Turn ${item.failed}` : null,
        ]
          .filter(Boolean)
          .join(". ")}
        style={{ gap: spacing.xs }}
      >
        {item.asked ? (
          <Text variant="footnote" tone="secondary" numberOfLines={1}>
            Asked: {item.asked}
          </Text>
        ) : null}
        {said ? (
          <View style={[styles.line, { gap: spacing.sm }]}>
            {item.going ? <Spinner size={12} /> : null}
            <Text variant="callout" tone={item.said ? "primary" : "tertiary"} numberOfLines={3} style={styles.fill}>
              {said}
            </Text>
          </View>
        ) : null}
        {status ? (
          <Chip label={status.text} tone={status.kind === "working" ? "running" : runTones[status.kind]} />
        ) : item.failed ? (
          <Chip label={`Turn ${item.failed}`} tone="danger" />
        ) : null}
      </View>
      {session ? <TurnSession session={session} move={move} /> : null}
    </View>
  );
}

/** A turn's session, one tap away; a long press offers Move to task…. */
function TurnSession({ session, move }: { session: RunnerSession; move: MoveTo | undefined }) {
  const { colors, spacing } = useTheme();
  const navigation = useNavigation();
  const menu = useSessionMenu(session, move);
  return (
    <>
      <Pressable
        {...menu.press}
        onPress={() => navigation.navigate("Session", { id: session.id, title: shortTitle(session.title) })}
        hitSlop={8}
        accessibilityRole="button"
        accessibilityLabel={`Open session ${shortTitle(session.title)}`}
        style={[styles.line, { gap: spacing.xs, alignSelf: "flex-start" }]}
      >
        <Icon name="terminal-outline" size={13} color={colors.textTertiary} />
        <Text variant="footnote" tone="secondary" numberOfLines={1} style={styles.shrink}>
          {`${shortTitle(session.title)} on ${session.place}`}
        </Text>
        <Text variant="footnote" tone="accent" weight="600">
          {" · Open session ›"}
        </Text>
      </Pressable>
      {menu.sheet}
    </>
  );
}

const momentMarks: [RegExp, IconName, keyof Palette][] = [
  [/\bticked\b/i, "checkmark", "done"],
  [/\bskipped\b/i, "remove", "textTertiary"],
  [/\bblocked\b/i, "alert-circle-outline", "danger"],
  [/\b(plan|step)\b/i, "list-outline", "textSecondary"],
];

function Moment({ moment, time }: { moment: TaskMoment; time: string }) {
  const { colors, spacing } = useTheme();
  const [, icon, tint] = momentMarks.find(([test]) => test.test(moment.text)) ?? [null, "ellipse-outline", "textTertiary"];
  const who = moment.who === OWNER ? "You" : null;
  return (
    <View
      style={[styles.line, { gap: spacing.sm, alignItems: "flex-start" }]}
      accessible
      accessibilityLabel={`${time}. ${who ? `${who}: ` : ""}${moment.text}`}
    >
      <View style={{ paddingTop: 2 }}>
        <Icon name={icon} size={14} color={colors[tint]} />
      </View>
      <Text variant="footnote" tone="secondary" style={styles.fill}>
        {who ? <Text variant="footnote" weight="600">{`${who}: `}</Text> : null}
        {moment.text}
      </Text>
    </View>
  );
}

function File({ artifact }: { artifact: TaskArtifact }) {
  const { colors, spacing } = useTheme();
  const navigation = useNavigation();
  const web = /^https?:\/\//.test(artifact.url);
  const open = () => {
    if (web) void Linking.openURL(artifact.url).catch(() => undefined);
    else
      navigation.navigate("File", {
        path: artifact.url,
        name: artifact.name,
        kind: artifact.kind === "video" ? "video" : artifact.thumb ? "image" : "file",
      });
  };
  return (
    <Pressable
      onPress={open}
      accessibilityRole={web ? "link" : "button"}
      accessibilityLabel={`Open ${artifact.caption ?? artifact.name}`}
      style={[styles.line, { gap: spacing.sm }]}
    >
      <Icon name={web ? "link-outline" : "document-outline"} size={14} color={colors.accent} />
      <Text variant="footnote" tone="accent" numberOfLines={1} style={styles.shrink}>
        {artifact.caption ?? artifact.name}
      </Text>
    </Pressable>
  );
}

/** The task's sessions, running first: each opens its transcript; a long press offers Move to task…. */
function TaskSessions({ sessions, move }: { sessions: RunnerSession[]; move: MoveTo | undefined }) {
  return (
    <Section title="Sessions">
      {taskSessions(sessions).map((s) => (
        <TaskSessionRow key={s.id} session={s} move={move} />
      ))}
    </Section>
  );
}

function TaskSessionRow({ session: s, move }: { session: RunnerSession; move: MoveTo | undefined }) {
  const { colors, spacing } = useTheme();
  const navigation = useNavigation();
  const menu = useSessionMenu(s, move);
  return (
    <>
      <Pressable
        {...menu.press}
        onPress={() => navigation.navigate("Session", { id: s.id, title: shortTitle(s.title) })}
        accessibilityRole="button"
        accessibilityLabel={`Open session ${shortTitle(s.title)}, ${sessionStatusText(s.status)}`}
        style={({ pressed }) => [
          styles.line,
          {
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
      {menu.sheet}
    </>
  );
}

const styles = StyleSheet.create({
  row: { flexDirection: "row" },
  line: { flexDirection: "row", alignItems: "center" },
  time: { width: 40, alignItems: "flex-start" },
  fill: { flex: 1 },
  shrink: { flexShrink: 1 },
});
