import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import {
  Alert,
  Pressable,
  ScrollView,
  StyleSheet,
  View,
  type NativeScrollEvent,
  type NativeSyntheticEvent,
  type ScrollViewInstance,
} from "react-native";
import { taskKey, type ChatFile, type RunnerSession } from "@/api";
import { Composer } from "@/features/chat/Composer";
import { inBackground } from "@/log";
import { useOpenLink } from "@/navigation/links";
import type { RootScreenProps } from "@/navigation/types";
import { api } from "@/services";
import { usePrefs } from "@/state/hooks";
import { useEmberhutState } from "@/state/queries";
import { useTheme } from "@/theme";
import { ErrorState, Icon, KeyboardAvoider, Loading, Markdown, Screen, Text } from "@/ui";
import { LiveBar, ThinkingLive } from "./LiveParts";
import { SessionHeader } from "./SessionHeader";
import { allSessions, isRunning, shortTitle } from "./sessionList";
import { TodoList } from "./TodoList";
import { isQueued, liveTodos, liveWord, runningTool, turns, type ToolCall } from "./transcript";
import { TurnView } from "./TurnView";
import { useSessionLive } from "./useSessionLive";

/**
 * One session, live: its header, the transcript turn by turn with Claude's words as they're written, the current step
 * pinned at the bottom, and a box to answer it. It follows the newest step until you scroll up ("Live" brings it back).
 */
export function SessionScreen({ route, navigation }: RootScreenProps<"Session">) {
  const { id, agent, title } = route.params;
  const { colors, spacing } = useTheme();
  const room = usePrefs((s) => s.room);
  const state = useEmberhutState(room);
  const info: RunnerSession | undefined = useMemo(() => allSessions(state.data).find((s) => s.id === id), [state.data, id]);
  const helper = agent ? state.data?.agentsNow?.find((a) => a.viewer === id && a.parent === agent) : undefined;
  const { view, ready, connected, failed, retry } = useSessionLive(id, agent);
  const head = view.head;
  const openLink = useOpenLink();

  const status = agent ? (helper ? "running" : "completed") : (head?.status ?? info?.status ?? "");
  const running = agent ? helper !== undefined : (head?.running ?? isRunning(status));
  const direct = !agent && (head?.meta?.direct ?? info?.direct ?? false);
  const since = agent ? (helper?.started ?? null) : (head?.since ?? info?.since ?? null);
  const all = useMemo(() => turns(view.steps), [view.steps]);
  const liveTurn = head?.turn ?? all.at(-1)?.id ?? null;
  const asking = status === "waiting" ? (view.steps.findLast((s) => s.kind === "question")?.seq ?? null) : null;
  const todos = running && !agent ? liveTodos(view.steps, head?.turn) : null;
  const tool = runningTool(view.steps);

  const [problem, setProblem] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const [draft, setDraft] = useState("");
  const scroll = useRef<ScrollViewInstance>(null);
  const [following, setFollowing] = useState(true);

  const name = title ?? (info ? shortTitle(info.title) : head ? shortTitle(head.title) : "Session");
  const tasks = (info?.tasks ?? head?.tasks ?? []).map((t) => {
    const task = state.data?.issues?.find((i) => i.id === t);
    return { id: t, title: task ? `${task.key} ${task.title}` : taskKey(t) };
  });

  const send = useCallback(
    async (text: string, files: ChatFile[] = []): Promise<boolean> => {
      if (sending) return false;
      setSending(true);
      try {
        await api.sessions.send(id, text, files);
        setProblem(null);
        setFollowing(true);
        return true;
      } catch (cause) {
        setProblem(`Not sent: ${cause instanceof Error ? cause.message : "unknown"}`);
        return false;
      } finally {
        setSending(false);
      }
    },
    [id, sending],
  );
  const interrupt = useCallback(() => {
    api.sessions.interrupt(id).catch((cause: unknown) => setProblem(`Not stopped: ${cause instanceof Error ? cause.message : "unknown"}`));
  }, [id]);
  const stop = useCallback(() => {
    Alert.alert("Stop this session?", "The step it's on ends now and nothing queued after it runs. A reply later carries it on.", [
      { text: "Cancel", style: "cancel" },
      {
        text: "Stop it",
        style: "destructive",
        onPress: () =>
          void api.sessions
            .stop(id)
            .catch((cause: unknown) => setProblem(`Not stopped: ${cause instanceof Error ? cause.message : "unknown"}`)),
      },
    ]);
  }, [id]);
  const openAgent = useCallback(
    (call: ToolCall, agentName: string) => call.call.id && navigation.push("Session", { id, agent: call.call.id, title: agentName }),
    [navigation, id],
  );

  useLayoutEffect(() => {
    navigation.setOptions({
      title: name,
      headerRight:
        running && !agent && !direct
          ? () => (
              <Pressable onPress={stop} accessibilityRole="button" accessibilityLabel="Stop this session" hitSlop={8}>
                <Icon name="stop-circle-outline" color={colors.danger} />
              </Pressable>
            )
          : undefined,
    });
  }, [navigation, name, running, agent, direct, stop, colors.danger]);

  useEffect(() => {
    if (following) scroll.current?.scrollToEnd({ animated: true });
  }, [following, view.steps.length, view.typing.length, view.thinking]);

  // Only your own scrolling stops the following; back at the bottom it follows again.
  const onScroll = (e: NativeSyntheticEvent<NativeScrollEvent>) => {
    const { contentOffset, contentSize, layoutMeasurement } = e.nativeEvent;
    setFollowing(contentOffset.y + layoutMeasurement.height >= contentSize.height - 80);
  };

  if (failed && !ready) return <ErrorState message="The session didn't load." onRetry={retry} />;
  if (!ready) return <Loading label="Opening the session" />;

  const word = (tick: number) =>
    liveWord(
      {
        thinking: view.thinking !== null,
        typing: view.typing !== "",
        tool: tool?.name ?? null,
        todo: todos?.find((t) => t.status === "in_progress")?.activeForm ?? null,
      },
      tick,
    );
  const detail = agent
    ? (helper?.line ?? null)
    : view.thinking === null && view.typing === "" && tool
      ? ((head?.doing ?? info?.doing ?? null)?.replace("Running: ", "$ ") ?? null)
      : null;

  return (
    <Screen padded={false} edges={["bottom"]}>
      <KeyboardAvoider>
        <View style={styles.fill}>
          <ScrollView
            ref={scroll}
            onScrollBeginDrag={() => setFollowing(false)}
            onMomentumScrollEnd={onScroll}
            onScrollEndDrag={onScroll}
            contentContainerStyle={{ padding: spacing.lg, gap: spacing.md }}
            keyboardShouldPersistTaps="handled"
          >
            <SessionHeader
              title={name}
              lead={agent ? (head?.title ?? info?.title ?? "its lead") : null}
              status={status}
              running={running}
              since={since}
              connected={connected}
              head={head}
              direct={direct}
              tasks={tasks}
              problem={problem}
            />
            {view.earlier && (
              <Text variant="footnote" tone="secondary">
                {"Older steps aren't shown here."}
              </Text>
            )}
            {all.map((turn) => (
              <TurnView
                key={turn.id}
                turn={turn}
                live={running && turn.id === liveTurn}
                queued={running && isQueued(turn.id, head?.turn)}
                asking={asking}
                onAnswer={(text) => inBackground(send(text), "SessionScreen: send")}
                agents={head?.agents ?? []}
                onAgent={openAgent}
                onLink={openLink}
              />
            ))}
            {running && !agent && view.thinking !== null && <ThinkingLive text={view.thinking} since={view.thinkingSince} />}
            {view.typing !== "" && <Markdown text={view.typing} taskRefs onLink={openLink} selectable />}
          </ScrollView>
          {running && !following && (
            <Pressable
              onPress={() => setFollowing(true)}
              accessibilityRole="button"
              accessibilityLabel="Back to the live step"
              style={[styles.live, { bottom: spacing.md, backgroundColor: colors.done, gap: spacing.xs }]}
            >
              <Icon name="arrow-down" size={14} color={colors.background} />
              <Text variant="footnote" weight="600" style={{ color: colors.background }}>
                Live
              </Text>
            </Pressable>
          )}
        </View>
        {todos && (
          <View style={{ paddingHorizontal: spacing.lg, paddingVertical: spacing.sm, backgroundColor: colors.surface }}>
            <TodoList todos={todos} live />
          </View>
        )}
        {running && (
          <LiveBar
            word={word}
            detail={detail}
            since={since}
            tokens={agent ? (helper?.tokens ?? null) : (view.tokens ?? head?.tokens ?? null)}
          />
        )}
        {!agent && (
          <Composer
            text={draft}
            setText={setDraft}
            placeholder={
              running ? "Add a message (it runs after this turn)" : direct ? "Message Claude" : "Answer it, or tell it to carry on"
            }
            dictation={state.data?.voice === true}
            working={running && direct}
            onStop={interrupt}
            stopLabel="Stop this turn"
            onSend={send}
            disabled={sending}
          />
        )}
      </KeyboardAvoider>
    </Screen>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1 },
  live: {
    position: "absolute",
    alignSelf: "center",
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: 12,
    paddingVertical: 6,
    borderRadius: 999,
  },
});
