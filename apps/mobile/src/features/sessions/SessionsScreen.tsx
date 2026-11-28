import { useNavigation } from "@react-navigation/native";
import type { NativeStackNavigationOptions } from "@react-navigation/native-stack";
import { useLayoutEffect } from "react";
import { Pressable, RefreshControl, SectionList, StyleSheet, View } from "react-native";
import type { AgentNow, RunnerSession, ExternalThread } from "@/api";
import { machineLabel } from "@/features/machines/machineNames";
import { useMachineNames } from "@/features/machines/useMachineNames";
import { usePrefs } from "@/state/hooks";
import { useEmberhutState } from "@/state/queries";
import { useTheme } from "@/theme";
import { Button, EmptyState, Icon, Loading, Screen, Text, usePullRefresh } from "@/ui";
import { SessionFilterBar } from "./SessionFilterBar";
import { isFiltered } from "./sessionFilters";
import { agentsByTask, groupSessions, shortTitle } from "./sessionList";
import { useMoveToTask } from "./SessionMenu";
import { AgentRow, RecentRow, RunningRow, ExternalThreadRow } from "./SessionRows";

type Entry =
  | { kind: "agent"; agent: AgentNow; alone: boolean }
  | { kind: "running"; session: RunnerSession }
  | { kind: "external"; thread: ExternalThread }
  | { kind: "recent"; session: RunnerSession };

/**
 * Every room's sessions: the agents working on your tasks, then every session running (yours and Emberhut's), external threads still
 * running, then the recent ones, waiting on you first. New session at the top. A tab of its own, or a page from More while
 * the server keeps task plans: only its header comes from where it's shown.
 */
export function SessionsScreen({
  navigation: header,
}: {
  navigation: { setOptions: (options: Partial<NativeStackNavigationOptions>) => void };
}) {
  const navigation = useNavigation();
  const { colors, spacing } = useTheme();
  const room = usePrefs((s) => s.room);
  const filter = usePrefs((s) => s.sessionFilter);
  const setFilter = usePrefs((s) => s.setSessionFilter);
  const state = useEmberhutState(room);
  const names = useMachineNames();
  const { refreshing, onRefresh } = usePullRefresh(state.refetch);
  const move = useMoveToTask();

  useLayoutEffect(() => {
    header.setOptions({
      headerRight: () => (
        <Pressable
          onPress={() => navigation.navigate("NewSession")}
          accessibilityRole="button"
          accessibilityLabel="New session"
          hitSlop={8}
          style={{ marginRight: spacing.lg }}
        >
          <Icon name="add" size={26} color={colors.text} />
        </Pressable>
      ),
    });
  }, [header, navigation, colors.text, spacing.lg]);

  if (state.data === undefined) return <Loading label="Reaching Emberhut" />;
  const groups = groupSessions(state.data, filter, names);
  const external = groups.external;
  const filtered = isFiltered(filter);
  const sections = [
    {
      key: "agents",
      title: "Working now",
      count: groups.agents.length,
      data: agentsByTask(groups.agents).flatMap((group) =>
        group.map((agent): Entry => ({ kind: "agent", agent, alone: !group.some((a) => a.lead) })),
      ),
    },
    {
      key: "running",
      title: "Running",
      count: groups.running.length,
      data: groups.running.map((session): Entry => ({ kind: "running", session })),
    },
    { key: "external", title: "External threads", count: 0, data: external.map((thread): Entry => ({ kind: "external", thread })) },
    { key: "recent", title: "Recent", count: 0, data: groups.recent.map((session): Entry => ({ kind: "recent", session })) },
  ].filter((s) => s.data.length > 0);

  const open = (s: RunnerSession) => navigation.navigate("Session", { id: s.id, title: shortTitle(s.title) });

  return (
    <Screen padded={false} underBars>
      <SectionList
        // Under the see-through bars iOS insets the list itself, so the large title shrinks as it scrolls.
        contentInsetAdjustmentBehavior="automatic"
        sections={sections}
        keyExtractor={(e) => (e.kind === "agent" ? e.agent.id : e.kind === "external" ? `external-${e.thread.id}` : e.session.id)}
        renderItem={({ item }) => {
          switch (item.kind) {
            case "agent":
              return (
                <AgentRow
                  agent={item.agent}
                  alone={item.alone}
                  onOpen={
                    item.agent.viewer
                      ? () => navigation.navigate("Session", { id: item.agent.viewer!, title: item.agent.title ?? item.agent.key })
                      : undefined
                  }
                />
              );
            case "running":
              return <RunningRow session={item.session} onOpen={() => open(item.session)} onMove={move} />;
            case "external":
              return <ExternalThreadRow thread={item.thread} place={machineLabel(names, item.thread.machine)} />;
            case "recent":
              return <RecentRow session={item.session} onOpen={() => open(item.session)} onMove={move} />;
          }
        }}
        renderSectionHeader={({ section }) => (
          <View
            style={[
              styles.header,
              {
                gap: spacing.sm,
                paddingHorizontal: spacing.lg,
                paddingTop: spacing.lg,
                paddingBottom: spacing.sm,
                backgroundColor: colors.background,
              },
            ]}
          >
            <Text variant="footnote" tone="secondary" weight="600">
              {section.title}
            </Text>
            {section.count > 0 && (
              <Text variant="caption" weight="700" tone="done">
                {section.count}
              </Text>
            )}
          </View>
        )}
        renderSectionFooter={({ section }) =>
          section.key === "external" ? (
            <Text variant="caption" tone="tertiary" style={{ paddingHorizontal: spacing.lg, paddingTop: spacing.xs }}>
              {"Read only: these finish in the app that started them. New work runs on Emberhut's runner."}
            </Text>
          ) : null
        }
        ItemSeparatorComponent={() => <View style={{ height: StyleSheet.hairlineWidth, backgroundColor: colors.border }} />}
        stickySectionHeadersEnabled={false}
        refreshControl={<RefreshControl refreshing={refreshing} onRefresh={onRefresh} tintColor={colors.accent} />}
        ListHeaderComponent={
          filtered || sections.length > 0 ? <SessionFilterBar filter={filter} picks={groups.picks} onChange={setFilter} /> : undefined
        }
        ListFooterComponent={
          filtered && groups.hidden > 0 && sections.length > 0 ? (
            <Pressable onPress={() => setFilter({})} accessibilityRole="button" style={{ padding: spacing.lg }}>
              <Text variant="footnote" tone="secondary">{`${groups.hidden} more hidden by the filter · Show all`}</Text>
            </Pressable>
          ) : undefined
        }
        ListEmptyComponent={
          filtered ? (
            <EmptyState
              icon="funnel-outline"
              title="Nothing matches the filter"
              message={`${groups.hidden} hidden by it.`}
              action={<Button label="Show all" onPress={() => setFilter({})} />}
            />
          ) : (
            <EmptyState
              icon="terminal-outline"
              title="No sessions yet"
              message="Open Claude Code on one of your machines and talk to it yourself. Emberhut's project work shows here too."
              action={<Button label="New session" onPress={() => navigation.navigate("NewSession")} />}
            />
          )
        }
        contentContainerStyle={{ paddingBottom: spacing.xl }}
      />
    </Screen>
  );
}

const styles = StyleSheet.create({ header: { flexDirection: "row", alignItems: "center" } });
