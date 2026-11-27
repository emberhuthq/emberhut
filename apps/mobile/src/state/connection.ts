import { createStore } from "zustand/vanilla";

/** A blip (the server busy a second or two) shows nothing; only this long of failing says Emberhut can't be reached. */
export const quietMs = 5_000;

export interface ConnectionState {
  /** When polls started failing; null while they succeed. */
  failingSince: number | null;
  lastFailure: number | null;
  reason: string | null;
  noteSuccess: () => void;
  noteFailure: (reason: string, now?: number) => void;
}

export function createConnectionStore() {
  return createStore<ConnectionState>()((set, get) => ({
    failingSince: null,
    lastFailure: null,
    reason: null,
    noteSuccess() {
      if (get().failingSince !== null) set({ failingSince: null, lastFailure: null, reason: null });
    },
    noteFailure(reason, now = Date.now()) {
      set((s) => ({ failingSince: s.failingSince ?? now, lastFailure: now, reason }));
    },
  }));
}

/** The line to show, once polls have failed for `quietMs`. */
export const offlineReason = (s: ConnectionState): string | null =>
  s.failingSince !== null && s.lastFailure !== null && s.lastFailure - s.failingSince >= quietMs ? s.reason : null;
