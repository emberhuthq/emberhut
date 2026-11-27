import { useMutation, useQueryClient } from "@tanstack/react-query";
import * as Haptics from "expo-haptics";
import { useMemo, useState } from "react";
import { Pressable, SectionList, StyleSheet, View } from "react-native";
import type { AgentRow, RoomRow } from "@/api";
import { agentLine } from "@/features/approvals/model";
import { inBackground } from "@/log";
import type { RootScreenProps } from "@/navigation/types";
import { api, prefsStore } from "@/services";
import { needsRows } from "@/features/needsYou/model";
import { useActions, usePrefs } from "@/state/hooks";
import { useEmberhutState } from "@/state/queries";
import { queryKeys } from "@/state/query";
import { goalColors, minTap, useTheme } from "@/theme";
import { ActionSheet, Face, Icon, Loading, Screen, Text, type SheetAction } from "@/ui";
import {
  dayFromNow,
  foldedRooms,
  isArchived,
  isMain,
  isPaused,
  openRooms,
  roomColor,
  roomNeeds,
  roomPreview,
  shortAgo,
  splitAgentRooms,
} from "./rooms";

/**
 * Main and a room per project, one line each, then each agent's room under Agents (its name, who it works for, Off while
 * disabled); paused ones folded under their own header. A tap opens that room.
 */
export function RoomsScreen({ navigation }: RootScreenProps<"Rooms">) {
  const { scheme, spacing } = useTheme();
  const room = usePrefs((s) => s.room);
  const state = useEmberhutState(room);
  const [pausedOpen, setPausedOpen] = useState(false);
  const [menu, setMenu] = useState<RoomRow | null>(null);
  const client = useQueryClient();
  const change = useMutation({
    mutationFn: (projectSet: { id: number; status: string; pausedUntil?: string }) => api.settings.apply({ projectSet }),
    onSettled: () => client.invalidateQueries({ queryKey: queryKeys.allStates }),
  });

  const gone = useActions((s) => s.gone);
  const all = useMemo(() => state.data?.rooms ?? [], [state.data?.rooms]);
  const split = useMemo(() => splitAgentRooms(all, state.data?.agents), [all, state.data?.agents]);
  const agentOf = useMemo(() => new Map(split.agents.map((a) => [a.room.id, a.agent])), [split.agents]);
  // From what the Needs you tab counts, so the rooms' counts add up to its badge.
  const needs = useMemo(
    () => roomNeeds(all, needsRows(state.data, gone), undefined, state.data?.approvals ?? [], state.data?.agents ?? []),
    [all, state.data, gone],
  );
  if (state.data === undefined) return <Loading label="Reaching Emberhut" />;
  const rooms = split.rooms;
  const paused = foldedRooms(rooms);
  const sections = [
    { key: "open", title: null, folds: false, data: openRooms(rooms) },
    ...(split.agents.length > 0
      ? [{ key: "agents", title: `Agents · ${split.agents.length}`, folds: false, data: split.agents.map((a) => a.room) }]
      : []),
    ...(paused.length > 0 ? [{ key: "paused", title: `Paused · ${paused.length}`, folds: true, data: pausedOpen ? paused : [] }] : []),
  ];
  const open = (id: number) => {
    inBackground(Haptics.selectionAsync(), "RoomsScreen: selectionAsync");
    prefsStore.getState().setRoom(id);
    navigation.navigate("Tabs", { screen: "Chat", params: { room: id } });
  };

  return (
    <Screen padded={false} edges={["bottom"]} transparent>
      <SectionList
        sections={sections}
        keyExtractor={(r) => String(r.id)}
        renderItem={({ item }) => (
          <RoomLine
            room={item}
            agent={agentOf.get(item.id)}
            color={roomColor(item, state.data?.goals, goalColors(scheme))}
            needs={needs.get(item.id) ?? 0}
            current={item.id === room}
            onPress={() => open(item.id)}
            onMenu={() => setMenu(item)}
          />
        )}
        renderSectionHeader={({ section }) =>
          section.title === null ? null : section.folds ? (
            <FoldHeader title={section.title} open={pausedOpen} onPress={() => setPausedOpen((o) => !o)} />
          ) : (
            <Text
              variant="footnote"
              tone="secondary"
              weight="600"
              accessibilityRole="header"
              style={{ paddingHorizontal: spacing.lg, paddingTop: spacing.lg, paddingBottom: spacing.sm }}
            >
              {section.title}
            </Text>
          )
        }
        ListFooterComponent={
          <Text variant="footnote" tone="tertiary" style={{ padding: spacing.lg }}>
            Long-press a room to pause or archive it.
          </Text>
        }
        ListHeaderComponent={
          <Text variant="headline" style={[styles.sheetTitle, { paddingTop: spacing.xl, paddingBottom: spacing.md }]}>
            Rooms
          </Text>
        }
        contentContainerStyle={{ paddingBottom: spacing.sm }}
      />
      <ActionSheet
        visible={menu !== null}
        title={menu?.name}
        actions={menu ? roomActions(menu, (set) => change.mutate(set)) : []}
        onClose={() => setMenu(null)}
      />
    </Screen>
  );
}

// What you can do with a project's room (its project's status): pause it a while, archive it, and back. Main has none.
function roomActions(room: RoomRow, apply: (set: { id: number; status: string; pausedUntil?: string }) => void): SheetAction[] {
  const id = room.project;
  if (id === null || id === undefined) return [];
  if (isArchived(room)) return [{ label: "Unarchive", icon: "archive-outline", onPress: () => apply({ id, status: "active" }) }];
  return [
    ...(isPaused(room)
      ? [{ label: "Resume now", icon: "play-circle-outline" as const, onPress: () => apply({ id, status: "active" }) }]
      : []),
    { label: "Pause for a week", icon: "pause-circle-outline", onPress: () => apply({ id, status: "paused", pausedUntil: dayFromNow(7) }) },
    {
      label: "Pause for a month",
      icon: "pause-circle-outline",
      onPress: () => apply({ id, status: "paused", pausedUntil: dayFromNow(30) }),
    },
    { label: "Archive", icon: "archive-outline", destructive: true, onPress: () => apply({ id, status: "archived" }) },
  ];
}

function FoldHeader({ title, open, onPress }: { title: string; open: boolean; onPress: () => void }) {
  const { colors, spacing } = useTheme();
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="button"
      accessibilityState={{ expanded: open }}
      accessibilityLabel={title}
      style={[
        styles.row,
        {
          gap: spacing.xs,
          paddingHorizontal: spacing.lg,
          paddingTop: spacing.lg,
          paddingBottom: spacing.sm,
        },
      ]}
    >
      <Text variant="footnote" tone="secondary" weight="600">
        {title}
      </Text>
      <Icon name={open ? "chevron-down" : "chevron-forward"} size={14} color={colors.textSecondary} />
    </Pressable>
  );
}

/**
 * One room: its mark and name (bold with a dot while unread, a pulse while Emberhut works there), when, its last line, what
 * needs you. An agent's room has the agent's mark and, under its name, who it works for (Off while disabled).
 */
function RoomLine({
  room,
  agent,
  needs,
  color,
  current,
  onPress,
  onMenu,
}: {
  room: RoomRow;
  /** The agent whose room it is; none for Main and the projects' rooms. */
  agent?: AgentRow;
  /** What waits on you there (roomNeeds). */
  needs: number;
  color: string;
  current: boolean;
  onPress: () => void;
  onMenu: () => void;
}) {
  const { colors, radius, spacing } = useTheme();
  const off = agent?.enabled === false;
  const quiet = isPaused(room) || isArchived(room);
  const unread = room.unread > 0 && !quiet && !off;
  const line = room.busy ? (room.doing ?? "Working…") : agent ? agentLine(agent) : quiet ? null : roomPreview(room);
  const ago = agent ? null : shortAgo(room.lastAt);
  const plain = agent === undefined && !isMain(room);
  const label = [
    room.name,
    agent ? `agent, ${agentLine(agent).toLowerCase()}` : null,
    room.busy ? "working" : null,
    unread ? `${room.unread} new` : null,
    needs > 0 && !quiet ? `${needs} need you` : null,
  ]
    .filter(Boolean)
    .join(", ");
  return (
    <Pressable
      onPress={onPress}
      onLongPress={plain ? onMenu : undefined}
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ selected: current }}
      accessibilityHint={plain ? "Long press to pause or archive" : undefined}
      style={({ pressed }) => [
        styles.row,
        {
          gap: spacing.md,
          minHeight: minTap + 12,
          marginHorizontal: spacing.sm,
          paddingHorizontal: spacing.md,
          paddingVertical: spacing.sm,
          borderRadius: radius.md,
          backgroundColor: current || pressed ? colors.surfaceRaised : "transparent",
        },
      ]}
    >
      <View style={styles.mark}>
        {agent ? (
          <Icon name="person-circle-outline" size={28} color={off ? colors.textTertiary : colors.accent} />
        ) : isMain(room) ? (
          <Face size={28} />
        ) : (
          <View style={[styles.dot, { backgroundColor: color }]} />
        )}
      </View>
      <View style={styles.fill}>
        <View style={[styles.row, { gap: spacing.xs + 2 }]}>
          <Text weight={unread ? "600" : "400"} tone={off ? "tertiary" : "primary"} numberOfLines={1} style={styles.shrink}>
            {room.name}
          </Text>
          {room.busy && <View style={[styles.small, { backgroundColor: colors.working }]} />}
          {unread && <View style={[styles.small, { backgroundColor: colors.accent }]} />}
          <View style={styles.fill} />
          {isPaused(room) ? (
            <Text variant="footnote" tone="secondary">
              {room.pausedUntil ? `Back ${room.pausedUntil.slice(5).replace("-", "/")}` : "Paused"}
            </Text>
          ) : ago && line ? (
            <Text variant="caption" tone="tertiary">
              {ago}
            </Text>
          ) : null}
        </View>
        {line && (
          <View style={[styles.row, { gap: spacing.sm }]}>
            <Text variant="footnote" numberOfLines={1} style={[styles.fill, { color: room.busy ? colors.working : colors.textSecondary }]}>
              {line}
            </Text>
            {needs > 0 && !quiet && <Needs count={needs} />}
          </View>
        )}
      </View>
      {!line && needs > 0 && !quiet && <Needs count={needs} />}
    </Pressable>
  );
}

function Needs({ count }: { count: number }) {
  const { colors, spacing } = useTheme();
  return (
    <View
      style={[
        styles.row,
        { gap: 3, paddingHorizontal: spacing.sm, paddingVertical: 2, borderRadius: 999, borderWidth: 1, borderColor: colors.needsYou },
      ]}
    >
      <Icon name="hand-left" size={12} color={colors.needsYou} />
      <Text variant="caption" weight="600" tone="needsYou">
        {count}
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  row: { flexDirection: "row", alignItems: "center" },
  fill: { flex: 1 },
  shrink: { flexShrink: 1 },
  sheetTitle: { textAlign: "center" },
  mark: { width: 28, alignItems: "center" },
  dot: { width: 10, height: 10, borderRadius: 5 },
  small: { width: 7, height: 7, borderRadius: 4 },
});
