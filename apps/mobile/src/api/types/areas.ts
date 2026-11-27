/** Rolled up from the leaves. */
export type AreaState = "done" | "working" | "needs_you" | "not_started";

export interface AreaNode {
  id: number;
  title: string;
  owner: string;
  /** The task's own status. */
  status: string;
  state: AreaState;
  action?: string;
  due?: string;
  doneAt?: string;
  /** Only on scan tasks; open = not landed, partly or unchecked. */
  findings?: { total: number; open: number };
  kids: AreaNode[];
}

export interface AreaFinding {
  id: number;
  title: string;
  landed: string;
  /** The scan task it came from. */
  scan: number;
  scanTitle: string;
  fixedBy?: number;
  fixedByTitle?: string;
  fixedByStatus?: string;
}

/** A project's long-running stream of work (state.areas). A done area comes back bare: empty questions and tree. */
export interface AreaTree {
  id: number;
  /** Its project. */
  pid: number;
  name: string;
  status: "active" | "paused" | "done";
  goal?: string;
  about?: string;
  state: AreaState;
  counts: { done: number; working: number; needsYou: number; notStarted: number };
  /** Its tasks waiting on you. */
  questions: AreaNode[];
  findings: {
    total: number;
    landed: number;
    partly: number;
    notLanded: number;
    superseded: number;
    unchecked: number;
    unlanded: AreaFinding[];
  };
  tree: AreaNode[];
}
