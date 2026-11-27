import { useNavigation } from "@react-navigation/native";
import { useState } from "react";
import { Pressable, StyleSheet, View } from "react-native";
import type { Issue, WorkRow } from "@/api";
import { ServerImage } from "@/features/files/ServerImage";
import { copyDraftFor } from "@/features/tasks/copyDraft";
import { DeadlineChip } from "@/features/tasks/DeadlineChip";
import { parseInstant, relative } from "@/format/time";
import { actionsStore } from "@/services";
import { useTheme } from "@/theme";
import { ActionSheet, Button, Icon, Text, type SheetAction } from "@/ui";
import { answerLabel, cardAction, cardAnswers, isStale, kindNames } from "./model";

/**
 * A card that needs you: the task, the action in bold, its kind and project small. A quick yes or a Done is answered
 * right on the card; anything else opens the task's page to read first. A long press puts it off a week or archives it.
 */
export function NeedsCard({
  row,
  issues,
  project,
  tier = false,
}: {
  row: WorkRow;
  issues: Issue[];
  project?: string | null;
  /** Grouped by project: the chip says Today, This week or This month (overdue in red). */
  tier?: boolean;
}) {
  const { colors, radius, spacing } = useTheme();
  const navigation = useNavigation();
  const [sheet, setSheet] = useState<"answers" | "later" | null>(null);
  const answers = cardAnswers(row, issues);
  const asked = parseInstant(row.asked ?? row.at);
  const stale = isStale(row);
  const task = row.task;
  const action = cardAction(row, issues);
  const answer = (choice: string) => {
    if (!answers) return;
    copyDraftFor(choice, answers.decision);
    actionsStore.getState().decide(answers.task, choice);
  };
  const [first, ...others] = answers?.options ?? [];
  const later: SheetAction[] =
    task == null
      ? []
      : [
          { label: "Not now: next week", icon: "moon-outline", onPress: () => actionsStore.getState().later(task) },
          { label: "Archive: put it away for good", icon: "archive-outline", onPress: () => actionsStore.getState().archive(task) },
        ];
  return (
    <View style={{ backgroundColor: colors.surface, borderRadius: radius.lg, borderWidth: 1, borderColor: `${colors.needsYou}55` }}>
      <Pressable
        onPress={task != null ? () => navigation.navigate("Task", { id: task }) : undefined}
        onLongPress={later.length > 0 ? () => setSheet("later") : undefined}
        accessibilityRole="button"
        accessibilityLabel={`${action}. ${row.title}`}
        accessibilityHint="Opens the task; a long press puts it off"
        style={({ pressed }) => [styles.body, { gap: spacing.md, padding: spacing.lg, opacity: pressed ? 0.8 : 1 }]}
      >
        <Icon name="hand-left" size={22} color={colors.needsYou} />
        <View style={{ flex: 1, gap: spacing.xs }}>
          <View style={[styles.line, { gap: spacing.sm }]}>
            <Text variant="footnote" tone="secondary" numberOfLines={1} style={{ flex: 1 }}>
              {row.title}
            </Text>
            <DeadlineChip day={row.decideBy} hard={row.hardBy === true} decide tier={tier} />
          </View>
          <Text weight="700" numberOfLines={3}>
            {action}
          </Text>
          {row.draft ? (
            <Text variant="footnote" weight="600" tone="needsYou">
              Read the draft first
            </Text>
          ) : null}
          <Text variant="caption" tone="tertiary" numberOfLines={1}>
            <Text variant="caption" tone="needsYou" weight="600">
              {kindNames[row.need ?? "choose"]}
            </Text>
            {project ? ` · ${project}` : ""}
            {stale ? (
              <Text variant="caption" tone="warning">
                {" · may be outdated"}
              </Text>
            ) : asked ? (
              ` · asked ${relative(asked)}`
            ) : (
              ""
            )}
          </Text>
        </View>
        {row.thumb ? <ServerImage path={row.thumb} style={{ width: 52, height: 52, borderRadius: radius.sm }} /> : null}
      </Pressable>
      {answers ? (
        <View style={[styles.line, { gap: spacing.sm, paddingHorizontal: spacing.lg, paddingBottom: spacing.md }]}>
          {first !== undefined ? (
            <>
              <Button label={answerLabel(first)} onPress={() => answer(first)} />
              {others.length === 1 ? (
                <Button label={answerLabel(others[0]!)} variant="secondary" onPress={() => answer(others[0]!)} />
              ) : null}
              {others.length > 1 ? (
                <Button label="More" variant="secondary" onPress={() => setSheet("answers")} accessibilityLabel="Other answers" />
              ) : null}
            </>
          ) : (
            <Button label="I did it" onPress={() => answer("Done")} />
          )}
        </View>
      ) : null}
      {sheet && (
        <ActionSheet
          visible
          title={action}
          actions={sheet === "later" ? later : others.map((o) => ({ label: answerLabel(o), onPress: () => answer(o) }))}
          onClose={() => setSheet(null)}
        />
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  body: { flexDirection: "row", alignItems: "flex-start" },
  line: { flexDirection: "row", alignItems: "center", flexWrap: "wrap" },
});
