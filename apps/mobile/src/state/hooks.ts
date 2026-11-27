import { useStore } from "zustand";
import type { PushState } from "@/push/registration";
import {
  actionsStore,
  chatStore,
  connectionStore,
  fallbackStore,
  liveStore,
  prefsStore,
  pushStore,
  sessionStore,
  talkStore,
  themeStore,
} from "@/services";
import type { ActionsState } from "./actions";
import type { ChatState } from "./chat";
import type { ConnectionState } from "./connection";
import type { FallbackState } from "./fallback";
import type { LiveState } from "./live";
import type { PrefsState } from "./prefs";
import type { SessionState } from "./session";
import type { TalkState } from "./talk";
import type { ThemeState } from "./theme";

export const useSession = <T>(select: (s: SessionState) => T): T => useStore(sessionStore, select);
export const usePrefs = <T>(select: (s: PrefsState) => T): T => useStore(prefsStore, select);
export const useChat = <T>(select: (s: ChatState) => T): T => useStore(chatStore, select);
export const useLive = <T>(select: (s: LiveState) => T): T => useStore(liveStore, select);
export const useConnection = <T>(select: (s: ConnectionState) => T): T => useStore(connectionStore, select);
export const useFallback = <T>(select: (s: FallbackState) => T): T => useStore(fallbackStore, select);
/** The server requests go to now: a live stream opened on one is opened again on the other when it changes. */
export const useServerInUse = () => useFallback((s) => s.using);
export const useActions = <T>(select: (s: ActionsState) => T): T => useStore(actionsStore, select);
export const usePush = <T>(select: (s: PushState) => T): T => useStore(pushStore, select);
export const useTalk = <T>(select: (s: TalkState) => T): T => useStore(talkStore, select);
export const useThemeStore = <T>(select: (s: ThemeState) => T): T => useStore(themeStore, select);
