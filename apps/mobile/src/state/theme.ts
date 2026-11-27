import type { StoreApi } from "zustand";
import { createStore } from "zustand/vanilla";
import { ApiError, type EmberhutApi, type ThemeChange, type ThemeChoice, type ThemePage, type ThemeReply } from "@/api";
import { inBackground } from "@/log";
import { builtInTheme } from "@/theme/builtIn";
import type { PrefsState } from "./prefs";

export interface ThemeState {
  /** A change on its way to Emberhut. */
  saving: boolean;
  /** Why the last change didn't take. */
  error: string | null;
  /** Fetches the theme (and sends what was changed offline, and the old local appearance once). */
  refresh: () => Promise<void>;
  /** The choice a state poll carried: fetches the theme when another device changed it, sends a held change. */
  noteState: (choice: ThemeChoice | null | undefined) => void;
  /**
   * Applies a change at once and saves it. A preset or the appearance works offline (from the cached presets, sent
   * later); a custom accent needs the server, and its preview's answer to show before the server's.
   */
  choose: (change: ThemeChange, preview?: ThemePage) => Promise<void>;
  /** A custom accent's tokens and contrast, nothing saved. */
  preview: (accent: string, signal?: AbortSignal) => Promise<ThemePage>;
}

/** The theme on screen: Emberhut's last answer, or the built-in one in this phone's old appearance. */
export function currentTheme(prefs: Pick<PrefsState, "themeCache" | "theme">): ThemePage {
  return prefs.themeCache ?? { ...builtInTheme, choice: { ...builtInTheme.choice, appearance: prefs.theme } };
}

/** What a change looks like before the server answers; null when it can't be known here (a custom accent, unpreviewed). */
export function applyLocally(page: ThemePage, change: ThemeChange, preview?: ThemePage): ThemePage | null {
  let next = page;
  if (typeof change.accent === "string") {
    if (!preview) return null;
    next = { ...next, tokens: preview.tokens, contrast: preview.contrast, choice: { ...next.choice, accent: preview.choice.accent } };
  } else if (change.preset !== undefined && change.accent === undefined && page.choice.accent !== null) {
    // A preset under a custom accent: the accent stays, so nothing on screen changes.
    next = { ...next, choice: { ...next.choice, preset: change.preset } };
  } else if (change.preset !== undefined || change.accent === null) {
    const id = change.preset ?? page.choice.preset;
    const preset = page.presets.find((p) => p.id === id);
    if (!preset) return null;
    next = { ...next, tokens: { light: preset.light, dark: preset.dark }, choice: { ...next.choice, preset: id, accent: null } };
  }
  if (change.appearance) next = { ...next, choice: { ...next.choice, appearance: change.appearance } };
  return next;
}

/** The page in a POST's answer, without its `ok`. */
function pageOf({ ok: _ok, error: _error, ...page }: ThemeReply): ThemePage {
  return page;
}

const reason = (cause: unknown) => (cause instanceof Error ? cause.message : "no reason given");
const unreachable = (cause: unknown) => cause instanceof ApiError && cause.unreachable;

/**
 * Emberhut's theme, kept in the prefs (so it's there offline and at the next launch) and applied live. The state poll says
 * when it changed elsewhere (`theme.at`); a change made here shows at once and goes back if the server refuses it.
 */
export function createThemeStore(api: EmberhutApi, prefs: StoreApi<PrefsState>) {
  let inflight: Promise<void> | null = null;
  // Bumped by every change made here: a fetch that started before one answers with the old choice, and is dropped.
  let changes = 0;

  return createStore<ThemeState>()((set, get) => {
    async function sync(): Promise<void> {
      const started = changes;
      const p = prefs.getState();
      let held = p.themePending;
      if (!p.themeMigrated) {
        // The first answer decides the old local appearance: kept only if no device ever chose.
        const page = await api.theme.get();
        if (page.choice.at === null && p.theme !== "system") held = { appearance: p.theme, ...held };
        prefs.getState().setThemeMigrated();
        if (!held) {
          if (started === changes) prefs.getState().setThemeCache(page);
          return;
        }
      }
      if (held) {
        try {
          const page = pageOf(await api.theme.set(held));
          if (prefs.getState().themePending === p.themePending) prefs.getState().setThemePending(null);
          if (started === changes) prefs.getState().setThemeCache(page);
          return;
        } catch (cause) {
          if (unreachable(cause)) throw cause;
          // Refused (a preset gone since): drop it and take the server's.
          prefs.getState().setThemePending(null);
        }
      }
      const page = await api.theme.get();
      if (started === changes) prefs.getState().setThemeCache(page);
    }

    return {
      saving: false,
      error: null,
      refresh() {
        inflight ??= sync()
          .catch(() => undefined) // offline or an older server: the cache stays
          .finally(() => (inflight = null));
        return inflight;
      },
      noteState(choice) {
        if (!choice) return;
        const p = prefs.getState();
        if (p.themePending || !p.themeMigrated || !p.themeCache || p.themeCache.choice.at !== choice.at)
          inBackground(get().refresh(), "theme: refresh");
      },
      async choose(change, preview) {
        changes++;
        const before = prefs.getState();
        const local = applyLocally(currentTheme(before), change, preview);
        if (local) prefs.getState().setThemeCache(local);
        set({ saving: true, error: null });
        const held = { ...before.themePending, ...change };
        try {
          const page = pageOf(await api.theme.set(held));
          prefs.getState().setThemePending(null);
          prefs.getState().setThemeCache(page);
          prefs.getState().setThemeMigrated(); // your choice now: the old local appearance has nothing to add
          set({ saving: false });
        } catch (cause) {
          if (unreachable(cause) && local && typeof change.accent !== "string") {
            // Kept here and sent when Emberhut answers again.
            prefs.getState().setThemePending(held);
            set({ saving: false });
            return;
          }
          if (local && prefs.getState().themeCache === local) prefs.getState().setThemeCache(before.themeCache);
          set({
            saving: false,
            error: unreachable(cause) ? "A custom colour needs Emberhut's server: it can't be reached" : reason(cause),
          });
          if (!unreachable(cause)) inBackground(get().refresh(), "theme: refresh");
        }
      },
      async preview(accent, signal) {
        return pageOf(await api.theme.preview(accent, signal));
      },
    };
  });
}
