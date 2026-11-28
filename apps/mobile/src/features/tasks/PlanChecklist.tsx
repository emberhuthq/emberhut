import { useNavigation } from "@react-navigation/native";
import { Linking, Pressable, StyleSheet, View } from "react-native";
import { STEP_STATE, type Plan, type PlanSummary, type StepState } from "@/api";
import { useTheme, type Palette } from "@/theme";
import { Icon, ProgressBar, SectionHeading, Text, type IconName } from "@/ui";
import { planFraction, planRows, stepLabel, stepLink, summaryLine, unplannedLine, type PlanRow } from "./plan";

const marks: Record<StepState, [IconName, keyof Palette]> = {
  done: ["checkmark-circle", "done"],
  doing: ["radio-button-on", "working"],
  todo: ["ellipse-outline", "textTertiary"],
  blocked: ["alert-circle", "danger"],
  skipped: ["remove-circle-outline", "textTertiary"],
};

/**
 * A task's plan as a checklist, at the top of its page: the summary ("4 of 9 done · ETA … · updated 3 min ago") and a
 * thin bar, then each step ticked as its agent goes. The step being worked on stands out; a blocked one says on what;
 * a skipped one is struck through; sub-steps sit indented under their step; a step's result opens with a tap.
 * `summary` alone (the page not fetched yet): its line, no steps.
 */
export function PlanChecklist({ plan, summary }: { plan: Plan | null; summary: PlanSummary }) {
  const { colors, radius, spacing } = useTheme();
  const rows = plan ? planRows(plan) : [];
  const fraction = planFraction(summary);
  const quiet = unplannedLine(summary);
  return (
    <View style={{ gap: spacing.sm }}>
      <SectionHeading>Plan</SectionHeading>
      <View style={{ backgroundColor: colors.surface, borderRadius: radius.lg, overflow: "hidden" }}>
        <View style={{ padding: spacing.lg, paddingBottom: rows.length > 0 ? spacing.sm : spacing.lg, gap: spacing.sm }}>
          <Text variant="footnote" tone="secondary" accessibilityRole="summary">
            {summaryLine(summary)}
          </Text>
          {fraction !== null ? (
            <ProgressBar
              value={fraction}
              color={summary.blocker ? colors.danger : colors.working}
              label={`${summary.done} of ${summary.total} steps done`}
            />
          ) : null}
          {quiet ? (
            <Text variant="footnote" tone="tertiary">
              {quiet}
            </Text>
          ) : null}
        </View>
        {rows.length > 0 ? (
          <View accessibilityRole="list" style={{ paddingBottom: spacing.sm }}>
            {rows.map((row) => (
              <StepRow key={row.step.id} row={row} />
            ))}
          </View>
        ) : null}
      </View>
    </View>
  );
}

function StepRow({ row }: { row: PlanRow }) {
  const { colors, spacing, radius } = useTheme();
  const navigation = useNavigation();
  const [icon, tint] = marks[row.state];
  const doing = row.current && row.state !== STEP_STATE.done && row.state !== STEP_STATE.skipped;
  const link = stepLink(row.step.link);
  const open = () => {
    const target = row.step.link?.trim();
    if (!link || !target) return;
    if (link.kind === "web") void Linking.openURL(target).catch(() => undefined);
    else if (link.kind === "server") navigation.navigate("File", { path: target, name: link.text, kind: "file" });
  };
  return (
    <View
      style={[
        styles.row,
        {
          gap: spacing.md,
          marginHorizontal: spacing.sm,
          paddingHorizontal: spacing.sm,
          paddingVertical: spacing.sm,
          paddingLeft: spacing.sm + (row.depth === 1 ? spacing.xl : 0),
          borderRadius: radius.md,
          backgroundColor: doing ? colors.surfaceRaised : "transparent",
        },
      ]}
    >
      <View style={styles.mark} accessibilityElementsHidden importantForAccessibility="no-hide-descendants">
        <Icon name={icon} size={row.depth === 1 ? 16 : 20} color={colors[tint]} />
      </View>
      <View style={styles.text}>
        {/* The step read as one: its state and note in words; its link is a control of its own. */}
        <View
          accessible
          accessibilityLabel={stepLabel(row)}
          accessibilityState={{ checked: row.state === STEP_STATE.done, disabled: row.state === STEP_STATE.skipped }}
          style={styles.text}
        >
          <Text
            variant={row.depth === 1 ? "callout" : "body"}
            weight={doing ? "600" : undefined}
            tone={row.state === STEP_STATE.done || row.state === STEP_STATE.skipped ? "secondary" : "primary"}
            style={row.state === STEP_STATE.skipped ? styles.struck : undefined}
          >
            {row.step.title}
          </Text>
          {doing && row.state === STEP_STATE.doing ? (
            <Text variant="caption" weight="600" style={{ color: colors.working }}>
              Working on it now
            </Text>
          ) : null}
          {row.step.note ? (
            <Text variant="footnote" tone={row.state === STEP_STATE.blocked ? "danger" : "secondary"} selectable>
              {row.state === STEP_STATE.blocked ? `Blocked: ${row.step.note}` : row.step.note}
            </Text>
          ) : null}
        </View>
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
              style={[styles.link, { gap: spacing.xs }]}
            >
              <Icon name={link.kind === "web" ? "open-outline" : "document-outline"} size={14} color={colors.accent} />
              <Text variant="footnote" tone="accent" numberOfLines={1} style={styles.fill}>
                {link.text}
              </Text>
            </Pressable>
          )
        ) : null}
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  row: { flexDirection: "row", alignItems: "flex-start" },
  mark: { width: 22, minHeight: 22, alignItems: "center", justifyContent: "center" },
  text: { flex: 1, gap: 2 },
  struck: { textDecorationLine: "line-through" },
  link: { flexDirection: "row", alignItems: "center", alignSelf: "flex-start", paddingTop: 2 },
  fill: { flexShrink: 1 },
});
