import * as Clipboard from "expo-clipboard";
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import {
  FlatList,
  Platform,
  Pressable,
  StyleSheet,
  View,
  type NativeScrollEvent,
  type NativeSyntheticEvent,
  type ScrollViewProps,
} from "react-native";
import { KeyboardChatScrollView, KeyboardEvents } from "react-native-keyboard-controller";
import type { ChatMsg, TaskStatus } from "@/api";
import { inBackground } from "@/log";
import type { Pending } from "@/state/talk";
import { useTheme } from "@/theme";
import { ActionSheet, Button, Icon, Loading, Text, type SheetAction } from "@/ui";
import { copyText } from "@/ui/markdownParse";
import { AssistantBubble, Stamp, UserBubble } from "./Bubbles";
import { EventBatch } from "./EventBatch";
import { fromNewest, isAway, isNewSend, newestOffset } from "./pinning";
import { buildRows, type Row } from "./rows";
import { Welcome } from "./Welcome";
import { Working } from "./Working";

type Item = Row | { kind: "pending"; key: "pending" } | { kind: "working"; key: "working" };

export interface MessageListProps {
  /** The room on screen: another one opens at its newest message. */
  room: number;
  /** The room's messages, oldest first; undefined while a room never held is on its way. */
  chat: readonly ChatMsg[] | undefined;
  pending: Pending | undefined;
  /** Your message reached the room's chat: its own copy shows instead. */
  delivered: boolean;
  working: boolean;
  doing: string | null;
  hasEarlier: boolean;
  onLoadEarlier: () => Promise<void>;
  onReply: (m: ChatMsg) => void;
  onRetry: () => void;
  onLink: (url: string) => void;
  /** "Explain simply": the same answer again in simpler words. */
  onExplain: (m: ChatMsg) => void;
  onStarter: (text: string) => void;
  taskStatus: (id: number) => TaskStatus | undefined;
  /** How far the list's bottom sits above the screen's when the keyboard is down (the tab bar): the keyboard lifts the
   * newest messages by its height less this, together with the message box. */
  keyboardOffset: number;
  /** What covers the list's ends: the header above, the message box and the tab bar below; it scrolls under them. */
  insets: { top: number; bottom: number };
  /** What the list shows before the room's chat is here (Emberhut not reached yet, or why not); "Opening the chat" if unset. */
  waiting?: ReactNode;
}

/**
 * The conversation, newest at the bottom like Messages. It opens at the newest message, and a send goes back to it.
 * Pinned to the newest unless you scrolled up to read; then nothing moves the page, and a button (a pill saying so when
 * Emberhut said something since) takes you back down.
 */
export function MessageList(props: MessageListProps) {
  const { room, chat, pending, delivered, working, doing, hasEarlier, onLoadEarlier, onLink, taskStatus, keyboardOffset, insets } = props;
  const { colors, spacing } = useTheme();
  const list = useRef<FlatList<Item>>(null);
  const [away, setAway] = useState(false);
  const [unseen, setUnseen] = useState(false);
  const [loading, setLoading] = useState(false);
  const [menu, setMenu] = useState<ChatMsg | null>(null);
  // The keyboard's share of the list's end (its inset there), and how far up from the newest you last were.
  const keyboardInset = useRef(0);
  const distance = useRef(0);
  const toNewest = useCallback(
    (animated = true) => list.current?.scrollToOffset({ offset: newestOffset(Platform.OS, keyboardInset.current), animated }),
    [],
  );

  // Away or at the newest, from where you scrolled. Not from what the keyboard does: it moves the list by its height and
  // back (on Android by scrolling it, and the last of those scroll events can be dropped by the throttle), but it keeps
  // the newest where it was, so where you were before it moved is still where you are. Not from the way down to the
  // newest either: that ends at the newest.
  const held = useRef(false);
  const release = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const judge = useCallback(() => {
    const now = isAway(distance.current);
    setAway(now);
    if (!now) setUnseen(false);
  }, []);
  const settle = useCallback(() => {
    clearTimeout(release.current);
    held.current = false;
    judge();
  }, [judge]);
  const hold = useCallback(
    (ms: number) => {
      held.current = true;
      clearTimeout(release.current);
      release.current = setTimeout(settle, ms);
    },
    [settle],
  );
  const onScrolled = useCallback(
    (e: NativeSyntheticEvent<NativeScrollEvent>) => {
      if (held.current) return;
      distance.current = fromNewest(Platform.OS, e.nativeEvent.contentOffset.y, keyboardInset.current);
      judge();
    },
    [judge],
  );
  useEffect(() => {
    const moving = () => hold(1000);
    const subs = [
      KeyboardEvents.addListener("keyboardWillShow", moving),
      KeyboardEvents.addListener("keyboardWillHide", moving),
      KeyboardEvents.addListener("keyboardDidShow", settle),
      KeyboardEvents.addListener("keyboardDidHide", settle),
    ];
    return () => {
      clearTimeout(release.current);
      for (const sub of subs) sub.remove();
    };
  }, [hold, settle]);

  // Down to the newest, at once: it counts as there from this render on, so what this render adds (your message on its
  // way) is laid out at the newest instead of held where you were reading.
  const [toGo, setToGo] = useState<{ animated: boolean } | null>(null);
  const goNewest = (animated: boolean) => {
    setAway(false);
    setUnseen(false);
    setToGo({ animated });
  };
  const [sentFor, setSentFor] = useState(pending);
  if (pending !== sentFor) {
    setSentFor(pending);
    if (isNewSend(sentFor, pending)) goNewest(true);
  }
  const [shownRoom, setShownRoom] = useState(room);
  if (room !== shownRoom) {
    setShownRoom(room);
    goNewest(false);
  }
  useEffect(() => {
    if (!toGo) return;
    distance.current = 0;
    hold(toGo.animated ? 600 : 100);
    toNewest(toGo.animated);
  }, [toGo, hold, toNewest]);

  const rows = useMemo(() => buildRows(chat ?? []), [chat]);
  const items = useMemo<Item[]>(() => {
    const top: Item[] = [];
    if (working || (pending && !delivered && pending.note === null && !pending.failed)) top.push({ kind: "working", key: "working" });
    if (pending && !delivered) top.push({ kind: "pending", key: "pending" });
    return [...top, ...[...rows].reverse()];
  }, [rows, pending, delivered, working]);

  const latest = useMemo(() => chat?.findLast((m) => !m.mine && !m.event)?.id, [chat]);
  const seenLatest = useRef(latest);
  useEffect(() => {
    if (latest !== seenLatest.current && away) setUnseen(true);
    seenLatest.current = latest;
  }, [latest, away]);
  // At the newest, what arrives (a reply, Emberhut's step while it works) keeps the newest in view; away, it doesn't move.
  const newest = items[0]?.key;
  const followed = useRef(newest);
  useEffect(() => {
    if (newest === followed.current) return;
    followed.current = newest;
    if (newest !== undefined && !away && !held.current) toNewest();
  }, [newest, away, toNewest]);

  const earlier = async () => {
    setLoading(true);
    await onLoadEarlier().catch(() => undefined);
    setLoading(false);
  };

  const jump = useCallback(
    (id: number) => {
      const index = items.findIndex((it) => (it.kind === "message" && it.msg.id === id) || (it.kind === "events" && it.ids.includes(id)));
      if (index >= 0) list.current?.scrollToIndex({ index, viewPosition: 0.5, animated: true });
    },
    [items],
  );

  const renderItem = ({ item }: { item: Item }) => {
    switch (item.kind) {
      case "working":
        return <Working doing={doing} />;
      case "pending":
        return pending ? <PendingRow pending={pending} onRetry={props.onRetry} onLink={onLink} /> : null;
      case "events":
        return (
          <View>
            {item.stamp && <Stamp date={item.stamp} />}
            <EventBatch text={item.text} />
          </View>
        );
      case "message": {
        const m = item.msg;
        return (
          <View style={{ gap: spacing.xs }}>
            {item.stamp && <Stamp date={item.stamp} />}
            {m.mine ? (
              <UserBubble text={m.text} files={m.files} reply={m.reply} onLink={onLink} onMenu={() => setMenu(m)} onQuote={jump} />
            ) : (
              <AssistantBubble
                text={m.text}
                files={m.files}
                refs={m.refs}
                onLink={onLink}
                onMenu={() => setMenu(m)}
                taskStatus={taskStatus}
              />
            )}
          </View>
        );
      }
    }
  };

  // The keyboard lifts the newest messages with the message box, and a drag down the list takes it away, finger-led.
  const scroller = useCallback(
    (scroll: ScrollViewProps) => (
      <KeyboardChatScrollView
        {...scroll}
        inverted
        offset={keyboardOffset}
        onContentInsetChange={(inset) => (keyboardInset.current = inset.top)}
      />
    ),
    [keyboardOffset],
  );

  // The list is there from the first frame, empty while the chat is on its way: react-native-screens applies the chat
  // screen's scroll-edge settings to the scroll view it finds when the screen attaches, and finds none if a loading
  // view stands in its place (a cold start opens straight into the chat).
  const opening = chat === undefined;

  const actions: SheetAction[] = menu
    ? [
        { label: "Reply", icon: "arrow-undo-outline", onPress: () => props.onReply(menu) },
        {
          label: "Copy",
          icon: "copy-outline",
          onPress: () => inBackground(Clipboard.setStringAsync(copyText(menu.text)), "MessageList: setStringAsync"),
        },
        ...(menu.mine
          ? []
          : [{ label: "Explain simply", icon: "chatbubble-ellipses-outline" as const, onPress: () => props.onExplain(menu) }]),
      ]
    : [];

  return (
    <View style={styles.fill}>
      {/* Upside down, the list would get iOS 26's scroll-edge fade over all of it: the Chat tab's stack screen turns
          the effect off for it (scrollEdgeEffects, Tabs.tsx). */}
      <FlatList
        ref={list}
        inverted
        renderScrollComponent={scroller}
        data={opening ? [] : items}
        keyExtractor={(item) => item.key}
        renderItem={renderItem}
        // Inverted: the content's top is the screen's bottom.
        contentContainerStyle={[
          {
            paddingHorizontal: spacing.lg,
            paddingTop: spacing.md + insets.bottom,
            paddingBottom: spacing.md + insets.top,
            gap: spacing.sm + 2,
          },
          opening && styles.grow,
        ]}
        scrollIndicatorInsets={{ top: insets.bottom, bottom: insets.top }}
        keyboardShouldPersistTaps="handled"
        keyboardDismissMode="interactive"
        onScroll={onScrolled}
        onScrollEndDrag={onScrolled}
        onMomentumScrollEnd={onScrolled}
        scrollEventThrottle={100}
        onScrollToIndexFailed={() => undefined}
        // Held in place only while you read further up; at the newest, what arrives pushes the rest up, above the keyboard.
        maintainVisibleContentPosition={away ? { minIndexForVisible: 1 } : undefined}
        ListEmptyComponent={
          opening ? (
            // One View: the inverted list flips it back upright through its style; a bare element or a Fragment would show upside down.
            <View style={styles.fill}>{props.waiting ?? <Loading label="Opening the chat" />}</View>
          ) : (
            <Welcome onStarter={props.onStarter} />
          )
        }
        ListFooterComponent={
          !opening && hasEarlier && rows.length > 0 ? (
            <View style={styles.earlier}>
              <Button
                label="Show earlier messages"
                variant="plain"
                loading={loading}
                onPress={() => inBackground(earlier(), "MessageList: earlier")}
              />
            </View>
          ) : undefined
        }
      />
      {away && (
        <Pressable
          onPress={() => goNewest(true)}
          accessibilityRole="button"
          accessibilityLabel={unseen ? "New messages: go to the newest" : "Go to the newest message"}
          hitSlop={8}
          style={[
            styles.pill,
            unseen ? styles.centre : [styles.round, { right: spacing.lg }],
            { bottom: insets.bottom + 12, backgroundColor: colors.surface, borderColor: colors.border, gap: spacing.xs },
          ]}
        >
          <Icon name="arrow-down" size={unseen ? 16 : 20} color={colors.accent} />
          {unseen && (
            <Text variant="footnote" weight="600">
              New messages
            </Text>
          )}
        </Pressable>
      )}
      <ActionSheet visible={menu !== null} actions={actions} onClose={() => setMenu(null)} />
    </View>
  );
}

// Your message on its way, with the line under it: why it wasn't sent (a tap sends it again), or what happens next.
function PendingRow({ pending, onRetry, onLink }: { pending: Pending; onRetry: () => void; onLink: (url: string) => void }) {
  const { spacing } = useTheme();
  return (
    <View style={{ gap: spacing.xs }}>
      {(pending.text !== "" || pending.files.length > 0) && (
        <UserBubble text={pending.text} files={pending.files} reply={pending.reply} onLink={onLink} />
      )}
      {pending.failed ? (
        <Pressable
          onPress={onRetry}
          accessibilityRole="button"
          accessibilityLabel={pending.note ?? "Not sent: tap to send again"}
          style={styles.note}
        >
          <Text variant="caption" tone="danger" align="right">
            {pending.note ?? "Not sent · tap to send again"}
          </Text>
        </Pressable>
      ) : pending.note ? (
        <Text variant="caption" tone="secondary" align="right">
          {pending.note}
        </Text>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1 },
  grow: { flexGrow: 1 },
  earlier: { alignItems: "center", paddingBottom: 4 },
  note: { alignSelf: "flex-end" },
  pill: {
    position: "absolute",
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: 14,
    paddingVertical: 8,
    borderRadius: 999,
    borderWidth: StyleSheet.hairlineWidth,
    elevation: 3,
  },
  centre: { alignSelf: "center" },
  round: { width: 40, height: 40, paddingHorizontal: 0, paddingVertical: 0, justifyContent: "center" },
});
