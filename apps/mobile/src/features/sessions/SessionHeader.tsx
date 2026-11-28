import { useNavigation } from "@react-navigation/native";
import { Pressable, StyleSheet, View } from "react-native";
import type { SessionHead } from "@/api";
import { useTheme } from "@/theme";
import { Icon, Text } from "@/ui";
import { Elapsed } from "./Elapsed";
import { effortName, modelName, sessionStatusText, shortPath, shortTitle } from "./sessionList";
import { StatusMark } from "./StatusMark";

export interface SessionHeaderProps {
  title: string;
  /** A helper agent's view: whose it is. */
  lead: string | null;
  status: string;
  running: boolean;
  since: string | null;
  connected: boolean;
  head: SessionHead | null;
  direct: boolean;
  tasks: { id: number; title: string }[];
  problem: string | null;
  /** Move to task…, where it's offered (a server that keeps plans). */
  onMove?: () => void;
}

/** Its status and how long it's been running, live or reconnecting, where and on what it runs, its tasks, Move to task…. */
export function SessionHeader({
  title,
  lead,
  status,
  running,
  since,
  connected,
  head,
  direct,
  tasks,
  problem,
  onMove,
}: SessionHeaderProps) {
  const { colors, radius, spacing } = useTheme();
  const navigation = useNavigation();
  const where = [head?.place, head?.accountName ?? head?.account, head?.cwd ? shortPath(head.cwd) : null, head?.branch]
    .filter(Boolean)
    .join(" · ");
  const model = direct ? [modelName(head?.model), head?.effort ? effortName(head.effort) : null].filter(Boolean).join(" · ") : "";
  return (
    <View style={{ gap: spacing.xs + 2, padding: spacing.md, borderRadius: radius.lg, backgroundColor: colors.surface }}>
      <Text variant="headline">{shortTitle(title)}</Text>
      {lead && (
        <Text variant="caption" tone="secondary">
          A helper agent of {shortTitle(lead)}
        </Text>
      )}
      <View style={[styles.row, { gap: spacing.sm }]}>
        <StatusMark status={status} />
        <Text variant="callout" weight="600">
          {sessionStatusText(status)}
        </Text>
        {running && <Elapsed since={since} tone="done" variant="callout" />}
        <View style={styles.fill} />
        {running && (
          <View style={[styles.row, { gap: 3 }]}>
            <Icon name={connected ? "radio-outline" : "cloud-offline-outline"} size={14} color={connected ? colors.done : colors.warning} />
            <Text variant="caption" tone={connected ? "done" : "warning"}>
              {connected ? "Live" : "Reconnecting"}
            </Text>
          </View>
        )}
      </View>
      {where !== "" && (
        <Text variant="caption" tone="secondary">
          {where}
        </Text>
      )}
      {model !== "" && (
        <Text variant="caption" tone="secondary">
          {model}
        </Text>
      )}
      {tasks.map((t) => (
        <Pressable
          key={t.id}
          onPress={() => navigation.navigate("Task", { id: t.id })}
          accessibilityRole="button"
          accessibilityLabel={`Open ${t.title}`}
          style={[styles.row, { gap: spacing.sm, minHeight: 32 }]}
        >
          <Text variant="callout" tone="accent" numberOfLines={1} style={styles.fill}>
            {t.title}
          </Text>
          <Icon name="chevron-forward" size={14} color={colors.accent} />
        </Pressable>
      ))}
      {onMove && (
        <Pressable
          onPress={onMove}
          accessibilityRole="button"
          accessibilityLabel="Move to task…"
          accessibilityHint="Picks the task its next turns work on"
          style={[styles.row, { gap: spacing.sm, minHeight: 32 }]}
        >
          <Icon name="swap-horizontal-outline" size={16} color={colors.accent} />
          <Text variant="callout" tone="accent" style={styles.fill}>
            Move to task…
          </Text>
        </Pressable>
      )}
      {problem && (
        <Text variant="footnote" tone="danger">
          {problem}
        </Text>
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  row: { flexDirection: "row", alignItems: "center" },
  fill: { flex: 1 },
});
