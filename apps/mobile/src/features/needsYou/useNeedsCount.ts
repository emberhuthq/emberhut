import { useActions } from "@/state/hooks";
import { useEmberhutState } from "@/state/queries";
import { needsCount } from "./model";

/** The Needs you tab's badge: what waits on you (the asks, Later left out, and the approvals), as the page counts it. */
export function useNeedsCount(): number {
  const state = useEmberhutState();
  const gone = useActions((s) => s.gone);
  return needsCount(state.data, gone);
}
