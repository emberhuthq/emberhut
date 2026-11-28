import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useLayoutEffect, useMemo, useState, type Dispatch, type SetStateAction } from "react";
import { Platform, Pressable, StyleSheet, View } from "react-native";
import { useBottomTabBarHeight } from "react-native-bottom-tabs";
import { KeyboardStickyView } from "react-native-keyboard-controller";
import type { AgentRow, ChatFile, ChatMsg, ChatQuote, EmberhutState } from "@/api";
import { agentsByRoom, agentWaitingOnYou, visible, waitingWords } from "@/features/approvals/model";
import { handoffLine } from "@/features/browser/browser";
import { useBrowserDevice } from "@/features/browser/browserDevice";
import { inBackground } from "@/log";
import { useOpenLink } from "@/navigation/links";
import type { TabScreenProps } from "@/navigation/types";
import { api, chatStore, prefsStore, talkStore } from "@/services";
import { offlineReason } from "@/state/connection";
import { useChat, useConnection, useFallback, useLive, usePrefs, useTalk } from "@/state/hooks";
import { liveLine } from "@/state/live";
import { useEmberhutState, useRoomLive } from "@/state/queries";
import { queryKeys } from "@/state/query";
import { delivered } from "@/state/talk";
import { useTheme } from "@/theme";
import { ErrorState, Icon, Loading, Screen, Text, useBarInsets, useSeeThrough } from "@/ui";
import { FallbackBanner, HandoffBanner, OfflineBanner, PausedBanner } from "./Banners";
import { ChatTitle } from "./ChatTitle";
import { Composer } from "./Composer";
import { setDraft } from "./composerText";
import { MessageList } from "./MessageList";
import { quoteOf } from "./rows";
import { emberhutStatus } from "./status";
import { useSeen } from "./useSeen";

/** Main or a project's room: its conversation, Emberhut's step while it works, and the message box. */
export function ChatScreen({ route, navigation }: TabScreenProps<"Chat">) {
  const held = usePrefs((s) => s.room);
  const asked = route.params?.room;
  const room = asked ?? held;
  useEffect(() => {
    if (asked !== undefined && asked !== prefsStore.getState().room) prefsStore.getState().setRoom(asked);
  }, [asked]);

  const state = useEmberhutState(room);
  useRoomLive(room);
  const offline = useConnection(offlineReason);
  const onFallback = useFallback((s) => s.using === "fallback");
  const data = state.data;
  // Just switched, the state on hand is the last room's until the next answer.
  const here = data !== undefined && (data.room === null || data.room === undefined || data.room === room);
  const chat = useChat((s) => s.chats[room]);
  const complete = useChat((s) => s.complete[room] ?? false);
  const live = useLive((s) => liveLine(s, room, here ? data?.live : null));
  const working = live !== null || (here && data?.busy === true);
  const pending = useTalk((s) => s.pending[room]);
  const sent = pending !== undefined && chat !== undefined && delivered(pending, chat);
  const latest = useMemo(() => chat?.findLast((m) => !m.mine && !m.event)?.id, [chat]);
  useSeen(room, here && data?.unread === true, latest);

  const current = data?.rooms?.find((r) => r.id === room);
  const agent = useMemo(() => agentsByRoom(data?.agents).get(room), [data?.agents, room]);
  const name = agent ? visible(agent.name) : (current?.name ?? "Emberhut");
  const status = useMemo(() => emberhutStatus(data, working, offline !== null), [data, working, offline]);
  useChatHeader(navigation, name, status, data, agent);

  // A draft per room: what's half typed in one room stays there when you switch, and isn't sent to the next.
  const [drafts, setDrafts] = useState<Record<number, string>>({});
  const text = drafts[room] ?? "";
  const setText = useCallback<Dispatch<SetStateAction<string>>>((change) => setDrafts((all) => setDraft(all, room, change)), [room]);
  // A draft handed over (Talk on a task) fills the box once, then leaves the route so the next Talk fills it again.
  const draft = route.params?.draft;
  const [taken, setTaken] = useState<string | undefined>(undefined);
  if (draft !== taken) {
    setTaken(draft);
    if (draft !== undefined) setText(draft);
  }
  useEffect(() => {
    if (draft !== undefined) navigation.setParams({ draft: undefined });
  }, [draft, navigation]);
  // The quote belongs to the room you picked it in: another room starts without one.
  const [quoted, setQuoted] = useState<{ room: number; quote: ChatQuote } | null>(null);
  const replying = quoted?.room === room ? quoted.quote : null;
  const openLink = useOpenLink();
  const device = useBrowserDevice();
  const handoff = handoffLine(data?.browser, device);
  const client = useQueryClient();
  const settings = useMutation({
    mutationFn: (change: Parameters<typeof api.settings.apply>[0]) => api.settings.apply(change),
    onSettled: () => client.invalidateQueries({ queryKey: queryKeys.state(room) }),
  });

  const send = useCallback(
    (words: string, files: ChatFile[]) => {
      inBackground(talkStore.getState().send(room, { text: words, files, reply: replying }), "ChatScreen: send");
      setQuoted(null);
    },
    [room, replying],
  );
  const explain = useCallback(
    (m: ChatMsg) => {
      const words = m.text.replace(/\[([^\]]+)\]\([^)]*\)/g, "$1");
      const quote = words.length > 120 ? `${words.slice(0, 120)}…` : words;
      inBackground(
        talkStore.getState().send(room, { text: `Explain this in simpler words, with a real example: “${quote}”`, files: [] }),
        "ChatScreen: send",
      );
    },
    [room],
  );
  const stop = useCallback(() => {
    talkStore.getState().forget(room);
    settings.mutate({ interrupt: true, room });
  }, [room, settings]);
  const taskStatus = useCallback((id: number) => data?.issues?.find((t) => t.id === id)?.status, [data?.issues]);
  // The keyboard rises over the tab bar: the message box and the newest messages move up by its height less the bar's.
  const tabBar = useBottomTabBarHeight();
  const bars = useBarInsets();
  const seeThrough = useSeeThrough();
  const { colors } = useTheme();
  const [dock, setDock] = useState(0);
  const keyboardOffset = useMemo(() => ({ closed: 0, opened: tabBar }), [tabBar]);

  // Before anything of the room is here, the list shows why (its own empty state), so the list is mounted from the first
  // frame (MessageList). The message box is mounted too, hidden, so its height is known before the first message: the
  // newest one then lands above it instead of moving up once the box is measured.
  const waiting =
    data === undefined && chat === undefined ? (
      state.isError && offline ? (
        <ErrorState message={offline} onRetry={() => inBackground(state.refetch(), "ChatScreen: refetch")} />
      ) : (
        <Loading label="Reaching Emberhut" />
      )
    ) : undefined;

  return (
    <Screen padded={false} underBars>
      <MessageList
        room={room}
        chat={chat}
        pending={pending}
        delivered={sent}
        working={working}
        doing={live?.doing ?? null}
        hasEarlier={here && data?.earlier === true && !complete}
        onLoadEarlier={() => chatStore.getState().loadEarlier(room)}
        onReply={(m) => setQuoted({ room, quote: quoteOf(m) })}
        onRetry={() => inBackground(talkStore.getState().retry(room), "ChatScreen: retry")}
        onLink={openLink}
        onExplain={explain}
        onStarter={setText}
        taskStatus={taskStatus}
        keyboardOffset={tabBar}
        insets={{ top: bars.top, bottom: bars.bottom + dock }}
        waiting={waiting}
      />
      {/* The message box floats over the list's end, in glass, so the conversation scrolls under it. Where nothing is
          see-through (Android, Reduce Transparency) the dock is solid, so no message shows through its gaps. */}
      <KeyboardStickyView
        offset={keyboardOffset}
        style={[
          styles.dock,
          { bottom: bars.bottom },
          !seeThrough && { backgroundColor: colors.background },
          waiting !== undefined && styles.hidden,
        ]}
        pointerEvents={waiting === undefined ? "auto" : "none"}
        accessibilityElementsHidden={waiting !== undefined}
        importantForAccessibility={waiting === undefined ? "auto" : "no-hide-descendants"}
        onLayout={(e) => setDock(e.nativeEvent.layout.height)}
      >
        <View style={styles.accessory}>
          {here && data?.on === false && <PausedBanner onResume={() => settings.mutate({ on: true })} resuming={settings.isPending} />}
          {onFallback && <FallbackBanner />}
          {offline && <OfflineBanner reason={offline} />}
          {handoff && <HandoffBanner line={handoff} onOpen={() => navigation.navigate("Browser")} />}
        </View>
        <Composer
          text={text}
          setText={setText}
          placeholder={
            agent
              ? `Message ${visible(agent.name)}`
              : current && current.project
                ? `Message Emberhut in ${current.name}`
                : "Message Emberhut"
          }
          replying={replying}
          onUnquote={() => setQuoted(null)}
          dictation={data?.voice === true}
          working={working}
          onStop={stop}
          onSend={send}
          disabled={waiting !== undefined}
          hidden={waiting !== undefined}
        />
      </KeyboardStickyView>
    </Screen>
  );
}

/**
 * The room's name and Emberhut's status as the title (a tap lists the rooms); the sessions running now on the right. In an
 * agent's room the title says it's an agent and who it works for, and how many of its requests wait on you sit beside the
 * sessions (as every other count), opening that agent's whole list.
 */
function useChatHeader(
  navigation: TabScreenProps<"Chat">["navigation"],
  name: string,
  status: ReturnType<typeof emberhutStatus>,
  data: EmberhutState | undefined,
  agent: AgentRow | undefined,
) {
  const { colors, spacing } = useTheme();
  const running = runningNow(data);
  const waiting = agent ? agentWaitingOnYou(data, agent.id) : 0;
  useLayoutEffect(() => {
    const sessions = (
      <Pressable
        onPress={() => navigation.navigate("Sessions")}
        accessibilityRole="button"
        accessibilityLabel={`Sessions, ${running} running`}
        hitSlop={ios ? 8 : undefined}
        style={ios ? { marginRight: spacing.lg } : styles.action}
      >
        <Icon name="terminal-outline" size={ios ? undefined : 24} color={colors.text} />
        {running > 0 && (
          <View style={[ios ? styles.badge : styles.actionBadge, { backgroundColor: colors.done, borderColor: colors.surface }]}>
            <Text variant="caption" weight="700" style={[!ios && styles.badgeText, { color: colors.background }]}>
              {running}
            </Text>
          </View>
        )}
      </Pressable>
    );
    const approvals = agent ? (
      <Pressable
        onPress={() => navigation.navigate("Approvals", { agent: agent.id })}
        accessibilityRole="button"
        accessibilityLabel={`${visible(agent.name)}'s approvals, ${waitingWords(waiting)}`}
        hitSlop={ios ? 8 : undefined}
        style={ios ? { marginRight: spacing.lg } : styles.action}
      >
        <Icon name="shield-checkmark-outline" size={ios ? undefined : 24} color={colors.text} />
        {waiting > 0 && (
          <View style={[ios ? styles.badge : styles.actionBadge, { backgroundColor: colors.needsYou, borderColor: colors.surface }]}>
            <Text variant="caption" weight="700" style={[!ios && styles.badgeText, { color: colors.onAccent }]}>
              {waiting}
            </Text>
          </View>
        )}
      </Pressable>
    ) : null;
    navigation.setOptions({
      // The title is a button with its chevron, centred as on iOS. Android's toolbar starts a custom title at its left,
      // where the title's own centred row (ChatTitle) left a short room name floating part way across the bar.
      headerTitleAlign: "center",
      headerTitle: () => <ChatTitle name={name} status={status} agent={agent} onPress={() => navigation.navigate("Rooms")} />,
      headerRight: () => (
        <View style={styles.actions}>
          {approvals}
          {sessions}
        </View>
      ),
    });
  }, [navigation, name, status, running, colors, spacing, agent, waiting]);
}

/** What runs now: each agent, the sessions with no agent row of their own, external threads still running. */
function runningNow(data: EmberhutState | undefined): number {
  const agents = data?.agentsNow ?? [];
  const leads = new Set(agents.flatMap((a) => (a.lead && a.viewer ? [a.viewer] : [])));
  const sessions = (data?.sessions ?? []).filter((s) => (s.status === "running" || s.status === "queued") && !leads.has(s.id)).length;
  const external = (data?.t3Running ?? []).filter((t) => t.status === "running").length;
  return agents.length + sessions + external;
}

const ios = Platform.OS === "ios";

const styles = StyleSheet.create({
  dock: { position: "absolute", left: 0, right: 0 },
  hidden: { opacity: 0 },
  accessory: { gap: 8, paddingHorizontal: 12 },
  actions: { flexDirection: "row", alignItems: "center" },
  badge: {
    position: "absolute",
    top: -6,
    right: -10,
    minWidth: 18,
    height: 18,
    borderRadius: 9,
    alignItems: "center",
    justifyContent: "center",
    paddingHorizontal: 4,
  },
  // Android's toolbar clips an action to its own bounds, so the action is Material's 48dp square with the 24dp icon in
  // its middle, and the badge sits inside it on the icon's top end, ringed in the bar's color so it reads apart from
  // the icon's outline.
  action: { width: 48, height: 48, alignItems: "center", justifyContent: "center" },
  actionBadge: {
    position: "absolute",
    top: 4,
    right: 2,
    minWidth: 20,
    height: 20,
    borderRadius: 10,
    borderWidth: 2,
    alignItems: "center",
    justifyContent: "center",
    paddingHorizontal: 3,
  },
  badgeText: { fontSize: 11, lineHeight: 13, includeFontPadding: false, textAlign: "center" },
});
