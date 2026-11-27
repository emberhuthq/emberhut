import { useNavigation, type NavigationProp, type ParamListBase } from "@react-navigation/native";
import { useState, type ReactNode } from "react";
import { StyleSheet, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import type { Decision, Issue } from "@/api";
import { answerLabel } from "@/features/needsYou/model";
import { useLiftToast } from "@/features/needsYou/UndoToast";
import { actionsStore } from "@/services";
import { useActions } from "@/state/hooks";
import { useTheme } from "@/theme";
import { ActionSheet, Button } from "@/ui";
import { copyDraftFor } from "./copyDraft";
import { taskActions } from "./taskPage";

/** After a tap the page goes back to the list, where the task leaves under the toast's Undo; a beat first, to see it. */
const leaveMs = 350;

/** Back to where you came from after an answer, a beat later; a page opened from a notification stays. */
export function leaveSoon(navigation: Pick<NavigationProp<ParamListBase>, "canGoBack" | "goBack">): void {
  setTimeout(() => navigation.canGoBack() && navigation.goBack(), leaveMs);
}

/**
 * The task's answers and actions, pinned under the page: the first answer as the main button, the rest beside it, then
 * Not now (next week), Archive (put away for good) and Talk (its room). Your own task: Mark done; an archived one: Unarchive.
 */
export function TaskActionsBar({
  task,
  issues,
  decision,
  answersInPage,
  room,
}: {
  task: Issue;
  issues: Issue[];
  decision: Decision | null;
  answersInPage: boolean;
  room: number | null;
}) {
  const { colors, spacing } = useTheme();
  const insets = useSafeAreaInsets();
  const navigation = useNavigation();
  const plan = taskActions(task, issues, answersInPage);
  const sending = useActions((s) => s.sending.has(task.id) || s.sending.has(plan.target));
  // Held for its undo window or on its way: an Undo or a refusal gives the buttons back, as in the reference client.
  const tapped = useActions((s) => s.gone.has(task.id) || s.gone.has(plan.target));
  const [more, setMore] = useState(false);
  const lift = useLiftToast();
  const act = (run: () => void) => {
    run();
    leaveSoon(navigation);
  };
  const answer = (choice: string) =>
    act(() => {
      copyDraftFor(choice, decision);
      actionsStore.getState().decide(plan.target, choice);
    });
  const primary = ((): { label: string; run: () => void } | null => {
    const p = plan.primary;
    switch (p?.kind) {
      case "answer":
        return { label: answerLabel(p.choice), run: () => answer(p.choice) };
      case "didIt":
        return { label: "I did it", run: () => act(() => actionsStore.getState().decide(plan.target, "Done")) };
      case "markDone":
        return { label: "Mark done", run: () => act(() => actionsStore.getState().markDone(task.id)) };
      case "unarchive":
        return { label: "Unarchive", run: () => act(() => actionsStore.getState().unarchive(task.id)) };
      default:
        return null;
    }
  })();
  // As in the reference client: the box starts with what it's about, your words to follow.
  const draft = `About ${task.key} "${task.title}": `;
  const talk = () => navigation.navigate("Tabs", { screen: "Chat", params: room === null ? { draft } : { room, draft } });
  return (
    <View
      onLayout={(e) => lift(e.nativeEvent.layout.height)}
      style={[
        styles.bar,
        {
          gap: spacing.sm,
          padding: spacing.md,
          paddingBottom: insets.bottom + spacing.sm,
          backgroundColor: colors.surface,
          borderTopColor: colors.border,
        },
      ]}
    >
      {primary || plan.others.length > 0 ? (
        <View style={[styles.row, { gap: spacing.sm }]}>
          {primary ? (
            <View style={styles.wide}>
              <Button label={primary.label} onPress={primary.run} loading={sending} disabled={tapped} block />
            </View>
          ) : null}
          {plan.others.length === 1 ? (
            <Grow>
              <Button
                label={answerLabel(plan.others[0]!)}
                variant="secondary"
                onPress={() => answer(plan.others[0]!)}
                disabled={tapped}
                block
              />
            </Grow>
          ) : plan.others.length > 1 ? (
            <Grow>
              <Button
                label="More"
                variant="secondary"
                onPress={() => setMore(true)}
                disabled={tapped}
                accessibilityLabel="Other answers"
                block
              />
            </Grow>
          ) : null}
        </View>
      ) : null}
      <View style={[styles.row, { gap: spacing.sm }]}>
        {plan.notNow ? (
          <Grow>
            <Button
              label="Not now"
              variant="secondary"
              onPress={() => act(() => actionsStore.getState().later(task.id))}
              disabled={tapped}
              accessibilityLabel="Not now, next week"
              block
            />
          </Grow>
        ) : null}
        {plan.archive ? (
          <Grow>
            <Button
              label="Archive"
              variant="secondary"
              onPress={() => act(() => actionsStore.getState().archive(task.id))}
              disabled={tapped}
              accessibilityLabel="Archive, put it away for good"
              block
            />
          </Grow>
        ) : null}
        <Grow>
          <Button label="Talk" variant="secondary" onPress={talk} accessibilityLabel="Talk to Emberhut about this" block />
        </Grow>
      </View>
      {more && (
        <ActionSheet
          visible
          title="Other answers"
          actions={plan.others.map((choice) => ({ label: answerLabel(choice), onPress: () => answer(choice) }))}
          onClose={() => setMore(false)}
        />
      )}
    </View>
  );
}

function Grow({ children }: { children: ReactNode }) {
  return <View style={styles.grow}>{children}</View>;
}

const styles = StyleSheet.create({
  bar: { borderTopWidth: StyleSheet.hairlineWidth },
  row: { flexDirection: "row" },
  grow: { flex: 1 },
  wide: { flex: 2 },
});
