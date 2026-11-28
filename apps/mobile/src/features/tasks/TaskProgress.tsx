import { useNavigation } from "@react-navigation/native";
import { useState } from "react";
import { Linking, Pressable, ScrollView, StyleSheet, View } from "react-native";
import { TASK_KEY_PREFIX, type Issue, type RunInfo, type TaskArtifact, type TaskLink } from "@/api";
import { ServerImage } from "@/features/files/ServerImage";
import { parseInstant, relative, span, weekdayClock } from "@/format/time";
import { useTheme, type Palette } from "@/theme";
import { Chip, Collapsible, Icon, ListRow, Section, Text, type ChipTone, type IconName } from "@/ui";
import { liveLine, stateTone, statusName, taskState, type StateTone } from "./model";
import { runStatus, type RunStatusKind } from "./taskPage";
import { TaskStateMark } from "./StateMark";

/** The task key a check-in's reason starts with, and the marks around it. */
const leadingKey = new RegExp(`^\\W*${TASK_KEY_PREFIX}\\d+\\W*`);

const chipTones: Record<StateTone, ChipTone> = { needsYou: "needsYou", working: "working", done: "done", danger: "danger", muted: "muted" };

/** Right now: its state and since when, the ETA, what you should do or what runs, where, and when the last run ended. */
export function LiveCard({ task, runs }: { task: Issue; runs: RunInfo[] }) {
  const { spacing } = useTheme();
  const since = parseInstant(task.live?.since);
  const open = runs.find((r) => r.ended == null);
  const last = runs.find((r) => r.ended != null);
  const lastEnd = parseInstant(last?.ended);
  const lastStart = parseInstant(last?.started);
  const run = task.live?.run;
  const state = taskState(task);
  const facts: [string, string][] = [];
  const where = liveLine(task);
  if (where) facts.push([state === "scheduled" ? "When" : "Where", where]);
  const waitsOn = task.live?.watch?.title ?? task.live?.checkin?.why.replace(leadingKey, "");
  if (state === "scheduled" && waitsOn) facts.push(["Waits on", waitsOn]);
  if (open?.asked) facts.push(["Asked", open.asked]);
  if (lastEnd) facts.push(["Last run", `Ended ${relative(lastEnd)}${lastStart ? `, after ${span(lastStart, lastEnd)}` : ""}`]);
  return (
    <View style={{ gap: spacing.sm }}>
      <View style={styles.line}>
        <Chip label={[statusName(task), since ? relative(since) : null].filter(Boolean).join(" · ")} tone={chipTones[stateTone(task)]} />
        <View style={{ flex: 1 }} />
        {task.eta && task.status !== "done" && task.status !== "canceled" ? (
          <Text variant="footnote" tone="secondary">
            ETA {task.eta}
          </Text>
        ) : null}
      </View>
      {task.status === "waiting" && task.action ? (
        <Text variant="headline">{task.action}</Text>
      ) : run && (state === "running" || state === "queued") && (run.worker || run.title) ? (
        <Text variant="headline">
          {run.worker ? "Emberhut's worker is on it" : `${state === "queued" ? "Waiting to start on" : "Working on"} “${run.title}”`}
        </Text>
      ) : null}
      {facts.map(([label, value]) => (
        <View key={label} style={[styles.line, { gap: spacing.md, alignItems: "flex-start" }]}>
          <Text variant="footnote" tone="secondary" style={styles.label}>
            {label}
          </Text>
          <Text variant="footnote" style={{ flex: 1 }} numberOfLines={label === "When" ? 4 : 2}>
            {value}
          </Text>
        </View>
      ))}
    </View>
  );
}

/** A run's STATUS as a chip and a mark: done, working, blocked, needs you. */
export const runTones: Record<RunStatusKind, ChipTone & keyof Palette> = {
  done: "done",
  working: "working",
  blocked: "danger",
  needs: "needsYou",
};

/** Every run, newest first: when and how long, what it was asked, its answer, its STATUS. */
export function RunList({ runs }: { runs: RunInfo[] }) {
  const [all, setAll] = useState(false);
  const shown = all ? runs : runs.slice(0, 5);
  return (
    <Section title="Runs" count={runs.length}>
      {shown.map((run) => (
        <RunRow key={run.id} run={run} />
      ))}
      {runs.length > shown.length ? (
        <ListRow key="all" title={`Show all ${runs.length} runs`} onPress={() => setAll(true)} opens={false} />
      ) : null}
    </Section>
  );
}

function RunRow({ run }: { run: RunInfo }) {
  const { colors, spacing } = useTheme();
  const status = runStatus(run.status);
  const going = run.ended == null;
  return (
    <View style={{ flexDirection: "row", gap: spacing.md, padding: spacing.lg, backgroundColor: colors.surface }}>
      <View
        style={{
          width: 10,
          height: 10,
          marginTop: 5,
          borderRadius: 5,
          borderWidth: 2,
          borderColor: going ? colors.warning : status ? colors[runTones[status.kind]] : colors.textTertiary,
          backgroundColor: going ? colors.warning : "transparent",
        }}
      />
      <View style={{ flex: 1, gap: spacing.xs }}>
        <Text variant="caption" tone="secondary">
          {runWhen(run)}
        </Text>
        {run.asked ? (
          <Text variant="footnote" tone="secondary" numberOfLines={1}>
            Asked: {run.asked}
          </Text>
        ) : null}
        {run.answer?.trim() ? (
          <Collapsible text={run.answer} lines={2} variant="callout" />
        ) : going ? (
          <Text variant="callout" tone="tertiary">
            {run.started ? "Working on it…" : "Not started yet"}
          </Text>
        ) : null}
        {status ? (
          <Chip label={status.text} tone={runTones[status.kind]} />
        ) : run.outcome && run.outcome !== "completed" ? (
          <Chip label={`Turn ${run.outcome}`} tone="danger" />
        ) : null}
      </View>
    </View>
  );
}

/** When a run was: "Mon 14:05 → Mon 14:40 · 35 min", "Started Mon 14:05 · 3 min so far", "Sent Mon 14:05 · queued". */
export function runWhen(run: RunInfo): string {
  const start = parseInstant(run.started);
  const end = parseInstant(run.ended);
  if (start && end) return `${weekdayClock(start)} → ${weekdayClock(end)} · ${span(start, end)}`;
  if (start) return `Started ${weekdayClock(start)} · ${span(start, new Date())} so far`;
  if (end) return weekdayClock(end);
  const sent = parseInstant(run.requested);
  return sent ? `Sent ${weekdayClock(sent)} · queued` : "Queued";
}

/** What it produced: pictures and videos as tiles, then documents and links; each opens full screen or in its app. */
export function Results({ items, title = "Results" }: { items: TaskArtifact[]; title?: string }) {
  const { radius, spacing } = useTheme();
  const navigation = useNavigation();
  const media = items.filter((a) => a.thumb);
  const rest = items.filter((a) => !a.thumb && !a.x);
  const open = (a: TaskArtifact) => {
    if (a.kind === "link" || /^https?:\/\//.test(a.url)) void Linking.openURL(a.url).catch(() => undefined);
    else navigation.navigate("File", { path: a.url, name: a.name, kind: a.kind === "video" ? "video" : a.thumb ? "image" : "file" });
  };
  if (media.length === 0 && rest.length === 0) return null;
  return (
    <Section title={title} count={media.length + rest.length}>
      {media.length > 0 ? (
        <ScrollView
          key="media"
          horizontal
          showsHorizontalScrollIndicator={false}
          contentContainerStyle={{ gap: spacing.sm, padding: spacing.md }}
        >
          {media.map((a) => (
            <Pressable
              key={a.id}
              onPress={() => open(a)}
              accessibilityRole="button"
              accessibilityLabel={`${a.kind === "video" ? "Play" : "Open"} ${a.caption ?? a.name}`}
            >
              <ServerImage path={a.thumb!} style={{ width: 200, height: 124, borderRadius: radius.md }} />
              {a.kind === "video" ? (
                <View style={[StyleSheet.absoluteFill, styles.center]}>
                  <Icon name="play-circle" size={40} color="#FFFFFF" />
                </View>
              ) : null}
              <Text variant="caption" tone="secondary" numberOfLines={1} style={{ width: 200, marginTop: spacing.xs }}>
                {a.caption ?? a.name}
              </Text>
            </Pressable>
          ))}
        </ScrollView>
      ) : null}
      {rest.map((a) => (
        <ListRow
          key={a.id}
          icon={artifactIcon(a)}
          title={a.caption ?? a.name}
          subtitle={a.caption ? a.name : undefined}
          onPress={() => open(a)}
          opens={a.kind !== "link"}
        />
      ))}
    </Section>
  );
}

function artifactIcon(a: TaskArtifact): IconName {
  if (a.kind === "link") return a.url.includes("/pull/") ? "git-pull-request-outline" : "link-outline";
  if (a.kind === "pdf") return "document-attach-outline";
  if (a.name.endsWith(".html")) return "newspaper-outline";
  return "document-text-outline";
}

const linkIcons: Record<string, IconName> = { thread: "chatbox-ellipses-outline", ticket: "ticket-outline", chat: "chatbubble-outline" };

/** A thread, ticket or chat that moves the task, with where it stands and its latest word. */
export function LinkRow({ link }: { link: TaskLink }) {
  const at = parseInstant(link.at);
  const status = link.status && link.status !== "unknown" ? (link.status === "completed" ? "finished" : link.status.toLowerCase()) : null;
  const sub = [
    status,
    link.kind === "thread" ? link.machine : link.kind === "ticket" && link.title !== link.ref ? link.ref : null,
    at ? weekdayClock(at) : null,
    link.line,
  ]
    .filter(Boolean)
    .join(" · ");
  return <ListRow icon={linkIcons[link.kind] ?? "link-outline"} title={link.title ?? link.ref} subtitle={sub || undefined} />;
}

/** A subtask or a finding in the page: its mark and title, opening its own page. */
export function TaskLinkRow({ task, onPress }: { task: Issue; onPress: () => void }) {
  const { colors, spacing } = useTheme();
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="button"
      accessibilityLabel={`${task.key} ${task.title}, ${statusName(task)}`}
      style={({ pressed }) => [
        styles.line,
        {
          gap: spacing.md,
          padding: spacing.md,
          paddingHorizontal: spacing.lg,
          backgroundColor: pressed ? colors.surfaceRaised : colors.surface,
        },
      ]}
    >
      <TaskStateMark task={task} />
      <Text style={{ flex: 1 }} numberOfLines={2}>
        {task.title}
      </Text>
      <Icon name="chevron-forward" size={16} color={colors.textTertiary} />
    </Pressable>
  );
}

const styles = StyleSheet.create({
  line: { flexDirection: "row", alignItems: "center" },
  label: { width: 72 },
  center: { alignItems: "center", justifyContent: "center" },
});
