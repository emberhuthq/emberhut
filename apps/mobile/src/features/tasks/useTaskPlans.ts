import { useEffect } from "react";
import { usePrefs } from "@/state/hooks";
import { useEmberhutState } from "@/state/queries";

/**
 * Whether the server keeps task plans (state.ui.taskPlans): then Tasks are the work running, each with its plan, and
 * Sessions move under More. Until the state answers, the server's last word, kept across launches, so the tabs don't
 * change under you as the app starts.
 */
export function useTaskPlans(): boolean {
  const state = useEmberhutState();
  const kept = usePrefs((s) => s.taskPlans);
  const setKept = usePrefs((s) => s.setTaskPlans);
  const said = state.data ? state.data.ui?.taskPlans === true : undefined;
  useEffect(() => {
    if (said !== undefined && said !== kept) setKept(said);
  }, [said, kept, setKept]);
  return said ?? kept;
}
