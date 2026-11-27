import { createJSONStorage, persist, type StateStorage } from "zustand/middleware";
import { createStore } from "zustand/vanilla";
import type { Appearance, ThemeChange, ThemePage } from "@/api";
import { mainRoom } from "@/config/constants";

export type ThemePreference = Appearance;

/** The Sessions tab's Worker and Environment picks (sessionFilters.ts); none picked: all. */
export interface SessionFilter {
  workers?: string[] | null;
  machines?: string[] | null;
  accounts?: string[] | null;
}

/** How Needs you is grouped: by project, the default, or by when you have to decide. */
export type NeedsGroupingPreference = "project" | "when";

export interface PrefsState {
  /** The appearance this phone kept for itself before the theme lived on the server: sent there once. */
  theme: ThemePreference;
  /** The last theme Emberhut answered (or the change you made on top of it), so the app looks right offline. */
  themeCache: ThemePage | null;
  /** A theme change made offline, sent when Emberhut answers again. */
  themePending: ThemeChange | null;
  /** The local appearance above has been offered to the server (or the server already had a choice). */
  themeMigrated: boolean;
  /** The room on screen, kept across launches. */
  room: number;
  needsGrouping: NeedsGroupingPreference;
  /** The Sessions tab's Worker and Environment picks, kept across launches. */
  sessionFilter: SessionFilter;
  setTheme: (theme: ThemePreference) => void;
  setThemeCache: (page: ThemePage | null) => void;
  setThemePending: (change: ThemeChange | null) => void;
  setThemeMigrated: () => void;
  setRoom: (room: number) => void;
  setNeedsGrouping: (grouping: NeedsGroupingPreference) => void;
  setSessionFilter: (filter: SessionFilter) => void;
}

/** What this device keeps for itself (not secrets: those are in secure storage). */
export function createPrefsStore(storage: StateStorage) {
  return createStore<PrefsState>()(
    persist(
      (set) => ({
        theme: "system",
        themeCache: null,
        themePending: null,
        themeMigrated: false,
        room: mainRoom,
        needsGrouping: "project",
        sessionFilter: {},
        setTheme: (theme) => set({ theme }),
        setThemeCache: (themeCache) => set({ themeCache }),
        setThemePending: (themePending) => set({ themePending }),
        setThemeMigrated: () => set({ themeMigrated: true }),
        setRoom: (room) => set({ room }),
        setNeedsGrouping: (needsGrouping) => set({ needsGrouping }),
        setSessionFilter: (sessionFilter) => set({ sessionFilter }),
      }),
      {
        name: "emberhut.prefs",
        // Still 1: the theme fields and sessionFilter are new keys, filled with their defaults on an older save.
        version: 1,
        storage: createJSONStorage(() => storage),
        partialize: ({ theme, themeCache, themePending, themeMigrated, room, needsGrouping, sessionFilter }) => ({
          theme,
          themeCache,
          themePending,
          themeMigrated,
          room,
          needsGrouping,
          sessionFilter,
        }),
      },
    ),
  );
}
