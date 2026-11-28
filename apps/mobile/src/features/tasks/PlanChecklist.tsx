import { useNavigation } from "@react-navigation/native";
import { useState } from "react";
import { Linking, Pressable, StyleSheet, View } from "react-native";
import { STEP_STATE, type Plan } from "@/api";
import { mix } from "@/theme/colors";
import { useTheme } from "@/theme";
import { Icon, SectionHeading, Text } from "@/ui";
import { isEmptyPlan, planRows, stepLabel, stepLink, unplannedLine, type PlanRow } from "./plan";
import { foldChecklist, soFar } from "./planView";
import { BlockedMark, Ring } from "./PlanMarks";

/**
 * A task's plan as a checklist its agent ticks: big round ticks, done steps dimmed in place, the step being worked on
 * lit (with its time so far), a blocked one with its note and a way down to the ask, a skipped one struck through,
 * sub-steps indented, each result under its step. Past eight rows it folds around the current step. `onAnswer`: scrolls
 * to what you're asked, when there is an ask.
 */
export function PlanChecklist({ plan, onAnswer }: { plan: Plan; onAnswer?: () => void }) {
  const { colors, radius, spacing } = useTheme();
  const [open, setOpen] = useState(false);
  const rows = planRows(plan);
  // Steps counted but not listed: the page hasn't answered yet.
  if (rows.length === 0) return isEmptyPlan(plan.summary) ? <NoPlan plan={plan} /> : null;
  const entries = foldChecklist(rows, open);
  return (
    <View style={{ gap: spacing.sm }}>
      <SectionHeading>{`Plan · ${rows.length} ${rows.length === 1 ? "step" : "steps"}`}</SectionHeading>
      <View accessibilityRole="list" style={{ backgroundColor: colors.surface, borderRadius: radius.lg, paddingVertical: spacing.sm }}>
        {entries.map((entry, i) =>
          entry.kind === "step" ? (
            <StepRow key={entry.row.step.id} row={entry.row} onAnswer={onAnswer} />
          ) : (
            <Pressable
              key={`fold-${i}`}
              onPress={() => setOpen(true)}
              accessibilityRole="button"
              accessibilityLabel={`Show ${entry.label}: ${entry.rows.map((r) => r.step.title).join(", ")}`}
              style={({ pressed }) => [
                styles.row,
                {
                  gap: spacing.md,
                  marginHorizontal: spacing.sm,
                  padding: spacing.sm,
                  borderRadius: radius.md,
                  backgroundColor: pressed ? colors.surfaceRaised : "transparent",
                },
              ]}
            >
              <View style={styles.mark}>
                <Icon name={entry.place === "before" ? "checkmark-done" : "ellipsis-horizontal"} size={18} color={colors.textTertiary} />
              </View>
              <Text variant="callout" tone="secondary" numberOfLines={1} style={styles.fill}>
                {entry.label}
                <Text variant="callout" tone="tertiary">{` · ${entry.rows.map((r) => r.step.title).join(", ")}`}</Text>
              </Text>
              <Icon name="chevron-forward" size={16} color={colors.textTertiary} />
            </Pressable>
          ),
        )}
      </View>
    </View>
  );
}

function StepRow({ row, onAnswer }: { row: PlanRow; onAnswer?: () => void }) {
  const { colors, spacing, radius } = useTheme();
  const navigation = useNavigation();
  const sub = row.depth === 1;
  const size = sub ? 18 : 22;
  const doing = row.state === STEP_STATE.doing && row.current;
  const link = stepLink(row.step.link);
  const time = row.current ? soFar(row.step) : null;
  const open = () => {
    const target = row.step.link?.trim();
    if (!link || !target) return;
    if (link.kind === "web") void Linking.openURL(target).catch(() => undefined);
    else if (link.kind === "server") navigation.navigate("File", { path: target, name: link.text, kind: "file" });
  };
  const mark =
    row.state === STEP_STATE.done ? (
      <Icon name="checkmark-circle" size={size + 2} color={colors.done} />
    ) : row.state === STEP_STATE.doing ? (
      <View style={[styles.center, { width: size, height: size, borderRadius: size / 2, borderWidth: 2, borderColor: colors.running }]}>
        <View style={{ width: size / 2.6, height: size / 2.6, borderRadius: size, backgroundColor: colors.running }} />
      </View>
    ) : row.state === STEP_STATE.blocked ? (
      <BlockedMark size={size} />
    ) : row.state === STEP_STATE.skipped ? (
      <Ring size={size} color={colors.textTertiary} dashed />
    ) : (
      <Ring size={size} color={colors.textSecondary} />
    );
  const faint = row.state === STEP_STATE.skipped;
  return (
    <View
      style={[
        styles.row,
        {
          gap: spacing.md,
          marginHorizontal: spacing.sm,
          paddingVertical: spacing.sm,
          paddingRight: spacing.sm,
          paddingLeft: spacing.sm + (sub ? 34 : 0),
          borderRadius: radius.md,
          backgroundColor: doing ? mix(colors.running, colors.surface, 0.12) : "transparent",
        },
      ]}
    >
      <View style={styles.mark} accessibilityElementsHidden importantForAccessibility="no-hide-descendants">
        {mark}
      </View>
      <View style={styles.text}>
        {/* The step read as one: its state and note in words; its result and Answer are controls of their own. */}
        <View
          accessible
          accessibilityLabel={[stepLabel(row), time].filter(Boolean).join(", ")}
          accessibilityState={{ checked: row.state === STEP_STATE.done, disabled: faint }}
          style={styles.text}
        >
          <Text
            variant={sub ? "callout" : "body"}
            weight={doing ? "600" : undefined}
            tone={faint ? "tertiary" : row.state === STEP_STATE.done ? "secondary" : "primary"}
            style={faint ? styles.struck : undefined}
          >
            {row.step.title}
          </Text>
          {doing && (row.step.note || time) ? (
            <Text variant="footnote" tone="secondary">
              {[row.step.note ? `Now: ${row.step.note}` : null, time].filter(Boolean).join(" · ")}
            </Text>
          ) : null}
          {row.state === STEP_STATE.blocked ? (
            <Text variant="footnote" tone="danger">
              {row.step.note ?? "Blocked"}
            </Text>
          ) : row.state === STEP_STATE.skipped && row.step.note ? (
            <Text variant="footnote" tone="tertiary">{`Skipped: ${row.step.note}`}</Text>
          ) : !doing && row.step.note ? (
            <Text variant="footnote" tone="secondary">
              {row.step.note}
            </Text>
          ) : null}
        </View>
        {row.state === STEP_STATE.blocked && onAnswer ? (
          <Pressable
            onPress={onAnswer}
            hitSlop={8}
            accessibilityRole="button"
            accessibilityLabel="Go to what you're asked"
            style={styles.inline}
          >
            <Text variant="footnote" tone="accent" weight="600">
              Answer ↓
            </Text>
          </Pressable>
        ) : null}
        {link ? (
          link.kind === "text" ? (
            <Text variant="footnote" tone="secondary" selectable numberOfLines={1}>
              {link.text}
            </Text>
          ) : (
            <Pressable
              onPress={open}
              hitSlop={8}
              accessibilityRole="link"
              accessibilityLabel={`Open the result of ${row.step.title}: ${link.text}`}
              style={[styles.inline, { gap: spacing.xs }]}
            >
              <Icon name={link.kind === "web" ? "link-outline" : "document-outline"} size={14} color={colors.accent} />
              <Text variant="footnote" tone="accent" numberOfLines={1} style={styles.shrink}>
                {link.text}
              </Text>
            </Pressable>
          )
        ) : null}
      </View>
    </View>
  );
}

/** No steps yet: a quiet box saying so, and when a turn of work ended without writing any. */
function NoPlan({ plan }: { plan: Plan }) {
  const { colors, radius, spacing } = useTheme();
  const quiet = unplannedLine(plan.summary);
  return (
    <View style={{ gap: spacing.sm }}>
      <SectionHeading>Plan</SectionHeading>
      <View
        accessible
        accessibilityLabel={["No plan yet. Emberhut writes the steps when it starts.", quiet].filter(Boolean).join(" ")}
        style={{
          borderWidth: 1.5,
          borderStyle: "dashed",
          borderColor: colors.border,
          borderRadius: radius.lg,
          padding: spacing.lg,
          gap: spacing.md,
        }}
      >
        {[0.7, 0.5, 0.6].map((width, i) => (
          <View key={i} style={[styles.row, { gap: spacing.md, alignItems: "center" }]}>
            <Ring size={16} color={colors.border} />
            <View style={{ height: 6, width: `${width * 80}%`, borderRadius: 3, backgroundColor: colors.border }} />
          </View>
        ))}
        <View style={{ gap: spacing.xs, alignItems: "center", paddingTop: spacing.sm }}>
          <Text variant="headline">No plan yet</Text>
          <Text variant="footnote" tone="secondary" align="center">
            Emberhut writes the steps when it starts.
          </Text>
          {quiet ? (
            <Text variant="footnote" tone="tertiary" align="center">
              {quiet}
            </Text>
          ) : null}
        </View>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  row: { flexDirection: "row", alignItems: "flex-start" },
  center: { alignItems: "center", justifyContent: "center" },
  mark: { width: 24, minHeight: 24, alignItems: "center", justifyContent: "center" },
  text: { flex: 1, gap: 2 },
  struck: { textDecorationLine: "line-through" },
  inline: { flexDirection: "row", alignItems: "center", alignSelf: "flex-start", paddingTop: 2 },
  shrink: { flexShrink: 1 },
  fill: { flex: 1 },
});
