import { useNavigation } from "@react-navigation/native";
import { StyleSheet, View, Pressable } from "react-native";
import type { RunnerSession } from "@/api";
import { Elapsed } from "@/features/sessions/Elapsed";
import { isRunning, sessionStatusText, sessionSummary, shortTitle } from "@/features/sessions/sessionList";
import { parseInstant, relative } from "@/format/time";
import { minTap, useTheme } from "@/theme";
import { Icon, Text } from "@/ui";
import { Spinner } from "./PlanMarks";

/**
 * A session with no task, in the Tasks list: lighter than a task (no plan, never a bar). Its mark (the spinner while it
 * runs, the hand when it waits on you, a terminal otherwise), its title, what it's doing or said last, where it runs and
 * for how long. A tap opens the session.
 */
export function SessionItemRow({ session }: { session: RunnerSession }) {
  const { colors, spacing } = useTheme();
  const navigation = useNavigation();
  const title = shortTitle(session.title);
  const running = isRunning(session.status);
  const waiting = session.status === "waiting";
  const line = running
    ? (session.doing ?? sessionStatusText(session.status))
    : (sessionSummary(session) ?? sessionStatusText(session.status));
  const updated = parseInstant(session.updatedAt);
  const where = ["Session, no task", session.place, !running && updated ? relative(updated) : null].filter(Boolean).join(" · ");
  return (
    <Pressable
      onPress={() => navigation.navigate("Session", { id: session.id, title })}
      accessibilityRole="button"
      accessibilityLabel={`Session ${title}, ${sessionStatusText(session.status)}, ${line}, ${where}`}
      accessibilityHint="Opens the session"
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
      <View style={styles.lead} accessibilityElementsHidden importantForAccessibility="no-hide-descendants">
        {running ? (
          <Spinner />
        ) : waiting ? (
          <Icon name="hand-left" size={16} color={colors.needsYou} />
        ) : (
          <Icon name="terminal-outline" size={16} color={colors.textTertiary} />
        )}
      </View>
      <View style={styles.text}>
        <View style={[styles.line, { gap: spacing.sm }]}>
          <Text numberOfLines={1} style={styles.fill}>
            {title}
          </Text>
          {running ? <Elapsed since={session.since} variant="footnote" tone="secondary" /> : null}
        </View>
        <Text variant="footnote" tone={waiting ? "needsYou" : "secondary"} numberOfLines={1}>
          {line}
        </Text>
        <Text variant="caption" tone="tertiary" numberOfLines={1}>
          {where}
        </Text>
      </View>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  row: { minHeight: minTap, flexDirection: "row", alignItems: "flex-start" },
  lead: { width: 18, height: 22, alignItems: "center", justifyContent: "center" },
  text: { flex: 1, gap: 2 },
  line: { flexDirection: "row", alignItems: "center" },
  fill: { flex: 1 },
});
