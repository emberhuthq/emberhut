import { useStore } from "zustand";
import { createStore } from "zustand/vanilla";

interface LearningFold {
  /** Whether "Learning from you" is open: folded until you open it, and as you left it for the rest of the session. */
  open: boolean;
  toggle: () => void;
}

/** "Learning from you"'s fold, kept in memory for the session (not across launches). */
export const learningFold = createStore<LearningFold>()((set) => ({
  open: false,
  toggle: () => set((s) => ({ open: !s.open })),
}));

export const useLearningFold = <T>(select: (s: LearningFold) => T): T => useStore(learningFold, select);
