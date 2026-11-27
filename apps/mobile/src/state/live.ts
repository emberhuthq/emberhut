import { createStore } from "zustand/vanilla";
import type { Live } from "@/api";

export interface LiveState {
  /** Each room's live line from its stream; null when Emberhut isn't working there. */
  byRoom: Record<number, Live | null>;
  /** The room's stream is connected: its line is current, not the last poll's. */
  following: Record<number, boolean>;
  set: (room: number, live: Live | null) => void;
  setFollowing: (room: number, following: boolean) => void;
}

export function createLiveStore() {
  return createStore<LiveState>()((set) => ({
    byRoom: {},
    following: {},
    set: (room, live) => set((s) => (sameLive(s.byRoom[room] ?? null, live) ? s : { byRoom: { ...s.byRoom, [room]: live } })),
    setFollowing: (room, following) =>
      set((s) => ({ following: { ...s.following, [room]: following }, byRoom: following ? s.byRoom : { ...s.byRoom, [room]: null } })),
  }));
}

function sameLive(a: Live | null, b: Live | null): boolean {
  return a === b || (a !== null && b !== null && a.text === b.text && a.doing === b.doing);
}

/** What Emberhut is doing in a room: the stream's line while connected, the last state's otherwise. */
export function liveLine(s: LiveState, room: number, polled: Live | null | undefined): Live | null {
  return s.following[room] ? (s.byRoom[room] ?? null) : (polled ?? null);
}
