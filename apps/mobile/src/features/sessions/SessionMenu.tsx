import { useNavigation } from "@react-navigation/native";
import { useState, type ReactNode } from "react";
import type { AccessibilityActionEvent, AccessibilityActionInfo } from "react-native";
import { useTaskPlans } from "@/features/tasks/useTaskPlans";
import { ActionSheet } from "@/ui";
import { isMovable } from "./moveToTask";
import { shortTitle } from "./sessionList";

/** What a session's menu acts on. */
export interface MenuSession {
  id: string;
  title: string;
  tasks?: readonly number[];
}

/** Opens Move to task… for a session. */
export type MoveTo = (session: MenuSession) => void;

/** A session row's menu: what to spread on its Pressable, and the sheet to put beside it. */
export interface SessionMenu {
  press: {
    onLongPress?: () => void;
    accessibilityActions?: AccessibilityActionInfo[];
    onAccessibilityAction?: (event: AccessibilityActionEvent) => void;
  };
  sheet: ReactNode;
}

/** The name of the accessibility action that moves a session to another task. */
const MOVE = "move";

/**
 * Opens Move to task… for a session; undefined where it isn't offered: a server that keeps no plans, or a session it
 * can't move (only a runner's and a worker's move).
 */
export function useMoveToTask(): MoveTo | undefined {
  const plans = useTaskPlans();
  const navigation = useNavigation();
  if (!plans) return undefined;
  return (session) => {
    if (!isMovable(session.id)) return;
    navigation.navigate("MoveToTask", {
      session: session.id,
      title: shortTitle(session.title),
      tasks: session.tasks ? [...session.tasks] : undefined,
    });
  };
}

/**
 * A session row's long-press menu: Move to task… (`move`, from the list's useMoveToTask, so the rows don't each read the
 * state), also as an accessibility action. Spread `press` on the row's Pressable and put `sheet` beside it; both are
 * empty where nothing is offered.
 */
export function useSessionMenu(session: MenuSession, move: MoveTo | undefined): SessionMenu {
  const [open, setOpen] = useState(false);
  if (!move || !isMovable(session.id)) return { press: {}, sheet: null };
  return {
    press: {
      onLongPress: () => setOpen(true),
      accessibilityActions: [{ name: MOVE, label: "Move to task…" }],
      onAccessibilityAction: (event) => {
        if (event.nativeEvent.actionName === MOVE) move(session);
      },
    },
    sheet: open ? (
      <ActionSheet
        visible
        title={shortTitle(session.title)}
        actions={[{ label: "Move to task…", icon: "swap-horizontal-outline", onPress: () => move(session) }]}
        onClose={() => setOpen(false)}
      />
    ) : null,
  };
}
