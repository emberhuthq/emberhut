import { useState } from "react";
import { Pressable, StyleSheet, View } from "react-native";
import { taskKey, type AgentNow, type RunnerSession, type ExternalThread } from "@/api";
import { shortAgo } from "@/features/chat/rooms";
import { useTheme } from "@/theme";
import { Icon, Text } from "@/ui";
import { Elapsed } from "./Elapsed";
import { sessionMeta, sessionStatusText, sessionSummary, shortTitle } from "./sessionList";
import { useSessionMenu, type MoveTo, type SessionMenu } from "./SessionMenu";
import { LiveDot, StatusMark } from "./StatusMark";

function Row({
  children,
  onPress,
  label,
  press,
}: {
  children: React.ReactNode;
  onPress?: () => void;
  label: string;
  press?: SessionMenu["press"];
}) {
  const { colors, spacing } = useTheme();
  return (
    <Pressable
      {...press}
      onPress={onPress}
      disabled={!onPress}
      accessibilityRole={onPress ? "button" : undefined}
      accessibilityLabel={label}
      style={({ pressed }) => [
        {
          gap: 3,
          paddingHorizontal: spacing.lg,
          paddingVertical: spacing.md,
          backgroundColor: pressed ? colors.surfaceRaised : colors.surface,
        },
      ]}
    >
      {children}
    </Pressable>
  );
}

/** Running: its live line under the title, how long it's been at it. A long press offers `onMove` (Move to task…). */
export function RunningRow({ session, onOpen, onMove }: { session: RunnerSession; onOpen: () => void; onMove?: MoveTo }) {
  const { colors, spacing } = useTheme();
  const title = shortTitle(session.title);
  const menu = useSessionMenu(session, onMove);
  return (
    <>
      <Row onPress={onOpen} label={`${title}, running, ${session.doing ?? ""}`} press={menu.press}>
        <View style={[styles.line, { gap: spacing.sm }]}>
          <StatusMark status={session.status} />
          <Text weight="500" numberOfLines={1} style={styles.fill}>
            {title}
          </Text>
          <Elapsed since={session.since} style={{ color: colors.done }} />
        </View>
        <Text variant="footnote" tone="secondary" numberOfLines={2}>
          {session.doing ?? sessionStatusText(session.status)}
        </Text>
        <Text variant="caption" tone="tertiary" numberOfLines={1}>
          {sessionMeta(session)}
        </Text>
      </Row>
      {menu.sheet}
    </>
  );
}

/**
 * Finished, stopped, failed or waiting: one line; the chevron shows what it said last, the row opens it, a long press
 * offers `onMove` (Move to task…).
 */
export function RecentRow({ session, onOpen, onMove }: { session: RunnerSession; onOpen: () => void; onMove?: MoveTo }) {
  const { colors, spacing } = useTheme();
  const [open, setOpen] = useState(false);
  const menu = useSessionMenu(session, onMove);
  const summary = sessionSummary(session);
  const ago = shortAgo(session.updatedAt);
  const title = shortTitle(session.title);
  return (
    <View style={{ backgroundColor: colors.surface }}>
      <View style={[styles.line, { paddingLeft: spacing.sm }]}>
        <Pressable
          onPress={() => setOpen((o) => !o)}
          disabled={!summary}
          accessibilityRole="button"
          accessibilityLabel={open ? "Hide what it said" : "Show what it said"}
          style={styles.chevron}
        >
          {summary && <Icon name={open ? "chevron-down" : "chevron-forward"} size={14} color={colors.textTertiary} />}
        </Pressable>
        <Pressable
          {...menu.press}
          onPress={onOpen}
          accessibilityRole="button"
          accessibilityLabel={`${title}, ${sessionStatusText(session.status)}`}
          style={({ pressed }) => [
            styles.line,
            styles.fill,
            { gap: spacing.sm, paddingRight: spacing.lg, paddingVertical: spacing.md, opacity: pressed ? 0.7 : 1 },
          ]}
        >
          <StatusMark status={session.status} />
          <View style={styles.fill}>
            <Text variant="callout" weight={session.status === "waiting" ? "600" : "400"} numberOfLines={1}>
              {title}
            </Text>
            <Text variant="caption" tone="tertiary" numberOfLines={1}>
              {sessionMeta(session)}
            </Text>
          </View>
          {ago && (
            <Text variant="caption" tone="tertiary">
              {ago}
            </Text>
          )}
        </Pressable>
      </View>
      {open && summary && (
        <Text
          variant="footnote"
          tone="secondary"
          selectable
          style={{ paddingLeft: spacing.xxl + spacing.lg, paddingRight: spacing.lg, paddingBottom: spacing.md }}
        >
          {summary}
        </Text>
      )}
      {menu.sheet}
    </View>
  );
}

/** An agent on one of your tasks: its lead agent with the task, its helpers indented under it. */
export function AgentRow({ agent, alone, onOpen }: { agent: AgentNow; alone: boolean; onOpen?: () => void }) {
  const { colors, spacing } = useTheme();
  const name = agent.name ?? (agent.lead ? "Lead" : "Agent");
  const title = agent.lead ? `${agent.key} ${agent.title ?? ""}`.trim() : name;
  const line = agent.lead
    ? `${name} · ${agent.line ?? "Thinking…"}`
    : alone
      ? `${agent.key} · ${agent.line ?? agent.title ?? ""}`
      : (agent.line ?? "Working");
  return (
    <Row onPress={onOpen} label={`${title}, ${line}`}>
      <View style={[styles.line, { gap: spacing.sm, paddingLeft: agent.lead || alone ? 0 : spacing.xl }]}>
        <LiveDot color={colors.done} />
        <Text weight={agent.lead ? "500" : "400"} variant={agent.lead ? "body" : "callout"} numberOfLines={1} style={styles.fill}>
          {title}
        </Text>
        <Elapsed since={agent.started} tone="secondary" />
      </View>
      <Text
        variant="footnote"
        tone="secondary"
        numberOfLines={1}
        style={{ paddingLeft: agent.lead || alone ? 0 : spacing.xl + spacing.lg }}
      >
        {line}
      </Text>
    </Row>
  );
}

/** An external thread still running, read only: it finishes in the app that started it. `place` is its machine's name, else its id. */
export function ExternalThreadRow({ thread, place }: { thread: ExternalThread; place: string }) {
  const { spacing } = useTheme();
  const meta = [
    thread.tasks.map(taskKey).join(", "),
    thread.project,
    `External · ${place}`,
    thread.status === "waiting" ? "waiting for you" : null,
  ]
    .filter(Boolean)
    .join(" · ");
  return (
    <Row label={`${thread.title}, ${meta}`}>
      <View style={[styles.line, { gap: spacing.sm }]}>
        <StatusMark status={thread.status === "waiting" ? "interrupted" : thread.status} />
        <Text weight="500" numberOfLines={1} style={styles.fill}>
          {thread.title}
        </Text>
      </View>
      {thread.last ? (
        <Text variant="footnote" tone="secondary" numberOfLines={2}>
          {thread.last.replace(/[*`]/g, "")}
        </Text>
      ) : null}
      <Text variant="caption" tone="tertiary" numberOfLines={1}>
        {meta}
      </Text>
    </Row>
  );
}

const styles = StyleSheet.create({
  line: { flexDirection: "row", alignItems: "center" },
  fill: { flex: 1 },
  chevron: { width: 28, height: 44, alignItems: "center", justifyContent: "center" },
});
